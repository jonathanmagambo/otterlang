use std::fmt::Write as _;

use super::types::{
    CallTemplate, CrateSpec, DependencyConfig, FunctionSpec, PublicItem, RustTypeRef, StubSource,
    TypeSpec,
};

enum ArgContext<'a> {
    C {
        indent: &'a str,
        default_return: &'a str,
    },
    Json {
        indent: &'a str,
        array_name: &'a str,
        func_name: &'a str,
    },
}

/// Emits the `Cargo.toml` and `lib.rs` contents for a bridge crate.
#[derive(Clone, Debug)]
pub struct RustStubGenerator {
    crate_name: String,
    dependency: DependencyConfig,
}

impl RustStubGenerator {
    pub fn new(crate_name: String, dependency: DependencyConfig) -> Self {
        Self {
            crate_name,
            dependency,
        }
    }

    pub fn generate(&self, functions: &[FunctionSpec]) -> StubSource {
        let manifest = self.render_manifest();
        let source = self.render_source(functions);

        StubSource { manifest, source }
    }

    /// Convert a CrateSpec's public synchronous functions into bridge FunctionSpec entries.
    pub fn functions_from_crate_spec(&self, spec: &CrateSpec) -> Vec<FunctionSpec> {
        let mut out = Vec::new();
        for item in &spec.items {
            if let PublicItem::Function { sig, path, .. } = item {
                // Map parameter and return types; skip if any are unsupported
                let mut params = Vec::new();
                let mut skip = false;
                for p in &sig.params {
                    match map_rust_type_to_spec(p) {
                        Some(ts) => params.push(ts),
                        None => {
                            skip = true;
                            break;
                        }
                    }
                }
                if skip {
                    continue;
                }
                let result = match &sig.return_type {
                    Some(rt) => match map_rust_type_to_spec(rt) {
                        Some(ts) => ts,
                        None => continue,
                    },
                    None => TypeSpec::Unit,
                };

                let (export_name, rust_path) = if path.segments.is_empty() {
                    (
                        sig.name.clone(),
                        Some(format!("{}::{}", self.dependency.name, sig.name)),
                    )
                } else {
                    let export = path.segments.join(".");
                    let rust = path.segments.join("::");
                    (export, Some(rust))
                };

                if sig.is_async || matches!(sig.return_type, Some(RustTypeRef::Future { .. })) {
                    let spawn_name = format!("{}.{}_spawn", export_name, sig.name);
                    let spawn_expr = {
                        let rp = rust_path.clone().unwrap();
                        format!(
                            "ffi_store::insert(rt().spawn(async move {{ {rp}({args}) }}))",
                            rp = rp,
                            args = (0..params.len())
                                .map(|i| format!("{{{}}}", i))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    };
                    out.push(FunctionSpec {
                        name: spawn_name,
                        symbol: format!(
                            "otter_{}_{}_spawn",
                            self.dependency.name,
                            sig.name.to_lowercase()
                        ),
                        params: params.clone(),
                        result: TypeSpec::Opaque,
                        doc: None,
                        rust_path: None,
                        call: CallTemplate::Expr(spawn_expr),
                    });
                    let await_name = format!("{}.{}_await", export_name, sig.name);
                    let out_ty = match &sig.return_type {
                        Some(RustTypeRef::Future { output }) => {
                            map_rust_type_to_spec(output).unwrap_or(TypeSpec::Opaque)
                        }
                        Some(other) => map_rust_type_to_spec(other).unwrap_or(TypeSpec::Opaque),
                        None => TypeSpec::Unit,
                    };
                    let join_ty = rust_value_ty(&out_ty);
                    let await_expr = format!(
                        "match rt().block_on(ffi_store::take::<tokio::task::JoinHandle<{jt}>>({{0}})) {{ Ok(v) => v, Err(_) => {fn} }}",
                        jt = join_ty,
                        fn = out_ty.default_return()
                    );
                    out.push(FunctionSpec {
                        name: await_name,
                        symbol: format!(
                            "otter_{}_{}_await",
                            self.dependency.name,
                            sig.name.to_lowercase()
                        ),
                        params: vec![TypeSpec::Opaque],
                        result: out_ty,
                        doc: None,
                        rust_path: None,
                        call: CallTemplate::Expr(await_expr),
                    });
                } else {
                    let export_name_clone = export_name.clone();
                    let params_clone = params.clone();
                    out.push(FunctionSpec {
                        name: export_name,
                        symbol: format!(
                            "otter_{}_{}",
                            self.dependency.name,
                            sig.name.to_lowercase()
                        ),
                        params,
                        result,
                        doc: None,
                        rust_path,
                        call: CallTemplate::Direct,
                    });

                    if let Some(ret_ty) = &sig.return_type {
                        let args_ph = (0..sig.params.len())
                            .map(|i| format!("{{{}}}", i))
                            .collect::<Vec<_>>()
                            .join(", ");
                        let rust_call_path = if path.segments.is_empty() {
                            format!("{}::{}", self.dependency.name, sig.name)
                        } else {
                            path.segments.join("::")
                        };
                        let rust_call = format!("{}({})", rust_call_path, args_ph);

                        match ret_ty {
                            RustTypeRef::Option { .. } => {
                                let helper_name =
                                    format!("{}.{}_optjson", export_name_clone, sig.name);
                                let expr = format!(
                                    "match {} {{ Some(v) => serde_json::to_string(&json!({{{{\"some\": true, \"value\": v}}}})).unwrap_or_default(), None => \"{{{{\"some\":false}}}}\".to_string() }}",
                                    rust_call
                                );
                                out.push(FunctionSpec {
                                    name: helper_name,
                                    symbol: format!(
                                        "otter_{}_{}_optjson",
                                        self.dependency.name,
                                        sig.name.to_lowercase()
                                    ),
                                    params: params_clone.clone(),
                                    result: TypeSpec::Str,
                                    doc: None,
                                    rust_path: None,
                                    call: CallTemplate::Expr(expr),
                                });
                            }
                            RustTypeRef::Result { .. } => {
                                let helper_name = format!("{}.{}_try", export_name_clone, sig.name);
                                let expr = format!(
                                    "match {} {{ Ok(v) => serde_json::to_string(&json!({{{{\"ok\": true, \"value\": v}}}})).unwrap_or_default(), Err(e) => serde_json::to_string(&json!({{{{\"ok\": false, \"error\": format!(\"{{:?}}\", e)}}}})).unwrap_or_default() }}",
                                    rust_call
                                );
                                out.push(FunctionSpec {
                                    name: helper_name,
                                    symbol: format!(
                                        "otter_{}_{}_try",
                                        self.dependency.name,
                                        sig.name.to_lowercase()
                                    ),
                                    params: params_clone.clone(),
                                    result: TypeSpec::Str,
                                    doc: None,
                                    rust_path: None,
                                    call: CallTemplate::Expr(expr),
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        out
    }

    fn render_manifest(&self) -> String {
        let mut manifest = format!(
            "[package]\nname = \"otterffi_{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\ncrate-type = [\"cdylib\"]\n\n[dependencies]\n",
            name = self.crate_name
        );
        manifest.push_str(&format!(
            "{dep} = {spec}\n",
            dep = self.dependency.name,
            spec = self.dependency.manifest_entry()
        ));
        manifest.push_str("abi_stable = \"0.11\"\n");
        manifest.push_str("once_cell = \"1.19\"\n");
        manifest.push_str("parking_lot = \"0.12\"\n");
        manifest.push_str("serde = { version = \"1.0\", features = [\"derive\"] }\n");
        manifest.push_str("serde_json = \"1.0\"\n");
        manifest.push_str("tokio = { version = \"1\", features = [\"rt-multi-thread\"] }\n");
        manifest
    }

    fn render_source(&self, functions: &[FunctionSpec]) -> String {
        let mut source = String::new();
        source.push_str("use std::ffi::{CStr, CString};\n");
        source.push_str("use std::os::raw::c_char;\n");
        source.push_str("use std::panic::{self, AssertUnwindSafe};\n");
        source.push_str("use abi_stable::std_types::{RString, RVec};\n");
        source.push_str("use abi_stable::StableAbi;\n");
        source.push_str("use once_cell::sync::Lazy;\n");
        source.push_str("use parking_lot::Mutex;\n");
        source.push_str("use serde_json::{json, Value};\n");
        source.push_str("use std::any::Any;\n");
        source.push_str("use std::sync::atomic::{AtomicU64, Ordering};\n\n");
        source.push_str("use tokio::runtime::Runtime;\n");

        // Convert crate name (hyphens to underscores for import)
        let dep_import_name = self.dependency.name.replace('-', "_");
        source.push_str(&format!(
            "use {dep_import_name} as ffi_dep;\n",
            dep_import_name = dep_import_name
        ));

        // Add crate-specific imports
        source.push('\n');

        source.push_str(
            "#[expect(dead_code)]\nmod ffi_store {\n    use super::*;\n    use std::collections::HashMap;\n\n    struct Entry {\n        value: Box<dyn Any + Send + Sync>,\n        refs: u64,\n    }\n\n    static NEXT_ID: AtomicU64 = AtomicU64::new(1);\n    static STORE: Lazy<Mutex<HashMap<u64, Entry>>> = Lazy::new(|| Mutex::new(HashMap::new()));\n\n    pub fn insert<T: Any + Send + Sync + 'static>(value: T) -> i64 {\n        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);\n        STORE.lock().insert(id, Entry { value: Box::new(value), refs: 1 });\n        id as i64\n    }\n\n    pub fn clone_handle(id: i64) -> i64 {\n        let mut store = STORE.lock();\n        if let Some(entry) = store.get_mut(&(id as u64)) {\n            entry.refs += 1;\n            id\n        } else {\n            panic!(\"invalid opaque handle\");\n        }\n    }\n\n    pub fn release_handle(id: i64) {\n        let mut store = STORE.lock();\n        if let Some(mut entry) = store.remove(&(id as u64)) {\n            if entry.refs > 1 {\n                entry.refs -= 1;\n                store.insert(id as u64, entry);\n            }\n        }\n    }\n\n    pub fn take<T: Any + Send + Sync + 'static>(id: i64) -> T {\n        let mut store = STORE.lock();\n        let key = id as u64;\n        if let Some(mut entry) = store.remove(&key) {\n            if entry.refs > 1 {\n                // put back with decreased ref and fail fast to catch misuse\n                entry.refs -= 1;\n                store.insert(key, entry);\n                panic!(\"opaque handle still referenced\");\n            }\n            entry.value.downcast::<T>().map(|boxed| *boxed).expect(\"opaque handle type mismatch\")\n        } else {\n            panic!(\"invalid opaque handle\");\n        }\n    }\n\n    pub fn get<T: Any + Send + Sync + Clone + 'static>(id: i64) -> T {\n        let store = STORE.lock();\n        store\n            .get(&(id as u64))\n            .and_then(|e| e.value.downcast_ref::<T>())\n            .cloned()\n            .expect(\"invalid opaque handle\")\n    }\n}\n\n",
        );

        source.push_str(
            "#[repr(u8)]\n#[derive(Clone, Copy, Debug, StableAbi)]\npub enum FfiType {\n    Unit,\n    Bool,\n    I32,\n    I64,\n    F64,\n    Str,\n    Opaque,\n}\n\n",
        );
        source.push_str(
            "#[repr(C)]\n#[derive(Clone, StableAbi)]\npub struct StableFunction {\n    pub name: RString,\n    pub symbol: RString,\n    pub params: RVec<FfiType>,\n    pub result: FfiType,\n}\n\n",
        );
        source.push_str(
            "#[repr(C)]\n#[derive(Clone, StableAbi)]\npub struct StableExportSet {\n    pub functions: RVec<StableFunction>,\n}\n\n",
        );
        source.push_str(
            "static RUNTIME: once_cell::sync::Lazy<Runtime> = once_cell::sync::Lazy::new(|| {\n    tokio::runtime::Builder::new_multi_thread().enable_all().build().expect(\"failed to build tokio runtime\")\n});\n\nfn rt() -> &'static Runtime { &*RUNTIME }\n\n",
        );
        source.push_str(
            "#[no_mangle]\npub extern \"C\" fn otter_free(ptr: *mut c_char) {\n    if ptr.is_null() {\n        return;\n    }\n    unsafe {\n        let _ = CString::from_raw(ptr);\n    }\n}\n\n",
        );
        source.push_str(
            "#[no_mangle]\npub extern \"C\" fn otter_handle_clone(handle: i64) -> i64 {\n    ffi_store::clone_handle(handle)\n}\n\n#[no_mangle]\npub extern \"C\" fn otter_handle_release(handle: i64) {\n    ffi_store::release_handle(handle)\n}\n\n",
        );

        for function in functions {
            self.render_function(function, &mut source);
        }

        self.render_json_dispatch(functions, &mut source);
        self.render_exports(functions, &mut source);

        if functions.is_empty() {
            source.push_str(
                "// No functions requested yet; add entries to bridge.yaml to expose APIs.\n",
            );
        }

        source
    }

    fn render_function(&self, function: &FunctionSpec, out: &mut String) {
        if let Some(doc) = &function.doc {
            let _ = writeln!(out, "/// {}", doc);
        }
        let _ = writeln!(out, "#[no_mangle]");
        let _ = writeln!(out, "pub extern \"C\" fn {}(", function.symbol);

        for (idx, param) in function.params.iter().enumerate() {
            let is_last = idx + 1 == function.params.len();
            if is_last {
                let _ = writeln!(out, "    arg{}: {}", idx, param.to_rust());
            } else {
                let _ = writeln!(out, "    arg{}: {},", idx, param.to_rust());
            }
        }

        out.push_str(") -> ");
        out.push_str(function.result.to_rust());
        out.push_str(" {\n");
        self.render_function_body(function, out);
        out.push_str("}\n\n");
    }

    fn render_function_body(&self, function: &FunctionSpec, out: &mut String) {
        let default_return = function.result.default_return();
        let (setup, call_args) = self.build_call_arguments(
            function,
            ArgContext::C {
                indent: "    ",
                default_return,
            },
        );
        for line in setup {
            out.push_str(&line);
        }

        let invocation = self.resolve_invocation(function, &call_args);

        match &function.call {
            CallTemplate::Direct | CallTemplate::Expr(_) => {
                self.render_direct_call(function, &invocation, default_return, out);
            }
            CallTemplate::Result => {
                self.render_result_call(function, &invocation, default_return, out);
            }
        }
    }

    fn render_direct_call(
        &self,
        function: &FunctionSpec,
        invocation: &str,
        default_return: &str,
        out: &mut String,
    ) {
        match function.result {
            TypeSpec::Unit => {
                out.push_str(&format!(
                    "    let _ = ::std::panic::catch_unwind(|| {{ {invocation}; }});\n",
                    invocation = invocation
                ));
                out.push_str("    ()\n");
            }
            TypeSpec::Str => {
                out.push_str(&format!(
                    "    let result = match ::std::panic::catch_unwind(|| {{ {invocation} }}) {{\n        Ok(value) => value,\n        Err(_) => return {default_return},\n    }};\n    match CString::new(result) {{\n        Ok(cstr) => cstr.into_raw(),\n        Err(_) => {default_return},\n    }}\n",
                    invocation = invocation,
                    default_return = default_return
                ));
            }
            TypeSpec::Opaque => {
                out.push_str(&format!(
                    "    let result = match ::std::panic::catch_unwind(|| {{ {invocation} }}) {{\n        Ok(value) => value,\n        Err(_) => return {default_return},\n    }};\n    ffi_store::insert(result)\n",
                    invocation = invocation,
                    default_return = default_return
                ));
            }
            _ => {
                out.push_str(&format!(
                    "    match ::std::panic::catch_unwind(|| {{ {invocation} }}) {{\n        Ok(value) => value,\n        Err(_) => {default_return},\n    }}\n",
                    invocation = invocation,
                    default_return = default_return
                ));
            }
        }
    }

    fn render_result_call(
        &self,
        function: &FunctionSpec,
        invocation: &str,
        default_return: &str,
        out: &mut String,
    ) {
        out.push_str(&format!(
            "    let result = match ::std::panic::catch_unwind(|| {{ {invocation} }}) {{\n        Ok(value) => value,\n        Err(_) => return {default_return},\n    }};\n",
            invocation = invocation,
            default_return = default_return
        ));

        match function.result {
            TypeSpec::Unit => {
                out.push_str(
                    "    match result {\n        Ok(_) => (),\n        Err(_) => (),\n    }\n",
                );
            }
            TypeSpec::Str => {
                out.push_str(&format!(
                    "    match result {{\n        Ok(value) => match CString::new(value) {{\n            Ok(cstr) => cstr.into_raw(),\n            Err(_) => {default_return},\n        }},\n        Err(_) => {default_return},\n    }}\n",
                    default_return = default_return
                ));
            }
            _ => {
                out.push_str(&format!(
                    "    match result {{\n        Ok(value) => value,\n        Err(_) => {default_return},\n    }}\n",
                    default_return = default_return
                ));
            }
        }
    }

    fn render_json_dispatch(&self, functions: &[FunctionSpec], out: &mut String) {
        out.push_str(
            "#[no_mangle]\npub extern \"C\" fn otter_call_json(func: *const c_char, args_json: *const c_char) -> *mut c_char {\n",
        );
        out.push_str(
            "    let func_name = unsafe {\n        if func.is_null() {\n            return std::ptr::null_mut();\n        }\n        match CStr::from_ptr(func).to_str() {\n            Ok(value) => value.to_owned(),\n            Err(_) => return std::ptr::null_mut(),\n        }\n    };\n",
        );
        out.push_str(
            "    let args_string = unsafe {\n        if args_json.is_null() {\n            String::from(\"[]\")\n        } else {\n            match CStr::from_ptr(args_json).to_str() {\n                Ok(value) => value.to_owned(),\n                Err(_) => return std::ptr::null_mut(),\n            }\n        }\n    };\n",
        );
        out.push_str(
            "    let invocation = || -> Result<String, String> {\n        let args_value: Value = serde_json::from_str(&args_string).map_err(|e| e.to_string())?;\n        let args_array = args_value.as_array().ok_or_else(|| \"expected array\".to_string())?;\n        match func_name.as_str() {\n",
        );
        for function in functions {
            let key = &function.name;
            out.push_str(&format!("            \"{}\" => {{\n", key));
            let (setup, call_args) = self.build_call_arguments(
                function,
                ArgContext::Json {
                    indent: "                ",
                    array_name: "args_array",
                    func_name: key,
                },
            );
            for line in setup {
                out.push_str(&line);
            }
            let invocation = self.resolve_invocation(function, &call_args);
            match &function.call {
                CallTemplate::Direct | CallTemplate::Expr(_) => {
                    out.push_str(&format!(
                        "                let result = {invocation};\n",
                        invocation = invocation
                    ));
                    let json_expr = self.render_json_result_expr(function, "result");
                    out.push_str(&format!(
                        "                serde_json::to_string(&{json_expr}).map_err(|e| e.to_string())\n",
                        json_expr = json_expr
                    ));
                }
                CallTemplate::Result => {
                    out.push_str(&format!(
                        "                let result = {invocation};\n",
                        invocation = invocation
                    ));
                    let json_expr = self.render_json_result_expr(function, "value");
                    out.push_str(&format!(
                        "                match result {{\n                    Ok(value) => serde_json::to_string(&{json_expr}).map_err(|e| e.to_string()),\n                    Err(err) => Err(format!(\"{{:?}}\", err)),\n                }}\n",
                        json_expr = json_expr
                    ));
                }
            }
            out.push_str("            }\n");
        }
        out.push_str(
            "            _ => Err(format!(\"unknown function {}\", func_name)),\n        }\n    };\n",
        );
        out.push_str(
            "    match panic::catch_unwind(AssertUnwindSafe(invocation)) {\n        Ok(Ok(json)) => match CString::new(json) {\n            Ok(cstr) => cstr.into_raw(),\n            Err(_) => std::ptr::null_mut(),\n        },\n        _ => std::ptr::null_mut(),\n    }\n}\n\n",
        );
    }

    fn render_exports(&self, functions: &[FunctionSpec], out: &mut String) {
        out.push_str(
            "#[no_mangle]\npub extern \"C\" fn otterlang_exports() -> StableExportSet {\n",
        );
        if functions.is_empty() {
            out.push_str("    StableExportSet { functions: RVec::from(vec![]) }\n}\n\n");
            return;
        }

        out.push_str("    let functions = RVec::from(vec![\n");
        for function in functions {
            let params = function
                .params
                .iter()
                .map(TypeSpec::ffi_variant)
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!(
                "        StableFunction {{\n            name: RString::from(\"{name}\"),\n            symbol: RString::from(\"{symbol}\"),\n            params: RVec::from(vec![{params}]),\n            result: {result},\n        }},\n",
                name = function.name,
                symbol = function.symbol,
                params = params,
                result = function.result.ffi_variant(),
            ));
        }
        out.push_str("    ]);\n    StableExportSet { functions }\n}\n\n");
    }

    fn build_call_arguments(
        &self,
        function: &FunctionSpec,
        context: ArgContext<'_>,
    ) -> (Vec<String>, Vec<String>) {
        let mut setup = Vec::new();
        let mut call_args = Vec::new();

        for (idx, param) in function.params.iter().enumerate() {
            let arg_name = format!("arg{idx}");
            match (param, &context) {
                (
                    TypeSpec::Str,
                    ArgContext::C {
                        indent,
                        default_return,
                    },
                ) => {
                    let owned = format!("arg{idx}_string");
                    setup.push(format!(
                        "{indent}let {owned} = unsafe {{\n{indent}    if {arg_name}.is_null() {{\n{indent}        String::new()\n{indent}    }} else {{\n{indent}        match CStr::from_ptr({arg_name}).to_str() {{\n{indent}            Ok(value) => value.to_owned(),\n{indent}            Err(_) => return {default_return},\n{indent}        }}\n{indent}    }}\n{indent}}};\n",
                        indent = indent,
                        owned = owned,
                        arg_name = arg_name,
                        default_return = default_return
                    ));
                    call_args.push(format!("&{owned}"));
                }
                (
                    TypeSpec::Str,
                    ArgContext::Json {
                        indent,
                        array_name,
                        func_name,
                    },
                ) => {
                    let owned = format!("arg{idx}_string");
                    setup.push(format!(
                        "{indent}let {arg_name}_value = {array}.get({idx}).ok_or_else(|| format!(\"missing argument {idx} for {func}\"))?;\n",
                        indent = indent,
                        arg_name = arg_name,
                        array = array_name,
                        idx = idx,
                        func = func_name
                    ));
                    setup.push(format!(
                        "{indent}let {owned} = {arg_name}_value.as_str().ok_or_else(|| format!(\"argument {idx} for {func} must be a string\"))?.to_owned();\n",
                        indent = indent,
                        owned = owned,
                        arg_name = arg_name,
                        idx = idx,
                        func = func_name
                    ));
                    call_args.push(format!("&{owned}"));
                }
                (
                    TypeSpec::F64,
                    ArgContext::Json {
                        indent,
                        array_name,
                        func_name,
                    },
                ) => {
                    setup.push(format!(
                        "{indent}let {arg_name}_value = {array}.get({idx}).ok_or_else(|| format!(\"missing argument {idx} for {func}\"))?;\n{indent}let {arg_name} = {arg_name}_value.as_f64().ok_or_else(|| format!(\"argument {idx} for {func} must be a number\"))?;\n",
                        indent = indent,
                        arg_name = arg_name,
                        array = array_name,
                        idx = idx,
                        func = func_name
                    ));
                    call_args.push(arg_name);
                }
                (
                    TypeSpec::I32,
                    ArgContext::Json {
                        indent,
                        array_name,
                        func_name,
                    },
                ) => {
                    setup.push(format!(
                        "{indent}let {arg_name}_value = {array}.get({idx}).ok_or_else(|| format!(\"missing argument {idx} for {func}\"))?;\n{indent}let {arg_name}_raw = {arg_name}_value.as_i64().ok_or_else(|| format!(\"argument {idx} for {func} must be an integer\"))?;\n{indent}let {arg_name} = {arg_name}_raw as i32;\n",
                        indent = indent,
                        arg_name = arg_name,
                        array = array_name,
                        idx = idx,
                        func = func_name
                    ));
                    call_args.push(arg_name);
                }
                (
                    TypeSpec::I64 | TypeSpec::Opaque,
                    ArgContext::Json {
                        indent,
                        array_name,
                        func_name,
                    },
                ) => {
                    setup.push(format!(
                        "{indent}let {arg_name}_value = {array}.get({idx}).ok_or_else(|| format!(\"missing argument {idx} for {func}\"))?;\n{indent}let {arg_name} = {arg_name}_value.as_i64().ok_or_else(|| format!(\"argument {idx} for {func} must be an integer\"))?;\n",
                        indent = indent,
                        arg_name = arg_name,
                        array = array_name,
                        idx = idx,
                        func = func_name
                    ));
                    call_args.push(arg_name);
                }
                (
                    TypeSpec::Bool,
                    ArgContext::Json {
                        indent,
                        array_name,
                        func_name,
                    },
                ) => {
                    setup.push(format!(
                        "{indent}let {arg_name}_value = {array}.get({idx}).ok_or_else(|| format!(\"missing argument {idx} for {func}\"))?;\n{indent}let {arg_name} = {arg_name}_value.as_bool().ok_or_else(|| format!(\"argument {idx} for {func} must be a boolean\"))?;\n",
                        indent = indent,
                        arg_name = arg_name,
                        array = array_name,
                        idx = idx,
                        func = func_name
                    ));
                    call_args.push(arg_name);
                }
                (TypeSpec::Unit, _) => {}
                (_, ArgContext::C { .. }) => {
                    call_args.push(arg_name);
                }
            }
        }

        (setup, call_args)
    }

    fn resolve_invocation(&self, function: &FunctionSpec, call_args: &[String]) -> String {
        match &function.call {
            CallTemplate::Expr(expr) => self.render_expr_invocation(expr, call_args),
            _ => {
                let rust_path = if let Some(path) = &function.rust_path {
                    path.as_str()
                } else {
                    let fallback = format!(
                        "{}::{}",
                        self.dependency.name,
                        function.name.replace(':', "::")
                    );
                    return if call_args.is_empty() {
                        format!("{}()", fallback)
                    } else {
                        format!("{}({})", fallback, call_args.join(", "))
                    };
                };

                if call_args.is_empty() {
                    format!("{}()", rust_path)
                } else {
                    format!("{}({})", rust_path, call_args.join(", "))
                }
            }
        }
    }

    fn render_expr_invocation(&self, expr: &str, call_args: &[String]) -> String {
        let mut out = expr.to_string();
        for (i, arg) in call_args.iter().enumerate() {
            let ph = format!("{{{}}}", i);
            out = out.replace(&ph, arg);
        }
        out
    }

    fn render_json_result_expr(&self, function: &FunctionSpec, ident: &str) -> String {
        match function.result {
            TypeSpec::Unit => "json!(null)".to_string(),
            TypeSpec::Str
            | TypeSpec::Bool
            | TypeSpec::I32
            | TypeSpec::I64
            | TypeSpec::F64
            | TypeSpec::Opaque => {
                format!("json!({})", ident)
            }
        }
    }
}

fn map_rust_type_to_spec(ty: &RustTypeRef) -> Option<TypeSpec> {
    match ty {
        RustTypeRef::Unit => Some(TypeSpec::Unit),
        RustTypeRef::Bool => Some(TypeSpec::Bool),
        RustTypeRef::I8
        | RustTypeRef::I16
        | RustTypeRef::I32
        | RustTypeRef::U8
        | RustTypeRef::U16
        | RustTypeRef::U32
        | RustTypeRef::Char => Some(TypeSpec::I32),
        RustTypeRef::I64
        | RustTypeRef::U64
        | RustTypeRef::I128
        | RustTypeRef::U128
        | RustTypeRef::Usize
        | RustTypeRef::Isize => Some(TypeSpec::I64),
        RustTypeRef::F32 | RustTypeRef::F64 => Some(TypeSpec::F64),
        RustTypeRef::Str | RustTypeRef::String => Some(TypeSpec::Str),
        RustTypeRef::Option { inner } => map_rust_type_to_spec(inner).or(Some(TypeSpec::Opaque)),
        RustTypeRef::Result { ok, .. } => map_rust_type_to_spec(ok).or(Some(TypeSpec::Opaque)),
        RustTypeRef::Ref { inner, .. } => map_rust_type_to_spec(inner),
        RustTypeRef::Box { inner } | RustTypeRef::Rc { inner } | RustTypeRef::Arc { inner } => {
            map_rust_type_to_spec(inner).or(Some(TypeSpec::Opaque))
        }
        RustTypeRef::Vec { .. }
        | RustTypeRef::Slice { .. }
        | RustTypeRef::Array { .. }
        | RustTypeRef::Tuple { .. }
        | RustTypeRef::Future { .. }
        | RustTypeRef::HashMap { .. }
        | RustTypeRef::HashSet { .. }
        | RustTypeRef::Cow { .. }
        | RustTypeRef::Fn { .. }
        | RustTypeRef::Generic { .. }
        | RustTypeRef::Path { .. }
        | RustTypeRef::Opaque => Some(TypeSpec::Opaque),
    }
}

fn rust_value_ty(spec: &TypeSpec) -> &'static str {
    match spec {
        TypeSpec::Unit => "()",
        TypeSpec::Bool => "bool",
        TypeSpec::I32 => "i32",
        TypeSpec::I64 | TypeSpec::Opaque => "i64",
        TypeSpec::F64 => "f64",
        TypeSpec::Str => "String",
    }
}
