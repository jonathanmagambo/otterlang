use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use ast::nodes::{Program, Statement};
use ffi::{BridgeSymbolRegistry, CargoBridge, DynamicLibraryLoader, FunctionSpec, TypeSpec};
use libloading::Library;

use crate::runtime::ffi::register_dynamic_exports;
use crate::runtime::symbol_registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

pub(crate) fn prepare_rust_bridges(
    program: &Program,
    registry: &SymbolRegistry,
) -> Result<Vec<PathBuf>> {
    let imports = collect_rust_imports(program);
    if imports.is_empty() {
        return Ok(Vec::new());
    }

    let bridge_registry = BridgeSymbolRegistry::global().clone();
    let cargo_bridge = CargoBridge::new(bridge_registry.clone())?;
    let loader = DynamicLibraryLoader::global();
    let mut libraries = Vec::new();

    for (crate_name, aliases) in imports {
        let metadata = bridge_registry.ensure_metadata(&crate_name)?;
        let artifacts = cargo_bridge.ensure_bridge(&crate_name)?;
        loader.load(&artifacts.library_path).with_context(|| {
            format!("failed to load Rust bridge library for crate `{crate_name}`")
        })?;
        // Register all exports directly from the library (transparent + manual entries)
        unsafe {
            let lib = Library::new(&artifacts.library_path).with_context(|| {
                format!(
                    "failed to open library {}",
                    artifacts.library_path.display()
                )
            })?;
            register_dynamic_exports(&lib, registry)?;
        }
        // Also register planned functions to ensure symbol aliases are available immediately
        register_bridge_functions(&crate_name, &aliases, &metadata.functions, registry)?;
        libraries.push(artifacts.library_path.clone());
    }

    Ok(libraries)
}

fn collect_rust_imports(program: &Program) -> HashMap<String, HashSet<String>> {
    let mut imports: HashMap<String, HashSet<String>> = HashMap::new();

    for statement in &program.statements {
        if let Statement::Use {
            imports: use_imports,
        } = statement.as_ref()
        {
            for import in use_imports {
                if let Some((namespace, crate_name)) = import.as_ref().module.split_once(':')
                    && namespace == "rust"
                {
                    let aliases = imports.entry(crate_name.to_string()).or_default();
                    aliases.insert(crate_name.to_string());
                    if let Some(alias_name) = &import.as_ref().alias {
                        aliases.insert(alias_name.clone());
                    }
                }
            }
        }
    }

    imports
}

fn register_bridge_functions(
    crate_name: &str,
    aliases: &HashSet<String>,
    functions: &[FunctionSpec],
    registry: &SymbolRegistry,
) -> Result<()> {
    if functions.is_empty() {
        return Ok(());
    }

    for function in functions {
        let canonical_name = if function.name.contains(':') || function.name.contains('.') {
            function.name.clone()
        } else {
            format!("{crate_name}:{}", function.name)
        };

        let params = function
            .params
            .iter()
            .enumerate()
            .map(|(idx, param)| {
                type_spec_to_ffi(param, "parameter", &canonical_name).with_context(|| {
                    format!("parameter {idx} in `{canonical_name}` is not FFI compatible")
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let result = type_spec_to_ffi(&function.result, "return", &canonical_name)?;
        let signature = FfiSignature::new(params.clone(), result.clone());

        registry.register(FfiFunction {
            name: canonical_name.clone(),
            symbol: function.symbol.clone(),
            signature: signature.clone(),
        });

        for alias in aliases {
            let alias_name = alias_name(alias, crate_name, &canonical_name);
            registry.register(FfiFunction {
                name: alias_name,
                symbol: function.symbol.clone(),
                signature: FfiSignature::new(params.clone(), result.clone()),
            });
        }
    }

    registry.register(FfiFunction {
        name: format!("{crate_name}.__call_json"),
        symbol: "otter_call_json".into(),
        signature: FfiSignature::new(vec![FfiType::Str, FfiType::Str], FfiType::Str),
    });

    for alias in aliases {
        registry.register(FfiFunction {
            name: format!("{alias}.__call_json"),
            symbol: "otter_call_json".into(),
            signature: FfiSignature::new(vec![FfiType::Str, FfiType::Str], FfiType::Str),
        });
    }

    Ok(())
}

fn type_spec_to_ffi(spec: &TypeSpec, position: &str, function_name: &str) -> Result<FfiType> {
    match spec {
        TypeSpec::Unit => {
            if position == "return" {
                Ok(FfiType::Unit)
            } else {
                bail!("`{function_name}` cannot accept a unit value in parameter position")
            }
        }
        TypeSpec::Bool => Ok(FfiType::Bool),
        TypeSpec::I32 => Ok(FfiType::I32),
        TypeSpec::I64 => Ok(FfiType::I64),
        TypeSpec::F64 => Ok(FfiType::F64),
        TypeSpec::Str => Ok(FfiType::Str),
        TypeSpec::Opaque => Ok(FfiType::Opaque),
    }
}

fn alias_name(alias: &str, crate_name: &str, canonical: &str) -> String {
    if let Some(rest) = canonical.strip_prefix(&format!("{}:", crate_name)) {
        format!("{alias}.{rest}")
    } else {
        format!("{alias}.{canonical}")
    }
}
