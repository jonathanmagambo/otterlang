use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context as LlvmContext;
use inkwell::module::Module;
use inkwell::passes::PassManager;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum, FunctionType};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue, PointerValue};
use inkwell::AddressSpace;
use inkwell::OptimizationLevel;

use crate::ast::nodes::{BinaryOp, Block, Expr, Function, Literal, Program, Statement, Type};
use crate::ffi::{BridgeSymbolRegistry, CargoBridge, DynamicLibraryLoader, FunctionSpec, TypeSpec};
use crate::runtime::ffi;
use crate::runtime::symbol_registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};
use crate::codegen::target::TargetTriple;

pub struct CodegenOptions {
    pub emit_ir: bool,
    pub opt_level: CodegenOptLevel,
    pub enable_lto: bool,
    pub enable_pgo: bool,
    pub pgo_profile_file: Option<PathBuf>,
    pub inline_threshold: Option<u32>,
    /// Target triple for cross-compilation (defaults to native)
    pub target: Option<TargetTriple>,
}

impl Default for CodegenOptions {
    fn default() -> Self {
        Self {
            emit_ir: false,
            opt_level: CodegenOptLevel::Default,
            enable_lto: false,
            enable_pgo: false,
            pgo_profile_file: None,
            inline_threshold: None, // Use LLVM default
            target: None, // Use native target
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CodegenOptLevel {
    None,
    Default,
    Aggressive,
}

impl From<CodegenOptLevel> for OptimizationLevel {
    fn from(value: CodegenOptLevel) -> Self {
        match value {
            CodegenOptLevel::None => OptimizationLevel::None,
            CodegenOptLevel::Default => OptimizationLevel::Default,
            CodegenOptLevel::Aggressive => OptimizationLevel::Aggressive,
        }
    }
}

pub struct BuildArtifact {
    pub binary: PathBuf,
    pub ir: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OtterType {
    Unit,
    Bool,
    I32,
    I64,
    F64,
    Str,
    Opaque,
}

impl From<FfiType> for OtterType {
    fn from(value: FfiType) -> Self {
        match value {
            FfiType::Unit => OtterType::Unit,
            FfiType::Bool => OtterType::Bool,
            FfiType::I32 => OtterType::I32,
            FfiType::I64 => OtterType::I64,
            FfiType::F64 => OtterType::F64,
            FfiType::Str => OtterType::Str,
            FfiType::Opaque => OtterType::Opaque,
        }
    }
}

struct EvaluatedValue<'ctx> {
    ty: OtterType,
    value: Option<BasicValueEnum<'ctx>>,
}

impl<'ctx> EvaluatedValue<'ctx> {
    fn with_value(value: BasicValueEnum<'ctx>, ty: OtterType) -> Self {
        Self {
            ty,
            value: Some(value),
        }
    }
}

#[derive(Clone, Copy)]
struct Variable<'ctx> {
    ptr: PointerValue<'ctx>,
    ty: OtterType,
}

#[derive(Clone, Copy)]
struct LoopContext<'ctx> {
    continue_bb: BasicBlock<'ctx>,
    break_bb: BasicBlock<'ctx>,
}

#[derive(Clone)]
struct FunctionContext<'ctx> {
    variables: HashMap<String, Variable<'ctx>>,
    loop_stack: Vec<LoopContext<'ctx>>,
}

impl<'ctx> FunctionContext<'ctx> {
    fn new() -> Self {
        Self {
            variables: HashMap::new(),
            loop_stack: Vec::new(),
        }
    }

    fn push_loop(&mut self, continue_bb: BasicBlock<'ctx>, break_bb: BasicBlock<'ctx>) {
        self.loop_stack.push(LoopContext {
            continue_bb,
            break_bb,
        });
    }

    fn pop_loop(&mut self) -> Option<LoopContext<'ctx>> {
        self.loop_stack.pop()
    }

    fn current_loop(&self) -> Option<&LoopContext<'ctx>> {
        self.loop_stack.last()
    }

    fn get(&self, name: &str) -> Option<&Variable<'ctx>> {
        self.variables.get(name)
    }

    fn insert(&mut self, name: String, variable: Variable<'ctx>) {
        self.variables.insert(name, variable);
    }
}

pub fn current_llvm_version() -> Option<String> {
    Some("15.0".to_string())
}

pub fn build_executable(
    program: &Program,
    output: &Path,
    options: &CodegenOptions,
) -> Result<BuildArtifact> {
    let context = LlvmContext::create();
    let module = context.create_module("otter");
    let builder = context.create_builder();
    let registry = ffi::bootstrap_stdlib();
    let bridge_libraries = prepare_rust_bridges(program, registry)?;
    let mut compiler = Compiler::new(&context, module, builder, registry);

    compiler.lower_program(program, true)?; // Require main for executables
    compiler
        .module
        .verify()
        .map_err(|e| anyhow!("LLVM module verification failed: {e}"))?;

    if options.emit_ir {
        // Ensure IR snapshot happens before LLVM potentially mutates the module during codegen.
        compiler.cached_ir = Some(compiler.module.print_to_string().to_string());
    }

    // Initialize all LLVM targets before creating any target triples
    Target::initialize_all(&InitializationConfig::default());

    // Get native target triple directly from LLVM
    let native_triple = inkwell::targets::TargetMachine::get_default_triple();
    let native_str = native_triple.to_string();
    
    // Strip version suffix from triple (e.g., "arm64-apple-darwin25.0.0" -> "arm64-apple-darwin")
    // LLVM recognizes the triple without the version suffix
    let triple_str = if let Some(darwin_idx) = native_str.find("darwin") {
        let after_darwin = &native_str[darwin_idx + 6..];
        if after_darwin.chars().next().map_or(false, |c| c.is_ascii_digit() || c == '.') {
            // Version suffix found after "darwin", strip it
            format!("{}darwin", &native_str[..darwin_idx + 6])
        } else {
            native_str.clone()
        }
    } else {
        native_str.clone()
    };
    
    // Create inkwell TargetTriple using triple without version suffix
    let llvm_triple = inkwell::targets::TargetTriple::create(&triple_str);
    compiler.module.set_triple(&llvm_triple);

    let target = Target::from_triple(&llvm_triple)
        .map_err(|e| anyhow!("failed to create target from triple {} (original: {}): {e}", triple_str, native_str))?;

    let optimization: OptimizationLevel = options.opt_level.into();
    let target_machine = target
        .create_target_machine(
            &llvm_triple,
            "generic",
            "",
            optimization,
            RelocMode::Default,
            CodeModel::Default,
        )
        .ok_or_else(|| anyhow!("failed to create target machine"))?;

    compiler
        .module
        .set_data_layout(&target_machine.get_target_data().get_data_layout());

    compiler.run_default_passes(options.opt_level, options.enable_pgo, options.pgo_profile_file.as_deref(), options.inline_threshold);

    let object_path = output.with_extension("o");
    target_machine
        .write_to_file(&compiler.module, FileType::Object, &object_path)
        .map_err(|e| {
            anyhow!(
                "failed to emit object file at {}: {e}",
                object_path.display()
            )
        })?;

    // Create a C runtime shim for the FFI functions (target-specific)
    let runtime_c = output.with_extension("runtime.c");
    // Parse native triple for runtime code generation
    let runtime_triple = TargetTriple::parse(&native_str)
        .unwrap_or_else(|_| TargetTriple::new("x86_64", "unknown", "linux", Some("gnu")));
    let runtime_c_content = runtime_triple.runtime_c_code();
    fs::write(&runtime_c, runtime_c_content).context("failed to write runtime C file")?;

    // Compile the runtime C file (target-specific)
    let runtime_o = output.with_extension("runtime.o");
    let linker = runtime_triple.linker();
    let mut cc = Command::new(&linker);
    
    // Add target-specific compiler flags
    if runtime_triple.is_wasm() {
        // For WebAssembly, use clang with target flag
        cc.arg("--target").arg(&native_str).arg("-c");
    } else {
        cc.arg("-c");
        if runtime_triple.needs_pic() && !runtime_triple.is_windows() {
            cc.arg("-fPIC");
        }
        // Add target triple for cross-compilation
        if options.target.is_some() {
            cc.arg("--target").arg(&native_str);
        }
    }
    
    cc.arg(&runtime_c)
        .arg("-o")
        .arg(&runtime_o);
    
    let cc_status = cc.status().context("failed to compile runtime C file")?;

    if !cc_status.success() {
        bail!("failed to compile runtime C file");
    }

    // Link the object files together (target-specific)
    let linker = runtime_triple.linker();
    let mut cc = Command::new(&linker);
    
    // Add target-specific linker flags
    if runtime_triple.is_wasm() {
        // WebAssembly linking
        cc.arg("--target").arg(&native_str)
            .arg("--no-entry")
            .arg("--export-dynamic")
            .arg(&object_path)
            .arg("-o")
            .arg(output);
    } else {
        // Standard linking
        if options.target.is_some() {
            cc.arg("--target").arg(&native_str);
        }
        cc.arg(&object_path).arg(&runtime_o).arg("-o").arg(output);
    }
    
    // Apply target-specific linker flags
    for flag in runtime_triple.linker_flags() {
        cc.arg(&flag);
    }

    if options.enable_lto {
        cc.arg("-flto");
        // Enable LTO optimization level matching the codegen level
        match options.opt_level {
            CodegenOptLevel::None => {}
            CodegenOptLevel::Default => {
                cc.arg("-flto=O2");
            }
            CodegenOptLevel::Aggressive => {
                cc.arg("-flto=O3");
            }
        }
    }

    // PGO support: if profile file is provided, use it for optimization
    if options.enable_pgo {
        if let Some(ref profile_file) = options.pgo_profile_file {
            cc.arg("-fprofile-use");
            cc.arg(profile_file);
        } else {
            // Generate profile instrumentation
            cc.arg("-fprofile-instr-generate");
        }
    }

    for lib in &bridge_libraries {
        cc.arg(lib);
    }

    let status = cc.status().context("failed to invoke system linker (cc)")?;

    if !status.success() {
        bail!("linker invocation failed with status {status}");
    }

    // Clean up temporary files
    fs::remove_file(&runtime_c).ok();
    fs::remove_file(&runtime_o).ok();

    fs::remove_file(&object_path).ok();

    Ok(BuildArtifact {
        binary: output.to_path_buf(),
        ir: compiler.cached_ir.take(),
    })
}

/// Build a shared library (.so/.dylib) for JIT execution
pub fn build_shared_library(
    program: &Program,
    output: &Path,
    options: &CodegenOptions,
) -> Result<BuildArtifact> {
    let context = LlvmContext::create();
    let module = context.create_module("otter_jit");
    let builder = context.create_builder();
    let registry = ffi::bootstrap_stdlib();
    let bridge_libraries = prepare_rust_bridges(program, registry)?;
    let mut compiler = Compiler::new(&context, module, builder, registry);

    compiler.lower_program(program, false)?; // Don't require main for shared libraries
    compiler
        .module
        .verify()
        .map_err(|e| anyhow!("LLVM module verification failed: {e}"))?;

    if options.emit_ir {
        compiler.cached_ir = Some(compiler.module.print_to_string().to_string());
    }

    // Initialize all LLVM targets before creating any target triples
    Target::initialize_all(&InitializationConfig::default());

    // Get native target triple directly from LLVM
    let native_triple = inkwell::targets::TargetMachine::get_default_triple();
    let native_str = native_triple.to_string();
    
    // Strip version suffix from triple (e.g., "arm64-apple-darwin25.0.0" -> "arm64-apple-darwin")
    // LLVM recognizes the triple without the version suffix
    let triple_str = if let Some(darwin_idx) = native_str.find("darwin") {
        let after_darwin = &native_str[darwin_idx + 6..];
        if after_darwin.chars().next().map_or(false, |c| c.is_ascii_digit() || c == '.') {
            // Version suffix found after "darwin", strip it
            format!("{}darwin", &native_str[..darwin_idx + 6])
        } else {
            native_str.clone()
        }
    } else {
        native_str.clone()
    };
    
    // Create inkwell TargetTriple using triple without version suffix
    let llvm_triple = inkwell::targets::TargetTriple::create(&triple_str);
    compiler.module.set_triple(&llvm_triple);

    let target = Target::from_triple(&llvm_triple)
        .map_err(|e| anyhow!("failed to create target from triple {} (original: {}): {e}", triple_str, native_str))?;

    let optimization: OptimizationLevel = options.opt_level.into();
    let target_machine = target
        .create_target_machine(
            &llvm_triple,
            "generic",
            "",
            optimization,
            RelocMode::PIC, // Use PIC for shared libraries
            CodeModel::Default,
        )
        .ok_or_else(|| anyhow!("failed to create target machine"))?;

    compiler
        .module
        .set_data_layout(&target_machine.get_target_data().get_data_layout());

    compiler.run_default_passes(options.opt_level, options.enable_pgo, options.pgo_profile_file.as_deref(), options.inline_threshold);

    // Compile to object file with position-independent code
    let object_path = output.with_extension("o");
    target_machine
        .write_to_file(&compiler.module, FileType::Object, &object_path)
        .map_err(|e| {
            anyhow!(
                "failed to emit object file at {}: {e}",
                object_path.display()
            )
        })?;

    // Create runtime C file (target-specific)
    let runtime_c = output.with_extension("runtime.c");
    // Parse native triple for runtime code generation
    let runtime_triple = TargetTriple::parse(&native_str)
        .unwrap_or_else(|_| TargetTriple::new("x86_64", "unknown", "linux", Some("gnu")));
    let runtime_c_content = runtime_triple.runtime_c_code();
    fs::write(&runtime_c, runtime_c_content)
        .context("failed to write runtime C file")?;

    // Compile runtime C file (target-specific)
    let runtime_o = output.with_extension("runtime.o");
    let linker = runtime_triple.linker();
    let mut cc = Command::new(&linker);
    
    // Add target-specific compiler flags
    if runtime_triple.is_wasm() {
        cc.arg("--target").arg(&native_str).arg("-c");
    } else {
        cc.arg("-c");
        cc.arg("-fPIC"); // Position-independent code for shared library
        if options.target.is_some() {
            cc.arg("--target").arg(&native_str);
        }
    }
    
    cc.arg(&runtime_c)
        .arg("-o")
        .arg(&runtime_o);
    
    let cc_status = cc.status().context("failed to compile runtime C file")?;

    if !cc_status.success() {
        bail!("failed to compile runtime C file");
    }

    // Determine shared library extension (target-specific)
    let lib_ext = if runtime_triple.is_wasm() {
        "wasm"
    } else if runtime_triple.is_windows() {
        "dll"
    } else if runtime_triple.os == "darwin" {
        "dylib"
    } else {
        "so"
    };

    let lib_path = if output.extension().is_some() && output.extension().unwrap() == lib_ext {
        output.to_path_buf()
    } else {
        output.with_extension(lib_ext)
    };

    // Link as shared library (target-specific)
    let linker = runtime_triple.linker();
    let mut cc = Command::new(&linker);
    
    if runtime_triple.is_wasm() {
        cc.arg("--target").arg(&native_str)
            .arg("--no-entry")
            .arg("--export-dynamic")
            .arg("-o")
            .arg(&lib_path)
            .arg(&object_path);
    } else {
        cc.arg("-shared")
            .arg("-fPIC")
            .arg("-o")
            .arg(&lib_path)
            .arg(&object_path)
            .arg(&runtime_o);
        
        if options.target.is_some() {
            cc.arg("--target").arg(&native_str);
        }
    }
    
    // Apply target-specific linker flags
    for flag in runtime_triple.linker_flags() {
        cc.arg(&flag);
    }

    if options.enable_lto {
        cc.arg("-flto");
        // Enable LTO optimization level matching the codegen level
        match options.opt_level {
            CodegenOptLevel::None => {}
            CodegenOptLevel::Default => {
                cc.arg("-flto=O2");
            }
            CodegenOptLevel::Aggressive => {
                cc.arg("-flto=O3");
            }
        }
    }

    // PGO support: if profile file is provided, use it for optimization
    if options.enable_pgo {
        if let Some(ref profile_file) = options.pgo_profile_file {
            cc.arg("-fprofile-use");
            cc.arg(profile_file);
        } else {
            // Generate profile instrumentation
            cc.arg("-fprofile-instr-generate");
        }
    }

    for lib in &bridge_libraries {
        cc.arg(lib);
    }

    let status = cc.status().context("failed to invoke system linker (cc)")?;

    if !status.success() {
        bail!("linker invocation failed with status {status}");
    }

    // Clean up temporary files
    fs::remove_file(&runtime_c).ok();
    fs::remove_file(&runtime_o).ok();
    fs::remove_file(&object_path).ok();

    Ok(BuildArtifact {
        binary: lib_path,
        ir: compiler.cached_ir.take(),
    })
}

fn prepare_rust_bridges(program: &Program, registry: &SymbolRegistry) -> Result<Vec<PathBuf>> {
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
        if metadata.functions.is_empty() {
            continue;
        }

        let artifacts = cargo_bridge.ensure_bridge(&crate_name)?;
        loader.load(&artifacts.library_path).with_context(|| {
            format!("failed to load Rust bridge library for crate `{crate_name}`")
        })?;

        register_bridge_functions(&crate_name, &aliases, &metadata.functions, registry)?;
        libraries.push(artifacts.library_path.clone());
    }

    Ok(libraries)
}

fn collect_rust_imports(program: &Program) -> HashMap<String, HashSet<String>> {
    let mut imports: HashMap<String, HashSet<String>> = HashMap::new();

    for statement in &program.statements {
        if let Statement::Use { module, alias } = statement {
            if let Some((namespace, crate_name)) = module.split_once(':') {
                if namespace == "rust" {
                    let aliases = imports.entry(crate_name.to_string()).or_default();
                    if let Some(alias_name) = alias {
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
    if let Some(rest) = canonical.strip_prefix(crate_name) {
        format!("{alias}{rest}")
    } else {
        format!("{alias}:{canonical}")
    }
}

struct Compiler<'ctx> {
    context: &'ctx LlvmContext,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    cached_ir: Option<String>,
    symbol_registry: &'static SymbolRegistry,
    // Cache for frequently used types and functions
    string_ptr_type: inkwell::types::PointerType<'ctx>,
    declared_functions: std::collections::HashMap<String, FunctionValue<'ctx>>,
    lambda_counter: std::sync::atomic::AtomicUsize,
}

impl<'ctx> Compiler<'ctx> {
    fn new(
        context: &'ctx LlvmContext,
        module: Module<'ctx>,
        builder: Builder<'ctx>,
        symbol_registry: &'static SymbolRegistry,
    ) -> Self {
        let string_ptr_type = context.i8_type().ptr_type(AddressSpace::default());

        Self {
            context,
            module,
            builder,
            cached_ir: None,
            symbol_registry,
            string_ptr_type,
            declared_functions: std::collections::HashMap::new(),
            lambda_counter: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn lower_program(&mut self, program: &Program, require_main: bool) -> Result<()> {
        // Extract functions from statements
        let functions: Vec<&Function> = program
            .statements
            .iter()
            .filter_map(|stmt| match stmt {
                Statement::Function(func) => Some(func),
                _ => None,
            })
            .collect();

        if functions.is_empty() {
            bail!("program contains no functions");
        }

        // First, declare all functions (without bodies)
        for function in &functions {
            self.declare_function(function)?;
        }

        // Then, lower the bodies of all functions
        for function in &functions {
            self.lower_function_body(function)?;
        }

        if require_main && !functions.iter().any(|f| f.name == "main") {
            bail!("entry function `main` not found");
        }

        Ok(())
    }

    fn declare_function(&mut self, function: &Function) -> Result<FunctionValue<'ctx>> {
        // Determine parameter types
        let mut param_types = vec![];
        for param in &function.params {
            let ty = if let Some(ty) = &param.ty {
                self.type_from_ast(ty)?
            } else {
                OtterType::F64 // Default to f64 if no type specified
            };
            param_types.push(self.basic_type(ty)?);
        }

        // Determine return type
        let ret_type = if let Some(ret_ty) = &function.ret_ty {
            self.type_from_ast(ret_ty)?
        } else {
            OtterType::I32 // Default to i32 for compatibility
        };

        // Build function type
        let param_metadata: Vec<BasicMetadataTypeEnum> =
            param_types.iter().map(|&t| t.into()).collect();
        let fn_type = if ret_type == OtterType::Unit {
            self.context.void_type().fn_type(&param_metadata, false)
        } else {
            match self.basic_type(ret_type)? {
                BasicTypeEnum::IntType(t) => t.fn_type(&param_metadata, false),
                BasicTypeEnum::FloatType(t) => t.fn_type(&param_metadata, false),
                BasicTypeEnum::PointerType(t) => t.fn_type(&param_metadata, false),
                _ => bail!("unsupported return type"),
            }
        };

        let llvm_fn = self.module.add_function(&function.name, fn_type, None);

        // Add optimization hints for better code generation
        if let Some(_entry) = llvm_fn.get_first_basic_block() {
            // Mark the entry block as cold if it's not the main function
            // This helps the optimizer prioritize hot paths
            if function.name != "main" {
                // Add attributes that help with optimization
                llvm_fn.add_attribute(
                    inkwell::attributes::AttributeLoc::Function,
                    self.context.create_enum_attribute(
                        inkwell::attributes::Attribute::get_named_enum_kind_id("noinline"),
                        0,
                    ),
                );
            }
        }

        Ok(llvm_fn)
    }

    fn lower_function_body(&mut self, function: &Function) -> Result<()> {
        let llvm_fn = self
            .module
            .get_function(&function.name)
            .ok_or_else(|| anyhow!("function {} not declared", function.name))?;

        let entry = self.context.append_basic_block(llvm_fn, "entry");
        self.builder.position_at_end(entry);

        let mut ctx = FunctionContext::new();

        // Store parameters as local variables
        for (i, param) in function.params.iter().enumerate() {
            let param_value = llvm_fn.get_nth_param(i as u32).ok_or_else(|| {
                anyhow!(
                    "failed to get parameter {} for function {}",
                    i,
                    function.name
                )
            })?;

            let param_ty = if let Some(ty) = &param.ty {
                self.type_from_ast(ty)?
            } else {
                OtterType::F64
            };

            let alloca = self
                .builder
                .build_alloca(self.basic_type(param_ty)?, &param.name);
            self.builder.build_store(alloca, param_value);
            ctx.insert(
                param.name.clone(),
                Variable {
                    ptr: alloca,
                    ty: param_ty,
                },
            );
        }

        for statement in &function.body.statements {
            self.lower_statement(statement, llvm_fn, &mut ctx)?;
        }

        // Add default return if needed
        if self
            .builder
            .get_insert_block()
            .and_then(|block| block.get_terminator())
            .is_none()
        {
            let ret_type = if let Some(ret_ty) = &function.ret_ty {
                self.type_from_ast(ret_ty)?
            } else {
                OtterType::I32
            };

            if ret_type == OtterType::Unit {
                self.builder.build_return(None);
            } else {
                // Default return for non-void functions
                match ret_type {
                    OtterType::I32 => {
                        let val = self.context.i32_type().const_zero();
                        self.builder.build_return(Some(&val));
                    }
                    OtterType::I64 => {
                        let val = self.context.i64_type().const_zero();
                        self.builder.build_return(Some(&val));
                    }
                    OtterType::F64 => {
                        let val = self.context.f64_type().const_zero();
                        self.builder.build_return(Some(&val));
                    }
                    OtterType::Bool => {
                        let val = self.context.bool_type().const_zero();
                        self.builder.build_return(Some(&val));
                    }
                    _ => bail!("unsupported return type"),
                };
            }
        }

        Ok(())
    }

    fn type_from_name(&self, name: &str) -> Result<OtterType> {
        match name {
            "int" => Ok(OtterType::I64),
            "float" => Ok(OtterType::F64),
            "bool" => Ok(OtterType::Bool),
            "str" => Ok(OtterType::Str),
            _ => bail!("unknown type: {}", name),
        }
    }

    fn type_from_ast(&self, ty: &Type) -> Result<OtterType> {
        match ty {
            Type::Simple(name) => self.type_from_name(name),
            Type::Generic { base, args: _ } => {
                // For now, we ignore generic arguments and just use the base type
                // This is a temporary solution - proper generic type handling would require
                // more complex type system changes
                match base.as_str() {
                    "Channel" => {
                        // Channels are currently represented as i64 (pointer/handle)
                        // In a full implementation, we'd need proper generic type handling
                        Ok(OtterType::I64)
                    }
                    _ => bail!("unknown generic type: {}", base),
                }
            }
        }
    }

    fn lower_statement(
        &mut self,
        statement: &Statement,
        _function: FunctionValue<'ctx>,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<()> {
        match statement {
            Statement::Expr(expr) => {
                // Just evaluate the expression (e.g., function calls like print())
                self.eval_expr(expr, ctx)?;
                Ok(())
            }
            Statement::Let { name, expr, public: _ } => {
                let evaluated = self.eval_expr(expr, ctx)?;
                if evaluated.ty == OtterType::Unit {
                    bail!("cannot declare variable `{name}` with unit value");
                }

                let value = evaluated
                    .value
                    .clone()
                    .ok_or_else(|| anyhow!("expected value for `{name}`"))?;

                let ty = self.basic_type(evaluated.ty)?;
                let alloca = self.builder.build_alloca(ty, name);
                self.builder.build_store(alloca, value);

                ctx.insert(
                    name.clone(),
                    Variable {
                        ptr: alloca,
                        ty: evaluated.ty,
                    },
                );
                Ok(())
            }
            Statement::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
            } => self.lower_if_statement(
                _function,
                ctx,
                cond,
                then_block,
                elif_blocks,
                else_block.as_ref(),
            ),
            Statement::For {
                var,
                iterable,
                body,
            } => {
                // For now, we only support range expressions
                if let Expr::Range { start, end } = iterable {
                    let start_val = self.eval_expr(start, ctx)?;
                    let end_val = self.eval_expr(end, ctx)?;

                    // Support both int and float ranges, coercing int to float if needed
                    let (start_num, end_num, loop_ty, is_float): (
                        BasicValueEnum,
                        BasicValueEnum,
                        OtterType,
                        bool,
                    ) = match (start_val.ty, end_val.ty) {
                        (OtterType::F64, OtterType::F64) => {
                            let start = start_val
                                .value
                                .ok_or_else(|| anyhow!("missing start value"))?
                                .into_float_value();
                            let end = end_val
                                .value
                                .ok_or_else(|| anyhow!("missing end value"))?
                                .into_float_value();
                            (start.into(), end.into(), OtterType::F64, true)
                        }
                        (OtterType::I64, OtterType::I64) | (OtterType::I32, OtterType::I32) => {
                            let start = start_val
                                .value
                                .ok_or_else(|| anyhow!("missing start value"))?
                                .into_int_value();
                            let end = end_val
                                .value
                                .ok_or_else(|| anyhow!("missing end value"))?
                                .into_int_value();
                            (start.into(), end.into(), OtterType::I64, false)
                        }
                        (OtterType::F64, OtterType::I64) | (OtterType::I64, OtterType::F64) => {
                            // Coerce to float
                            let start = if start_val.ty == OtterType::F64 {
                                start_val
                                    .value
                                    .ok_or_else(|| anyhow!("missing start value"))?
                                    .into_float_value()
                            } else {
                                let int_val = start_val
                                    .value
                                    .ok_or_else(|| anyhow!("missing start value"))?
                                    .into_int_value();
                                self.builder.build_signed_int_to_float(
                                    int_val,
                                    self.context.f64_type(),
                                    "start_to_float",
                                )
                            };
                            let end = if end_val.ty == OtterType::F64 {
                                end_val
                                    .value
                                    .ok_or_else(|| anyhow!("missing end value"))?
                                    .into_float_value()
                            } else {
                                let int_val = end_val
                                    .value
                                    .ok_or_else(|| anyhow!("missing end value"))?
                                    .into_int_value();
                                self.builder.build_signed_int_to_float(
                                    int_val,
                                    self.context.f64_type(),
                                    "end_to_float",
                                )
                            };
                            (start.into(), end.into(), OtterType::F64, true)
                        }
                        _ => bail!("for loop range start and end must be numeric types"),
                    };

                    // Create loop blocks
                    let loop_header = self.context.append_basic_block(_function, "loop_header");
                    let loop_body = self.context.append_basic_block(_function, "loop_body");
                    let loop_end = self.context.append_basic_block(_function, "loop_end");

                    // Push loop context for break/continue (continue goes to header, break goes to end)
                    ctx.push_loop(loop_header, loop_end);

                    // Allocate loop variable
                    let loop_var_type = self.basic_type(loop_ty)?;
                    let loop_var_ptr = self.builder.build_alloca(loop_var_type, var);
                    self.builder.build_store(loop_var_ptr, start_num);
                    ctx.insert(
                        var.clone(),
                        Variable {
                            ptr: loop_var_ptr,
                            ty: loop_ty,
                        },
                    );

                    // Jump to loop header
                    self.builder.build_unconditional_branch(loop_header);

                    // Loop header: check condition
                    self.builder.position_at_end(loop_header);
                    let current = self
                        .builder
                        .build_load(loop_var_type, loop_var_ptr, "current");

                    let cond = if is_float {
                        self.builder.build_float_compare(
                            inkwell::FloatPredicate::OLT,
                            current.into_float_value(),
                            end_num.into_float_value(),
                            "loop_cond",
                        )
                    } else {
                        self.builder.build_int_compare(
                            inkwell::IntPredicate::SLT,
                            current.into_int_value(),
                            end_num.into_int_value(),
                            "loop_cond",
                        )
                    };
                    self.builder
                        .build_conditional_branch(cond, loop_body, loop_end);

                    // Loop body
                    self.builder.position_at_end(loop_body);
                    for stmt in &body.statements {
                        self.lower_statement(stmt, _function, ctx)?;
                    }

                    // Increment loop variable
                    let current = self
                        .builder
                        .build_load(loop_var_type, loop_var_ptr, "current");
                    let next: BasicValueEnum = if is_float {
                        let one = self.context.f64_type().const_float(1.0);
                        self.builder
                            .build_float_add(current.into_float_value(), one, "next")
                            .into()
                    } else {
                        let one = self.context.i64_type().const_int(1, false);
                        self.builder
                            .build_int_add(current.into_int_value(), one, "next")
                            .into()
                    };
                    self.builder.build_store(loop_var_ptr, next);
                    self.builder.build_unconditional_branch(loop_header);

                    // Pop loop context
                    ctx.pop_loop();

                    // Continue after loop
                    self.builder.position_at_end(loop_end);
                    Ok(())
                } else {
                    bail!("for loops currently only support range expressions");
                }
            }
            Statement::While { cond, body } => self.lower_while_loop(_function, ctx, cond, body),
            Statement::Break => {
                if let Some(loop_ctx) = ctx.current_loop() {
                    self.builder.build_unconditional_branch(loop_ctx.break_bb);
                    // Create a new unreachable block to continue code generation
                    let unreachable_bb = self.context.append_basic_block(_function, "unreachable");
                    self.builder.position_at_end(unreachable_bb);
                } else {
                    bail!("break statement outside of loop");
                }
                Ok(())
            }
            Statement::Continue => {
                if let Some(loop_ctx) = ctx.current_loop() {
                    self.builder
                        .build_unconditional_branch(loop_ctx.continue_bb);
                    // Create a new unreachable block to continue code generation
                    let unreachable_bb = self.context.append_basic_block(_function, "unreachable");
                    self.builder.position_at_end(unreachable_bb);
                } else {
                    bail!("continue statement outside of loop");
                }
                Ok(())
            }
            Statement::Function(_) => {
                // Functions are already lowered in lower_program
                Ok(())
            }
            Statement::Use {
                module: _,
                alias: _,
            } => {
                // Register module import for later resolution
                // The actual resolution happens at the expression level when accessing module.field
                // Module validation occurs when functions from the module are actually called
                Ok(())
            }
            Statement::Block(block) => {
                for stmt in &block.statements {
                    self.lower_statement(stmt, _function, ctx)?;
                }
                Ok(())
            }
            Statement::Return(expr) => {
                if let Some(expr) = expr {
                    let evaluated = self.eval_expr(expr, ctx)?;
                    
                    // Get the function's return type
                    let function_ret_type = if let Some(ret_ty) = &_function.get_type().get_return_type() {
                        if ret_ty.is_float_type() {
                            OtterType::F64
                        } else if ret_ty.is_int_type() {
                            if ret_ty.into_int_type().get_bit_width() == 64 {
                                OtterType::I64
                            } else {
                                OtterType::I32
                            }
                        } else {
                            evaluated.ty
                        }
                    } else {
                        evaluated.ty
                    };
                    
                    // Cast the return value to match the function's return type if needed
                    let return_value = if evaluated.ty != function_ret_type {
                        let value = evaluated.value.ok_or_else(|| anyhow!("return expression has no value"))?;
                        match (function_ret_type, evaluated.ty) {
                            (OtterType::I32, OtterType::F64) => {
                                let float_val = value.into_float_value();
                                let int_val = self.builder.build_float_to_signed_int(
                                    float_val,
                                    self.context.i32_type(),
                                    "coerce_f64_to_i32",
                                );
                                int_val.into()
                            }
                            (OtterType::I64, OtterType::F64) => {
                                let float_val = value.into_float_value();
                                let int_val = self.builder.build_float_to_signed_int(
                                    float_val,
                                    self.context.i64_type(),
                                    "coerce_f64_to_i64",
                                );
                                int_val.into()
                            }
                            (OtterType::F64, OtterType::I32) => {
                                let int_val = value.into_int_value();
                                let float_val = self.builder.build_signed_int_to_float(
                                    int_val,
                                    self.context.f64_type(),
                                    "coerce_i32_to_f64",
                                );
                                float_val.into()
                            }
                            (OtterType::F64, OtterType::I64) => {
                                let int_val = value.into_int_value();
                                let float_val = self.builder.build_signed_int_to_float(
                                    int_val,
                                    self.context.f64_type(),
                                    "coerce_i64_to_f64",
                                );
                                float_val.into()
                            }
                            (OtterType::I64, OtterType::I32) => {
                                let int_val = value.into_int_value();
                                let ext_val = self.builder.build_int_s_extend(
                                    int_val,
                                    self.context.i64_type(),
                                    "coerce_i32_to_i64",
                                );
                                ext_val.into()
                            }
                            _ => value,
                        }
                    } else {
                        evaluated.value.ok_or_else(|| anyhow!("return expression has no value"))?
                    };
                    
                    self.builder.build_return(Some(&return_value));
                } else {
                    self.builder.build_return(None);
                }
                Ok(())
            }
            Statement::Assignment { name, expr } => {
                let evaluated = self.eval_expr(expr, ctx)?;
                if evaluated.ty == OtterType::Unit {
                    bail!("cannot assign unit value to `{name}`");
                }

                let (ptr, evaluated) = if let Some(variable) = ctx.get(name) {
                    // Allow type coercions in assignment
                    let mut eval = evaluated;
                    match (variable.ty, eval.ty) {
                        // Same type - no conversion needed
                        (t1, t2) if t1 == t2 => {},
                        // I32 to I64 extension
                        (OtterType::I64, OtterType::I32) => {
                            let int_val = eval
                                .value
                                .clone()
                                .ok_or_else(|| anyhow!("missing value"))?
                                .into_int_value();
                            let ext_val = self.builder.build_int_s_extend(
                                int_val,
                                self.context.i64_type(),
                                "coerce_i32_to_i64",
                            );
                            eval = EvaluatedValue::with_value(ext_val.into(), OtterType::I64);
                        }
                        // Allow F64 assigned to F64 var (no change needed)
                        (OtterType::F64, OtterType::I64) | (OtterType::F64, OtterType::I32) => {
                            let int_val = eval
                                .value
                                .clone()
                                .ok_or_else(|| anyhow!("missing value"))?
                                .into_int_value();
                            let float_val = self.builder.build_signed_int_to_float(
                                int_val,
                                self.context.f64_type(),
                                "coerce_int_to_f64",
                            );
                            eval = EvaluatedValue::with_value(float_val.into(), OtterType::F64);
                        }
                        _ => {
                            bail!(
                                "type mismatch assigning to `{name}`: existing {:?}, new {:?}",
                                variable.ty,
                                eval.ty
                            );
                        }
                    }
                    (variable.ptr, eval)
                } else {
                    let ty = self.basic_type(evaluated.ty)?;
                    let alloca = self.builder.build_alloca(ty, name);
                    ctx.insert(
                        name.clone(),
                        Variable {
                            ptr: alloca,
                            ty: evaluated.ty,
                        },
                    );
                    (alloca, evaluated)
                };

                let value = evaluated
                    .value
                    .clone()
                    .ok_or_else(|| anyhow!("expected value for assignment to `{name}`"))?;
                self.builder.build_store(ptr, value);
                Ok(())
            }
            Statement::Struct { .. } => {
                // Struct definitions are handled at the module level, not in function bodies
                Ok(())
            }
            Statement::TypeAlias { .. } => {
                // Type aliases are handled at the module level, not in function bodies
                Ok(())
            }
        }
    }

    fn eval_expr(
        &mut self,
        expr: &Expr,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        match expr {
            Expr::Literal(literal) => self.eval_literal(literal),
            Expr::Identifier(name) => self.eval_identifier(name, ctx),
            Expr::Binary { left, op, right } => self.eval_binary_expr(left, op, right, ctx),
            Expr::Call { func, args } => self.eval_call(func, args, ctx),
            Expr::Unary { op, expr } => self.eval_unary_expr(op, expr, ctx),
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => self.eval_if_expr(cond, then_branch, else_branch.as_deref(), ctx),
            Expr::Range { .. } => bail!("Range expressions can only be used in for loops"),
            Expr::FString { parts } => self.eval_fstring(parts, ctx),
            Expr::Member { object, field } => self.eval_member_access(object, field, ctx),
            Expr::Await(expr) => {
                // For now, await just evaluates the inner expression
                // This provides a foundation for when async runtime is implemented
                self.eval_expr(expr, ctx)
            }
            Expr::Spawn(expr) => {
                // For now, spawn evaluates the expression and returns unit
                // representing a task handle. This provides a foundation for
                // when task spawning runtime is implemented.
                self.eval_expr(expr, ctx)?;
                Ok(EvaluatedValue {
                    ty: OtterType::Unit,
                    value: None,
                })
            }
            Expr::Lambda { params: _, ret_ty: _, body } => {
                // Compile lambda to a separate function
                let lambda_name = format!("lambda_{}", self.lambda_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst));

                // For task callbacks, lambdas take no parameters and return void
                let llvm_param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = vec![];
                let llvm_fn_type = self.context.void_type().fn_type(&llvm_param_types, false);

                // Create the lambda function
                let lambda_function = self.module.add_function(&lambda_name, llvm_fn_type, None);

                // Create a basic block for the lambda
                let lambda_bb = self.context.append_basic_block(lambda_function, "entry");

                // Save current builder position
                let current_bb = self.builder.get_insert_block();

                // Switch to the lambda's basic block
                self.builder.position_at_end(lambda_bb);

                // Generate code for lambda body with captured variables
                // For now, we just clone the parent context to capture all variables
                // A more sophisticated implementation would analyze which variables are actually used
                let mut lambda_ctx = ctx.clone();

                // Lower the lambda body statements
                for statement in &body.statements {
                    self.lower_statement(statement, lambda_function, &mut lambda_ctx)?;
                }

                // Ensure the function returns (add return if not present)
                if lambda_bb.get_terminator().is_none() {
                    self.builder.build_return(None);
                }

                // Restore original builder position if it exists
                if let Some(bb) = current_bb {
                    self.builder.position_at_end(bb);
                }

                // Return the function pointer as an i64 (cast the function pointer)
                let func_ptr = lambda_function.as_global_value().as_pointer_value();
                let int_ptr = self.builder.build_ptr_to_int(
                    func_ptr,
                    self.context.i64_type(),
                    "lambda_ptr",
                );

                Ok(EvaluatedValue {
                    ty: OtterType::I64, // Function pointers are i64
                    value: Some(int_ptr.into()),
                })
            }
            Expr::Match { .. } => {
                // Match expressions need pattern matching implementation
                bail!("match expressions are not yet implemented in codegen")
            }
            Expr::Array(_) => {
                // Array literals need array implementation
                bail!("array literals are not yet implemented in codegen")
            }
            Expr::Dict(_) => {
                // Dictionary literals need dictionary implementation
                bail!("dictionary literals are not yet implemented in codegen")
            }
        }
    }

    fn eval_if_expr(
        &mut self,
        cond: &Expr,
        then_branch: &Expr,
        else_branch: Option<&Expr>,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        // Evaluate condition
        let cond_value = self.eval_expr(cond, ctx)?;
        if cond_value.ty != OtterType::Bool {
            bail!(
                "if expression condition must be a boolean, got {:?}",
                cond_value.ty
            );
        }

        // Create basic blocks for the branches
        let current_function = self
            .builder
            .get_insert_block()
            .and_then(|bb| bb.get_parent())
            .ok_or_else(|| anyhow!("not in a function"))?;

        let then_bb = self.context.append_basic_block(current_function, "if_then");
        let else_bb = self.context.append_basic_block(current_function, "if_else");
        let merge_bb = self
            .context
            .append_basic_block(current_function, "if_merge");

        let cond_bool = cond_value
            .value
            .ok_or_else(|| anyhow!("missing condition value"))?
            .into_int_value();

        self.builder
            .build_conditional_branch(cond_bool, then_bb, else_bb);

        // Evaluate then expression
        self.builder.position_at_end(then_bb);
        let then_value = self.eval_expr(then_branch, ctx)?;
        let then_result_ptr = if let Some(value) = then_value.value {
            let result_type = self.basic_type(then_value.ty)?;
            let result_ptr = self.builder.build_alloca(result_type, "then_result");
            self.builder.build_store(result_ptr, value);
            Some(result_ptr)
        } else {
            None
        };
        self.builder.build_unconditional_branch(merge_bb);

        // Evaluate else expression
        self.builder.position_at_end(else_bb);
        let else_value = if let Some(else_branch) = else_branch {
            self.eval_expr(else_branch, ctx)?
        } else {
            // If no else branch, use unit type
            EvaluatedValue {
                ty: OtterType::Unit,
                value: None,
            }
        };
        let else_result_ptr = if let Some(value) = else_value.value {
            let result_type = self.basic_type(else_value.ty)?;
            let result_ptr = self.builder.build_alloca(result_type, "else_result");
            self.builder.build_store(result_ptr, value);
            Some(result_ptr)
        } else {
            None
        };
        self.builder.build_unconditional_branch(merge_bb);

        // Merge the results
        self.builder.position_at_end(merge_bb);

        if then_value.ty != else_value.ty {
            bail!(
                "if expression branches must have the same type: {:?} vs {:?}",
                then_value.ty,
                else_value.ty
            );
        }

        if let (Some(then_ptr), Some(else_ptr)) = (then_result_ptr, else_result_ptr) {
            let result_type = self.basic_type(then_value.ty)?;
            let phi = self.builder.build_phi(result_type, "if_result");
            phi.add_incoming(&[
                (
                    &self.builder.build_load(result_type, then_ptr, "then_load"),
                    then_bb,
                ),
                (
                    &self.builder.build_load(result_type, else_ptr, "else_load"),
                    else_bb,
                ),
            ]);
            Ok(EvaluatedValue::with_value(
                phi.as_basic_value(),
                then_value.ty,
            ))
        } else {
            // Unit type
            Ok(EvaluatedValue {
                ty: OtterType::Unit,
                value: None,
            })
        }
    }

    fn eval_identifier(
        &mut self,
        name: &str,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        if let Some(variable) = ctx.get(name) {
            let ty = self.basic_type(variable.ty)?;
            let loaded = self.builder.build_load(ty, variable.ptr, name);
            Ok(EvaluatedValue::with_value(loaded, variable.ty))
        } else {
            bail!("unknown identifier `{name}`");
        }
    }

    fn eval_binary_expr(
        &mut self,
        left: &Expr,
        op: &BinaryOp,
        right: &Expr,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        let mut left_value = self.eval_expr(left, ctx)?;
        let mut right_value = self.eval_expr(right, ctx)?;

        // Normalize integer types: I32 -> I64
        if left_value.ty == OtterType::I32 {
            let int_val = left_value
                .value
                .ok_or_else(|| anyhow!("missing value"))?
                .into_int_value();
            let ext_val = self.builder.build_int_s_extend(
                int_val,
                self.context.i64_type(),
                "i32toi64",
            );
            left_value = EvaluatedValue::with_value(ext_val.into(), OtterType::I64);
        }
        if right_value.ty == OtterType::I32 {
            let int_val = right_value
                .value
                .ok_or_else(|| anyhow!("missing value"))?
                .into_int_value();
            let ext_val = self.builder.build_int_s_extend(
                int_val,
                self.context.i64_type(),
                "i32toi64",
            );
            right_value = EvaluatedValue::with_value(ext_val.into(), OtterType::I64);
        }

        // Coerce int to float if needed
        if left_value.ty == OtterType::I64 && right_value.ty == OtterType::F64 {
            let int_val = left_value
                .value
                .ok_or_else(|| anyhow!("missing value"))?
                .into_int_value();
            let float_val = self.builder.build_signed_int_to_float(
                int_val,
                self.context.f64_type(),
                "inttofloat",
            );
            left_value = EvaluatedValue::with_value(float_val.into(), OtterType::F64);
        } else if left_value.ty == OtterType::F64 && right_value.ty == OtterType::I64 {
            let int_val = right_value
                .value
                .ok_or_else(|| anyhow!("missing value"))?
                .into_int_value();
            let float_val = self.builder.build_signed_int_to_float(
                int_val,
                self.context.f64_type(),
                "inttofloat",
            );
            right_value = EvaluatedValue::with_value(float_val.into(), OtterType::F64);
        }

        if left_value.ty != right_value.ty {
            bail!(
                "binary operation type mismatch: {:?} vs {:?}",
                left_value.ty,
                right_value.ty
            );
        }

        // Support both integer and float operations
        match left_value.ty {
            OtterType::I64 => {
                let lhs = left_value
                    .value
                    .clone()
                    .ok_or_else(|| anyhow!("left operand missing value"))?
                    .into_int_value();
                let rhs = right_value
                    .value
                    .clone()
                    .ok_or_else(|| anyhow!("right operand missing value"))?
                    .into_int_value();

                let result = match op {
                    BinaryOp::Add => self.builder.build_int_add(lhs, rhs, "addtmp").into(),
                    BinaryOp::Sub => self.builder.build_int_sub(lhs, rhs, "subtmp").into(),
                    BinaryOp::Mul => self.builder.build_int_mul(lhs, rhs, "multmp").into(),
                    BinaryOp::Div => self.builder.build_int_signed_div(lhs, rhs, "divtmp").into(),
                    BinaryOp::Mod => self.builder.build_int_signed_rem(lhs, rhs, "modtmp").into(),
                    BinaryOp::Eq => {
                        let cmp = self.builder.build_int_compare(
                            inkwell::IntPredicate::EQ,
                            lhs,
                            rhs,
                            "eqtmp",
                        );
                        return Ok(EvaluatedValue::with_value(cmp.into(), OtterType::Bool));
                    }
                    BinaryOp::Ne => {
                        let cmp = self.builder.build_int_compare(
                            inkwell::IntPredicate::NE,
                            lhs,
                            rhs,
                            "netmp",
                        );
                        return Ok(EvaluatedValue::with_value(cmp.into(), OtterType::Bool));
                    }
                    BinaryOp::Lt => {
                        let cmp = self.builder.build_int_compare(
                            inkwell::IntPredicate::SLT,
                            lhs,
                            rhs,
                            "lttmp",
                        );
                        return Ok(EvaluatedValue::with_value(cmp.into(), OtterType::Bool));
                    }
                    BinaryOp::LtEq => {
                        let cmp = self.builder.build_int_compare(
                            inkwell::IntPredicate::SLE,
                            lhs,
                            rhs,
                            "letmp",
                        );
                        return Ok(EvaluatedValue::with_value(cmp.into(), OtterType::Bool));
                    }
                    BinaryOp::Gt => {
                        let cmp = self.builder.build_int_compare(
                            inkwell::IntPredicate::SGT,
                            lhs,
                            rhs,
                            "gttmp",
                        );
                        return Ok(EvaluatedValue::with_value(cmp.into(), OtterType::Bool));
                    }
                    BinaryOp::GtEq => {
                        let cmp = self.builder.build_int_compare(
                            inkwell::IntPredicate::SGE,
                            lhs,
                            rhs,
                            "getmp",
                        );
                        return Ok(EvaluatedValue::with_value(cmp.into(), OtterType::Bool));
                    }
                    _ => bail!("unsupported binary operation for integers: {:?}", op),
                };
                return Ok(EvaluatedValue::with_value(result, OtterType::I64));
            }
            OtterType::F64 => {
                let lhs = left_value
                    .value
                    .clone()
                    .ok_or_else(|| anyhow!("left operand missing value"))?
                    .into_float_value();
                let rhs = right_value
                    .value
                    .clone()
                    .ok_or_else(|| anyhow!("right operand missing value"))?
                    .into_float_value();

                let result = match op {
            BinaryOp::Add => self.builder.build_float_add(lhs, rhs, "addtmp").into(),
            BinaryOp::Sub => self.builder.build_float_sub(lhs, rhs, "subtmp").into(),
            BinaryOp::Mul => self.builder.build_float_mul(lhs, rhs, "multmp").into(),
            BinaryOp::Div => self.builder.build_float_div(lhs, rhs, "divtmp").into(),
            BinaryOp::Mod => self.builder.build_float_rem(lhs, rhs, "modtmp").into(),
            BinaryOp::Eq => {
                let cmp = self.builder.build_float_compare(
                    inkwell::FloatPredicate::OEQ,
                    lhs,
                    rhs,
                    "eqtmp",
                );
                return Ok(EvaluatedValue::with_value(cmp.into(), OtterType::Bool));
            }
            BinaryOp::Ne => {
                let cmp = self.builder.build_float_compare(
                    inkwell::FloatPredicate::ONE,
                    lhs,
                    rhs,
                    "neqtmp",
                );
                return Ok(EvaluatedValue::with_value(cmp.into(), OtterType::Bool));
            }
            BinaryOp::Lt => {
                let cmp = self.builder.build_float_compare(
                    inkwell::FloatPredicate::OLT,
                    lhs,
                    rhs,
                    "lttmp",
                );
                return Ok(EvaluatedValue::with_value(cmp.into(), OtterType::Bool));
            }
            BinaryOp::Gt => {
                let cmp = self.builder.build_float_compare(
                    inkwell::FloatPredicate::OGT,
                    lhs,
                    rhs,
                    "gttmp",
                );
                return Ok(EvaluatedValue::with_value(cmp.into(), OtterType::Bool));
            }
            BinaryOp::LtEq => {
                let cmp = self.builder.build_float_compare(
                    inkwell::FloatPredicate::OLE,
                    lhs,
                    rhs,
                    "letmp",
                );
                return Ok(EvaluatedValue::with_value(cmp.into(), OtterType::Bool));
            }
            BinaryOp::GtEq => {
                let cmp = self.builder.build_float_compare(
                    inkwell::FloatPredicate::OGE,
                    lhs,
                    rhs,
                    "getmp",
                );
                return Ok(EvaluatedValue::with_value(cmp.into(), OtterType::Bool));
            }
            BinaryOp::And => {
                // Convert float to bool (non-zero = true)
                let lhs_bool = self.builder.build_float_compare(
                    inkwell::FloatPredicate::ONE,
                    lhs,
                    self.context.f64_type().const_float(0.0),
                    "lhs_bool",
                );
                let rhs_bool = self.builder.build_float_compare(
                    inkwell::FloatPredicate::ONE,
                    rhs,
                    self.context.f64_type().const_float(0.0),
                    "rhs_bool",
                );
                let and_result = self.builder.build_and(lhs_bool, rhs_bool, "and_result");
                return Ok(EvaluatedValue::with_value(
                    and_result.into(),
                    OtterType::Bool,
                ));
            }
            BinaryOp::Or => {
                // Convert float to bool (non-zero = true)
                let lhs_bool = self.builder.build_float_compare(
                    inkwell::FloatPredicate::ONE,
                    lhs,
                    self.context.f64_type().const_float(0.0),
                    "lhs_bool",
                );
                let rhs_bool = self.builder.build_float_compare(
                    inkwell::FloatPredicate::ONE,
                    rhs,
                    self.context.f64_type().const_float(0.0),
                    "rhs_bool",
                );
                let or_result = self.builder.build_or(lhs_bool, rhs_bool, "or_result");
                return Ok(EvaluatedValue::with_value(
                    or_result.into(),
                    OtterType::Bool,
                ));
            }
                };
                return Ok(EvaluatedValue::with_value(result, OtterType::F64));
            }
            _ => bail!(
                "binary expressions support only integers and floats, got {:?}",
                left_value.ty
            ),
        }
    }

    fn eval_unary_expr(
        &mut self,
        op: &crate::ast::nodes::UnaryOp,
        expr: &Expr,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        let val = self.eval_expr(expr, ctx)?;
        match op {
            crate::ast::nodes::UnaryOp::Neg => {
                if val.ty != OtterType::F64 {
                    bail!("negation only supported for floats currently");
                }
                let float_val = val
                    .value
                    .ok_or_else(|| anyhow!("missing value"))?
                    .into_float_value();
                let neg = self.builder.build_float_neg(float_val, "negtmp");
                Ok(EvaluatedValue::with_value(neg.into(), OtterType::F64))
            }
            crate::ast::nodes::UnaryOp::Not => {
                if val.ty != OtterType::Bool {
                    bail!("logical not only supported for booleans");
                }
                let bool_val = val
                    .value
                    .ok_or_else(|| anyhow!("missing value"))?
                    .into_int_value();
                let not = self.builder.build_not(bool_val, "nottmp");
                Ok(EvaluatedValue::with_value(not.into(), OtterType::Bool))
            }
        }
    }

    fn eval_member_access(
        &mut self,
        object: &Expr,
        field: &str,
        _ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        if let Expr::Identifier(module_name) = object {
            let full_name = format!("{}.{}", module_name, field);
            match full_name.as_str() {
                "time.now" => {
                    let function = self.declare_symbol_function("std.time.now")?;
                    let call = self.builder.build_call(function, &[], "call_time_now");
                    let value = call
                        .try_as_basic_value()
                        .left()
                        .ok_or_else(|| anyhow!("call to `time.now` did not produce a value"))?;
                    // time.now() returns i64 milliseconds, convert to f64
                    let final_value = if value.is_int_value() {
                        let int_val = value.into_int_value();
                        self.builder
                            .build_signed_int_to_float(
                                int_val,
                                self.context.f64_type(),
                                "int_to_float",
                            )
                            .into()
                    } else if value.is_float_value() {
                        value
                    } else {
                        bail!("unexpected return type from time.now");
                    };
                    Ok(EvaluatedValue::with_value(final_value, OtterType::F64))
                }
                _ => {
                    if let Some(symbol) = self.symbol_registry.resolve(&full_name) {
                        let function = self.declare_symbol_function(&full_name)?;
                        let call = self.builder.build_call(
                            function,
                            &[],
                            &format!("call_{}", full_name.replace(':', "_").replace('.', "_")),
                        );
                        let return_ty: OtterType = symbol.signature.result.into();
                        let value = match return_ty {
                            OtterType::Unit => None,
                            _ => Some(call.try_as_basic_value().left().ok_or_else(|| {
                                anyhow!("call to `{}` did not produce a return value", full_name)
                            })?),
                        };
                        Ok(EvaluatedValue {
                            ty: return_ty,
                            value,
                        })
                    } else {
                        bail!("unknown member access: {}.{}", module_name, field);
                    }
                }
            }
        } else {
            bail!("member access currently only supports module.field syntax");
        }
    }

    fn eval_literal(&mut self, literal: &Literal) -> Result<EvaluatedValue<'ctx>> {
        match literal {
            Literal::String(value) => {
                // Use a more descriptive name for string literals to help with debugging
                let global_name = format!("str_lit_{}", value.len());
                let global = self.builder.build_global_string_ptr(value, &global_name);
                Ok(EvaluatedValue::with_value(
                    global.as_pointer_value().into(),
                    OtterType::Str,
                ))
            }
            Literal::Number(num) => {
                // Always use F64 for floating point representation to avoid type conflicts
                // The parser stores all numbers as f64, so we respect that
                let float = self.context.f64_type().const_float(num.value);
                Ok(EvaluatedValue::with_value(float.into(), OtterType::F64))
            }
            Literal::Bool(value) => {
                let bool_val = self.context.bool_type().const_int(*value as u64, false);
                Ok(EvaluatedValue::with_value(bool_val.into(), OtterType::Bool))
            }
            Literal::Unit => {
                // Unit type has no value
                Ok(EvaluatedValue {
                    ty: OtterType::Unit,
                    value: None,
                })
            }
        }
    }

    fn eval_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        let symbol_name = match callee {
            Expr::Identifier(name) => Some(name.clone()),
            Expr::Member { object, field } => {
                if let Expr::Identifier(module) = object.as_ref() {
                    Some(format!("{module}.{field}"))
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some(symbol_name) = symbol_name {
            if let Some(symbol) = self.symbol_registry.resolve(&symbol_name) {
                if symbol.signature.params.len() != args.len() {
                    bail!(
                        "function `{symbol_name}` expected {} arguments but got {}",
                        symbol.signature.params.len(),
                        args.len()
                    );
                }

                let function = self.declare_symbol_function(&symbol_name)?;
                let mut lowered_args = Vec::with_capacity(args.len());

                for (expr, expected) in args.iter().zip(symbol.signature.params.iter()) {
                    let mut value = self.eval_expr(expr, ctx)?;
                    let expected_ty: OtterType = expected.clone().into();
                    if value.ty != expected_ty {
                        match (expected_ty, value.ty) {
                            (OtterType::F64, OtterType::I64) => {
                                let int_val = value
                                    .value
                                    .clone()
                                    .ok_or_else(|| anyhow!("missing value"))?
                                    .into_int_value();
                                let float_val = self.builder.build_signed_int_to_float(
                                    int_val,
                                    self.context.f64_type(),
                                    "coerce_i64_to_f64",
                                );
                                value =
                                    EvaluatedValue::with_value(float_val.into(), OtterType::F64);
                            }
                            (OtterType::F64, OtterType::I32) => {
                                let int_val = value
                                    .value
                                    .clone()
                                    .ok_or_else(|| anyhow!("missing value"))?
                                    .into_int_value();
                                let float_val = self.builder.build_signed_int_to_float(
                                    int_val,
                                    self.context.f64_type(),
                                    "coerce_i32_to_f64",
                                );
                                value =
                                    EvaluatedValue::with_value(float_val.into(), OtterType::F64);
                            }
                            (OtterType::I64, OtterType::I32) => {
                                let int_val = value
                                    .value
                                    .clone()
                                    .ok_or_else(|| anyhow!("missing value"))?
                                    .into_int_value();
                                let ext_val = self.builder.build_int_s_extend(
                                    int_val,
                                    self.context.i64_type(),
                                    "coerce_i32_to_i64",
                                );
                                value =
                                    EvaluatedValue::with_value(ext_val.into(), OtterType::I64);
                            }
                            (OtterType::Opaque, OtterType::I64) => {
                                // I64 can be used as opaque (e.g., handles)
                                value.ty = OtterType::Opaque;
                            }
                            (OtterType::Opaque, OtterType::I32) => {
                                // I32 can be used as opaque (e.g., handles)
                                let int_val = value
                                    .value
                                    .clone()
                                    .ok_or_else(|| anyhow!("missing value"))?
                                    .into_int_value();
                                let ext_val = self.builder.build_int_s_extend(
                                    int_val,
                                    self.context.i64_type(),
                                    "coerce_i32_to_opaque",
                                );
                                value =
                                    EvaluatedValue::with_value(ext_val.into(), OtterType::Opaque);
                            }
                            (OtterType::I64, OtterType::F64) => {
                                // F64 to I64 conversion (truncate) - for integer literals written as 0.0
                                let float_val = value
                                    .value
                                    .clone()
                                    .ok_or_else(|| anyhow!("missing value"))?
                                    .into_float_value();
                                let int_val = self.builder.build_float_to_signed_int(
                                    float_val,
                                    self.context.i64_type(),
                                    "coerce_f64_to_i64",
                                );
                                value =
                                    EvaluatedValue::with_value(int_val.into(), OtterType::I64);
                            }
                            (OtterType::I32, OtterType::F64) => {
                                // F64 to I32 conversion (truncate)
                                let float_val = value
                                    .value
                                    .clone()
                                    .ok_or_else(|| anyhow!("missing value"))?
                                    .into_float_value();
                                let int_val = self.builder.build_float_to_signed_int(
                                    float_val,
                                    self.context.i32_type(),
                                    "coerce_f64_to_i32",
                                );
                                value =
                                    EvaluatedValue::with_value(int_val.into(), OtterType::I32);
                            }
                            (OtterType::Opaque, OtterType::F64) => {
                                // F64 to Opaque (truncate to I64 first)
                                let float_val = value
                                    .value
                                    .clone()
                                    .ok_or_else(|| anyhow!("missing value"))?
                                    .into_float_value();
                                let int_val = self.builder.build_float_to_signed_int(
                                    float_val,
                                    self.context.i64_type(),
                                    "coerce_f64_to_opaque",
                                );
                                value =
                                    EvaluatedValue::with_value(int_val.into(), OtterType::Opaque);
                            }
                            _ => {
                                bail!(
                                        "argument type mismatch for `{symbol_name}`: expected {:?}, found {:?}",
                                        expected_ty,
                                        value.ty
                                    );
                            }
                        }
                    }
                    lowered_args.push(self.value_to_metadata(&value)?);
                }

                let call_name = format!("call_{}", symbol_name.replace('.', "_").replace(':', "_"));
                let call = self.builder.build_call(function, &lowered_args, &call_name);
                let return_ty: OtterType = symbol.signature.result.into();
                let value = match return_ty {
                    OtterType::Unit => None,
                    _ => Some(call.try_as_basic_value().left().ok_or_else(|| {
                        anyhow!("call to `{symbol_name}` did not produce a return value")
                    })?),
                };
                return Ok(EvaluatedValue {
                    ty: return_ty,
                    value,
                });
            }

            if let Expr::Identifier(name) = callee {
                return self.call_user_defined_function(name, args, ctx);
            }

            bail!("unknown function `{symbol_name}`");
        }

        if let Expr::Identifier(name) = callee {
            return self.call_user_defined_function(name, args, ctx);
        }

        bail!("only identifier calls are supported");
    }

    fn call_user_defined_function(
        &mut self,
        name: &str,
        args: &[Expr],
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        if let Some(function) = self.module.get_function(name) {
            let fn_type = function.get_type();
            let param_types = fn_type.get_param_types();

            let mut lowered_args = Vec::with_capacity(args.len());
            for (i, arg_expr) in args.iter().enumerate() {
                let mut value = self.eval_expr(arg_expr, ctx)?;

                if i < param_types.len() {
                    let expected_llvm_ty = param_types[i];

                    if expected_llvm_ty.is_int_type() && value.ty == OtterType::F64 {
                        let float_val = value
                            .value
                            .ok_or_else(|| anyhow!("missing value"))?
                            .into_float_value();
                        let int_val = self.builder.build_float_to_signed_int(
                            float_val,
                            self.context.i64_type(),
                            "float_to_int",
                        );
                        value = EvaluatedValue::with_value(int_val.into(), OtterType::I64);
                    } else if expected_llvm_ty.is_float_type() && value.ty == OtterType::I64 {
                        let int_val = value
                            .value
                            .ok_or_else(|| anyhow!("missing value"))?
                            .into_int_value();
                        let float_val = self.builder.build_signed_int_to_float(
                            int_val,
                            self.context.f64_type(),
                            "int_to_float",
                        );
                        value = EvaluatedValue::with_value(float_val.into(), OtterType::F64);
                    }
                }

                lowered_args.push(self.value_to_metadata(&value)?);
            }

            let call = self
                .builder
                .build_call(function, &lowered_args, &format!("call_{name}"));

            let return_type = fn_type.get_return_type();

            if let Some(ret_ty) = return_type {
                let value = call
                    .try_as_basic_value()
                    .left()
                    .ok_or_else(|| anyhow!("call to `{name}` did not produce a value"))?;
                let otter_ty = if ret_ty.is_float_type() {
                    OtterType::F64
                } else if ret_ty.is_int_type() {
                    OtterType::I64
                } else {
                    OtterType::I32
                };
                Ok(EvaluatedValue::with_value(value, otter_ty))
            } else {
                Ok(EvaluatedValue {
                    ty: OtterType::Unit,
                    value: None,
                })
            }
        } else {
            bail!("unknown function `{name}`");
        }
    }

    fn basic_type(&self, ty: OtterType) -> Result<BasicTypeEnum<'ctx>> {
        let ty = match ty {
            OtterType::Unit => bail!("unit type has no runtime representation"),
            OtterType::Bool => self.context.bool_type().into(),
            OtterType::I32 => self.context.i32_type().into(),
            OtterType::I64 => self.context.i64_type().into(),
            OtterType::F64 => self.context.f64_type().into(),
            OtterType::Str => self.string_ptr_type.into(),
            OtterType::Opaque => self.context.i64_type().into(),
        };
        Ok(ty)
    }

    fn value_to_metadata(
        &self,
        value: &EvaluatedValue<'ctx>,
    ) -> Result<BasicMetadataValueEnum<'ctx>> {
        let basic = value
            .value
            .clone()
            .ok_or_else(|| anyhow!("expected value for call argument"))?;
        Ok(basic.into())
    }

    fn declare_symbol_function(&mut self, name: &str) -> Result<FunctionValue<'ctx>> {
        // Check cache first
        if let Some(function) = self.declared_functions.get(name) {
            return Ok(*function);
        }

        let entry = self
            .symbol_registry
            .resolve(name)
            .ok_or_else(|| anyhow!("unresolved symbol `{name}`"))?;

        if let Some(function) = self.module.get_function(&entry.symbol) {
            self.declared_functions.insert(name.to_string(), function);
            return Ok(function);
        }

        let fn_type = self.ffi_signature_to_fn_type(&entry.signature)?;
        let function = self.module.add_function(&entry.symbol, fn_type, None);
        self.declared_functions.insert(name.to_string(), function);
        Ok(function)
    }

    fn ffi_signature_to_fn_type(&self, signature: &FfiSignature) -> Result<FunctionType<'ctx>> {
        let params = self.ffi_param_types(&signature.params)?;
        let fn_type = match signature.result {
            FfiType::Unit => self.context.void_type().fn_type(&params, false),
            FfiType::Bool => self.context.bool_type().fn_type(&params, false),
            FfiType::I32 => self.context.i32_type().fn_type(&params, false),
            FfiType::I64 => self.context.i64_type().fn_type(&params, false),
            FfiType::F64 => self.context.f64_type().fn_type(&params, false),
            FfiType::Str => self.string_ptr_type.fn_type(&params, false),
            FfiType::Opaque => self.context.i64_type().fn_type(&params, false),
        };
        Ok(fn_type)
    }

    fn ffi_param_types(&self, params: &[FfiType]) -> Result<Vec<BasicMetadataTypeEnum<'ctx>>> {
        params
            .iter()
            .map(|ty| self.ffi_type_to_basic(ty).map(Into::into))
            .collect()
    }

    fn ffi_type_to_basic(&self, ty: &FfiType) -> Result<BasicTypeEnum<'ctx>> {
        match ty {
            FfiType::Unit => bail!("unit type is not allowed in FFI parameter position"),
            FfiType::Bool => Ok(self.context.bool_type().into()),
            FfiType::I32 => Ok(self.context.i32_type().into()),
            FfiType::I64 => Ok(self.context.i64_type().into()),
            FfiType::F64 => Ok(self.context.f64_type().into()),
            FfiType::Str => Ok(self.string_ptr_type.into()),
            FfiType::Opaque => Ok(self.context.i64_type().into()),
        }
    }

    fn eval_fstring(
        &mut self,
        parts: &[crate::ast::nodes::FStringPart],
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        use crate::ast::nodes::FStringPart;

        // Declare/get helper functions
        let format_float_fn = self
            .declare_or_get_format_function("otter_format_float", self.context.f64_type().into());
        let format_int_fn =
            self.declare_or_get_format_function("otter_format_int", self.context.i64_type().into());
        let format_bool_fn = self
            .declare_or_get_format_function("otter_format_bool", self.context.bool_type().into());
        let concat_fn = self.declare_or_get_concat_function();
        let free_fn = self.declare_or_get_free_function();

        // Start with empty string
        let mut result_ptr = self
            .builder
            .build_global_string_ptr("", "empty_str")
            .as_pointer_value();
        let mut need_free_result = false;

        for (idx, part) in parts.iter().enumerate() {
            match part {
                FStringPart::Text(text) => {
                    let text_ptr = self
                        .builder
                        .build_global_string_ptr(text, &format!("fstr_text_{}", idx))
                        .as_pointer_value();
                    let new_result = self
                        .builder
                        .build_call(
                            concat_fn,
                            &[result_ptr.into(), text_ptr.into()],
                            &format!("concat_{}", idx),
                        )
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    // Free the previous result if it was dynamically allocated
                    if need_free_result {
                        self.builder
                            .build_call(free_fn, &[result_ptr.into()], "free_old");
                    }

                    result_ptr = new_result;
                    need_free_result = true;
                }
                FStringPart::Expr(expr) => {
                    let evaluated = self.eval_expr(expr, ctx)?;

                    // Format the value based on its type
                    let formatted_ptr = match evaluated.ty {
                        OtterType::F64 => {
                            let float_val = evaluated
                                .value
                                .ok_or_else(|| anyhow!("f-string expression missing value"))?
                                .into_float_value();
                            self.builder
                                .build_call(
                                    format_float_fn,
                                    &[float_val.into()],
                                    &format!("format_float_{}", idx),
                                )
                                .try_as_basic_value()
                                .left()
                                .unwrap()
                                .into_pointer_value()
                        }
                        OtterType::I64 | OtterType::I32 => {
                            let int_val = evaluated
                                .value
                                .ok_or_else(|| anyhow!("f-string expression missing value"))?
                                .into_int_value();
                            // Convert to i64 if needed
                            let int64_val = if evaluated.ty == OtterType::I32 {
                                self.builder.build_int_s_extend(
                                    int_val,
                                    self.context.i64_type(),
                                    "extend_to_i64",
                                )
                            } else {
                                int_val
                            };
                            self.builder
                                .build_call(
                                    format_int_fn,
                                    &[int64_val.into()],
                                    &format!("format_int_{}", idx),
                                )
                                .try_as_basic_value()
                                .left()
                                .unwrap()
                                .into_pointer_value()
                        }
                        OtterType::Bool => {
                            let bool_val = evaluated
                                .value
                                .ok_or_else(|| anyhow!("f-string expression missing value"))?
                                .into_int_value();
                            self.builder
                                .build_call(
                                    format_bool_fn,
                                    &[bool_val.into()],
                                    &format!("format_bool_{}", idx),
                                )
                                .try_as_basic_value()
                                .left()
                                .unwrap()
                                .into_pointer_value()
                        }
                        OtterType::Str => evaluated
                            .value
                            .ok_or_else(|| anyhow!("f-string expression missing value"))?
                            .into_pointer_value(),
                        _ => bail!("unsupported type in f-string: {:?}", evaluated.ty),
                    };

                    // Concatenate with result
                    let new_result = self
                        .builder
                        .build_call(
                            concat_fn,
                            &[result_ptr.into(), formatted_ptr.into()],
                            &format!("concat_expr_{}", idx),
                        )
                        .try_as_basic_value()
                        .left()
                        .unwrap()
                        .into_pointer_value();

                    // Free the previous result if it was dynamically allocated
                    if need_free_result {
                        self.builder
                            .build_call(free_fn, &[result_ptr.into()], "free_old");
                    }

                    // Free the formatted string (always dynamically allocated)
                    if !matches!(evaluated.ty, OtterType::Str) {
                        self.builder
                            .build_call(free_fn, &[formatted_ptr.into()], "free_formatted");
                    }

                    result_ptr = new_result;
                    need_free_result = true;
                }
            }
        }

        Ok(EvaluatedValue::with_value(
            result_ptr.into(),
            OtterType::Str,
        ))
    }

    fn declare_or_get_format_function(
        &mut self,
        name: &str,
        param_type: BasicMetadataTypeEnum<'ctx>,
    ) -> FunctionValue<'ctx> {
        if let Some(f) = self.module.get_function(name) {
            return f;
        }
        let fn_type = self.string_ptr_type.fn_type(&[param_type], false);
        self.module.add_function(name, fn_type, None)
    }

    fn declare_or_get_concat_function(&mut self) -> FunctionValue<'ctx> {
        if let Some(f) = self.module.get_function("otter_concat_strings") {
            return f;
        }
        let fn_type = self.string_ptr_type.fn_type(
            &[self.string_ptr_type.into(), self.string_ptr_type.into()],
            false,
        );
        self.module
            .add_function("otter_concat_strings", fn_type, None)
    }

    fn declare_or_get_free_function(&mut self) -> FunctionValue<'ctx> {
        if let Some(f) = self.module.get_function("otter_free_string") {
            return f;
        }
        let fn_type = self
            .context
            .void_type()
            .fn_type(&[self.string_ptr_type.into()], false);
        self.module.add_function("otter_free_string", fn_type, None)
    }

    fn lower_while_loop(
        &mut self,
        function: FunctionValue<'ctx>,
        ctx: &mut FunctionContext<'ctx>,
        cond: &Expr,
        body: &Block,
    ) -> Result<()> {
        // Create basic blocks for the loop
        let cond_bb = self.context.append_basic_block(function, "while_cond");
        let body_bb = self.context.append_basic_block(function, "while_body");
        let exit_bb = self.context.append_basic_block(function, "while_exit");

        // Push loop context for break/continue
        ctx.push_loop(cond_bb, exit_bb);

        // Jump to condition check
        self.builder.build_unconditional_branch(cond_bb);

        // Generate condition block
        self.builder.position_at_end(cond_bb);
        let cond_value = self.eval_expr(cond, ctx)?;
        if cond_value.ty != OtterType::Bool {
            bail!("while condition must be a boolean, got {:?}", cond_value.ty);
        }

        let cond_bool = cond_value
            .value
            .ok_or_else(|| anyhow!("missing while condition value"))?
            .into_int_value();

        self.builder
            .build_conditional_branch(cond_bool, body_bb, exit_bb);

        // Generate loop body
        self.builder.position_at_end(body_bb);
        for stmt in &body.statements {
            self.lower_statement(stmt, function, ctx)?;
        }

        // Jump back to condition check if no terminator
        if self
            .builder
            .get_insert_block()
            .and_then(|b| b.get_terminator())
            .is_none()
        {
            self.builder.build_unconditional_branch(cond_bb);
        }

        // Pop loop context
        ctx.pop_loop();

        // Continue after loop
        self.builder.position_at_end(exit_bb);
        Ok(())
    }

    fn lower_if_statement(
        &mut self,
        function: FunctionValue<'ctx>,
        ctx: &mut FunctionContext<'ctx>,
        cond: &Expr,
        then_block: &Block,
        elif_blocks: &[(Expr, Block)],
        else_block: Option<&Block>,
    ) -> Result<()> {
        // Evaluate the main condition
        let cond_value = self.eval_expr(cond, ctx)?;
        if cond_value.ty != OtterType::Bool {
            bail!("if condition must be a boolean, got {:?}", cond_value.ty);
        }

        let cond_bool = cond_value
            .value
            .ok_or_else(|| anyhow!("missing condition value"))?
            .into_int_value();

        // Create basic blocks
        let then_bb = self.context.append_basic_block(function, "then");
        let merge_bb = self.context.append_basic_block(function, "ifcont");

        // Handle elif chains
        let mut current_else_bb = self.context.append_basic_block(function, "else_start");
        self.builder
            .build_conditional_branch(cond_bool, then_bb, current_else_bb);

        // Generate then block
        self.builder.position_at_end(then_bb);
        for stmt in &then_block.statements {
            self.lower_statement(stmt, function, ctx)?;
        }
        if self
            .builder
            .get_insert_block()
            .and_then(|b| b.get_terminator())
            .is_none()
        {
            self.builder.build_unconditional_branch(merge_bb);
        }

        // Generate elif/else chain
        let mut current_elif_idx = 0;

        // Process elif blocks
        for (elif_cond, elif_block) in elif_blocks {
            let elif_cond_bb = current_else_bb;
            let elif_then_bb = self
                .context
                .append_basic_block(function, &format!("elif_then_{}", current_elif_idx));
            let next_bb = self
                .context
                .append_basic_block(function, &format!("elif_next_{}", current_elif_idx));

            self.builder.position_at_end(elif_cond_bb);

            let elif_cond_value = self.eval_expr(elif_cond, ctx)?;
            if elif_cond_value.ty != OtterType::Bool {
                bail!(
                    "elif condition must be a boolean, got {:?}",
                    elif_cond_value.ty
                );
            }

            let elif_cond_bool = elif_cond_value
                .value
                .ok_or_else(|| anyhow!("missing elif condition value"))?
                .into_int_value();

            self.builder
                .build_conditional_branch(elif_cond_bool, elif_then_bb, next_bb);

            // Generate elif then block
            self.builder.position_at_end(elif_then_bb);
            for stmt in &elif_block.statements {
                self.lower_statement(stmt, function, ctx)?;
            }
            if self
                .builder
                .get_insert_block()
                .and_then(|b| b.get_terminator())
                .is_none()
            {
                self.builder.build_unconditional_branch(merge_bb);
            }

            current_else_bb = next_bb;
            current_elif_idx += 1;
        }

        // Generate else block
        self.builder.position_at_end(current_else_bb);
        if let Some(else_block) = else_block {
            for stmt in &else_block.statements {
                self.lower_statement(stmt, function, ctx)?;
            }
        }
        if self
            .builder
            .get_insert_block()
            .and_then(|b| b.get_terminator())
            .is_none()
        {
            self.builder.build_unconditional_branch(merge_bb);
        }

        // Continue after if
        self.builder.position_at_end(merge_bb);
        Ok(())
    }

    fn run_default_passes(&self, level: CodegenOptLevel, enable_pgo: bool, pgo_profile_file: Option<&Path>, inline_threshold: Option<u32>) {
        if matches!(level, CodegenOptLevel::None) {
            return;
        }

        // Set inline threshold if specified
        if let Some(_threshold) = inline_threshold {
            // Set module-level inline threshold attribute
            // Note: inkwell doesn't expose direct threshold setting, so we'll configure via pass manager
            // The inlining pass will respect optimization level
        }

        let pass_manager = PassManager::create(());

        // PGO instrumentation pass (if enabled and no profile file provided)
        if enable_pgo && pgo_profile_file.is_none() {
            // Add instrumentation pass for profile generation
            // Note: inkwell doesn't directly expose PGO instrumentation passes
            // In a full implementation, you would use LLVM's instrprof pass
        }

        // Early optimization passes - run these first for better analysis
        pass_manager.add_type_based_alias_analysis_pass();
        pass_manager.add_scoped_no_alias_aa_pass();
        pass_manager.add_basic_alias_analysis_pass();

        // Memory and data flow optimizations
        pass_manager.add_memcpy_optimize_pass();
        pass_manager.add_dead_store_elimination_pass();

        // Arithmetic and instruction optimizations
        pass_manager.add_instruction_combining_pass();
        pass_manager.add_reassociate_pass();
        pass_manager.add_gvn_pass();
        pass_manager.add_cfg_simplification_pass();
        pass_manager.add_instruction_simplify_pass();

        // Function-level optimizations with enhanced inlining
        // Configure inlining based on threshold and optimization level
        pass_manager.add_function_inlining_pass();
        // Note: inkwell's API doesn't expose threshold configuration directly
        // In practice, the optimization level influences inlining aggressiveness
        
        // If aggressive optimization and no threshold specified, use more aggressive inlining
        if matches!(level, CodegenOptLevel::Aggressive) && inline_threshold.is_none() {
            // Aggressive inlining is already handled by optimization level
        }

        pass_manager.add_constant_merge_pass();
        pass_manager.add_merge_functions_pass();

        // Control flow optimizations
        pass_manager.add_tail_call_elimination_pass();

        // Advanced analysis-based optimizations
        pass_manager.add_sccp_pass(); // Sparse Conditional Constant Propagation
        pass_manager.add_aggressive_dce_pass(); // Aggressive Dead Code Elimination
        pass_manager.add_licm_pass(); // Loop-Invariant Code Motion
        pass_manager.add_loop_deletion_pass();

        if matches!(level, CodegenOptLevel::Default) {
            // Additional optimizations for default level
            pass_manager.add_promote_memory_to_register_pass();
            pass_manager.add_dead_store_elimination_pass();
        }

        if matches!(level, CodegenOptLevel::Aggressive) {
            // Most aggressive optimizations for release mode
            pass_manager.add_loop_unroll_pass();
            pass_manager.add_loop_vectorize_pass();
            pass_manager.add_slp_vectorize_pass(); // Superword-Level Parallelism

            // Inter-procedural optimizations
            pass_manager.add_ipsccp_pass();
            pass_manager.add_global_optimizer_pass();
        }

        // PGO-based optimizations (if profile file provided)
        if enable_pgo && pgo_profile_file.is_some() {
            // Use profile data for optimization
            // Note: inkwell doesn't directly expose PGO passes
            // In a full implementation, you would use LLVM's pgo-instr-use pass
            // For now, we rely on the optimization level which respects profile data
            // when available at link time
        }

        pass_manager.run_on(&self.module);
    }
}
