use anyhow::{anyhow, Context, Result};
use inkwell::context::Context as LlvmContext;
use libloading::{Library, Symbol};
use std::collections::HashMap;
use std::ffi::CString;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

use crate::ast::nodes::Program;
use crate::codegen::{build_shared_library, CodegenOptLevel, CodegenOptions};
use crate::runtime::symbol_registry::SymbolRegistry;
use crate::typecheck::TypeChecker;

use super::adaptive::{AdaptiveConcurrencyManager, AdaptiveMemoryManager};
use super::cache::FunctionCache;
use super::optimization::{CallGraph, Inliner, Reoptimizer};
use super::profiler::GlobalProfiler;
use super::specialization::{Specializer, TypeTracker};

/// Function pointer type for different signatures
#[allow(non_camel_case_types)]
pub enum FunctionPtr {
    NoArgs(unsafe extern "C" fn() -> u64),
    OneArg(unsafe extern "C" fn(u64) -> u64),
    TwoArgs(unsafe extern "C" fn(u64, u64) -> u64),
    ThreeArgs(unsafe extern "C" fn(u64, u64, u64) -> u64),
    VarArgs(unsafe extern "C" fn(*const u64, usize) -> u64),
}

/// Compiled function with metadata
struct CompiledFunction {
    library: Arc<Library>,
    function_ptr: FunctionPtr,
    arg_count: usize,
}

impl CompiledFunction {
    fn execute(&self, args: &[u64]) -> Result<u64> {
        if args.len() != self.arg_count {
            return Err(anyhow!(
                "Argument count mismatch: expected {}, got {}",
                self.arg_count,
                args.len()
            ));
        }

        let result = unsafe {
            match (&self.function_ptr, args.len()) {
                (FunctionPtr::NoArgs(f), 0) => f(),
                (FunctionPtr::OneArg(f), 1) => f(args[0]),
                (FunctionPtr::TwoArgs(f), 2) => f(args[0], args[1]),
                (FunctionPtr::ThreeArgs(f), 3) => f(args[0], args[1], args[2]),
                (FunctionPtr::VarArgs(f), n) => f(args.as_ptr(), n),
                _ => {
                    return Err(anyhow!(
                        "Function signature mismatch: expected {} args, got {}",
                        self.arg_count,
                        args.len()
                    ));
                }
            }
        };

        Ok(result)
    }
}

/// JIT execution engine that compiles programs and executes functions dynamically
pub struct JitEngine {
    #[allow(dead_code)]
    context: LlvmContext,
    profiler: GlobalProfiler,
    #[allow(dead_code)]
    specializer: Specializer,
    #[allow(dead_code)]
    type_tracker: TypeTracker,
    #[allow(dead_code)]
    function_cache: FunctionCache,
    #[allow(dead_code)]
    inliner: Inliner,
    #[allow(dead_code)]
    reoptimizer: Reoptimizer,
    #[allow(dead_code)]
    memory_manager: AdaptiveMemoryManager,
    concurrency_manager: AdaptiveConcurrencyManager,
    #[allow(dead_code)]
    symbol_registry: &'static SymbolRegistry,
    // Runtime state
    compiled_library: Arc<Mutex<Option<Arc<Library>>>>,
    compiled_functions: Arc<Mutex<HashMap<String, CompiledFunction>>>,
    temp_dir: TempDir,
    program: Option<Program>,
    #[allow(dead_code)]
    library_path: Arc<Mutex<Option<std::path::PathBuf>>>,
}

impl JitEngine {
    pub fn new(symbol_registry: &'static SymbolRegistry) -> Result<Self> {
        let temp_dir =
            TempDir::new().map_err(|e| anyhow!("Failed to create temp directory: {}", e))?;

        Ok(Self {
            context: LlvmContext::create(),
            profiler: GlobalProfiler::new(),
            specializer: Specializer::new(),
            type_tracker: TypeTracker::new(),
            function_cache: FunctionCache::new(256 * 1024 * 1024), // 256MB cache
            inliner: Inliner::new(),
            reoptimizer: Reoptimizer::new(),
            memory_manager: AdaptiveMemoryManager::new(),
            concurrency_manager: AdaptiveConcurrencyManager::new(),
            symbol_registry,
            compiled_library: Arc::new(Mutex::new(None)),
            compiled_functions: Arc::new(Mutex::new(HashMap::new())),
            temp_dir,
            program: None,
            library_path: Arc::new(Mutex::new(None)),
        })
    }

    /// Compile a program for JIT execution
    pub fn compile_program(&mut self, program: &Program) -> Result<()> {
        // Initialize concurrency manager
        self.concurrency_manager
            .initialize_thread_pool()
            .map_err(|e| anyhow!("Failed to initialize thread pool: {}", e))?;

        // Analyze call graph for optimization
        let mut call_graph = CallGraph::new();
        call_graph.analyze_program(program);

        // Store program
        self.program = Some(program.clone());

        // Compile to shared library
        let lib_path = self.temp_dir.path().join("jit_program");
        let options = CodegenOptions {
            target: None,
            emit_ir: false,
            opt_level: CodegenOptLevel::Default,
            enable_lto: false,
            enable_pgo: false,
            pgo_profile_file: None,
            inline_threshold: None,
        };

        let mut type_checker = TypeChecker::new().with_registry(SymbolRegistry::global());
        type_checker
            .check_program(program)
            .context("Type checking failed during JIT compilation")?;
        let expr_types = type_checker.into_expr_type_map();

        let artifact = build_shared_library(program, &expr_types, &lib_path, &options)
            .context("Failed to compile program to shared library")?;

        let lib_path = artifact.binary;

        // Load the shared library
        let library = unsafe {
            Library::new(&lib_path).map_err(|e| {
                anyhow!(
                    "Failed to load shared library {}: {}",
                    lib_path.display(),
                    e
                )
            })?
        };

        // Store library
        *self.compiled_library.lock().unwrap() = Some(Arc::new(library));
        *self.library_path.lock().unwrap() = Some(lib_path);

        // Extract function symbols from the program
        self.load_functions(program)?;

        Ok(())
    }

    /// Load all function symbols from the compiled library
    fn load_functions(&self, program: &Program) -> Result<()> {
        let library = self
            .compiled_library
            .lock()
            .unwrap()
            .as_ref()
            .ok_or_else(|| anyhow!("Library not loaded"))?
            .clone();

        let mut functions = self.compiled_functions.lock().unwrap();

        // Extract function definitions from program
        for stmt in &program.statements {
            if let crate::ast::nodes::Statement::Function(func) = stmt {
                let func_name = &func.name;
                let arg_count = func.params.len();

                // Try to load function with different signatures
                let func_ptr = self.load_function_symbol(&library, func_name, arg_count)?;

                functions.insert(
                    func_name.clone(),
                    CompiledFunction {
                        library: library.clone(),
                        function_ptr: func_ptr,
                        arg_count,
                    },
                );
            }
        }

        Ok(())
    }

    /// Load a function symbol from the library, trying different signatures
    fn load_function_symbol(
        &self,
        library: &Arc<Library>,
        name: &str,
        arg_count: usize,
    ) -> Result<FunctionPtr> {
        let name_cstr =
            CString::new(name).map_err(|e| anyhow!("Invalid function name '{}': {}", name, e))?;

        // Try different function signatures based on argument count
        unsafe {
            match arg_count {
                0 => {
                    let sym: Symbol<unsafe extern "C" fn() -> u64> = library
                        .get(name_cstr.as_bytes())
                        .map_err(|e| anyhow!("Failed to load function '{}': {}", name, e))?;
                    Ok(FunctionPtr::NoArgs(*sym))
                }
                1 => {
                    let sym: Symbol<unsafe extern "C" fn(u64) -> u64> = library
                        .get(name_cstr.as_bytes())
                        .map_err(|e| anyhow!("Failed to load function '{}': {}", name, e))?;
                    Ok(FunctionPtr::OneArg(*sym))
                }
                2 => {
                    let sym: Symbol<unsafe extern "C" fn(u64, u64) -> u64> = library
                        .get(name_cstr.as_bytes())
                        .map_err(|e| anyhow!("Failed to load function '{}': {}", name, e))?;
                    Ok(FunctionPtr::TwoArgs(*sym))
                }
                3 => {
                    let sym: Symbol<unsafe extern "C" fn(u64, u64, u64) -> u64> = library
                        .get(name_cstr.as_bytes())
                        .map_err(|e| anyhow!("Failed to load function '{}': {}", name, e))?;
                    Ok(FunctionPtr::ThreeArgs(*sym))
                }
                _ => {
                    // For functions with more than 3 args, use varargs
                    let sym: Symbol<unsafe extern "C" fn(*const u64, usize) -> u64> = library
                        .get(name_cstr.as_bytes())
                        .map_err(|e| anyhow!("Failed to load function '{}': {}", name, e))?;
                    Ok(FunctionPtr::VarArgs(*sym))
                }
            }
        }
    }

    /// Execute a function via JIT
    pub fn execute_function(&mut self, function_name: &str, args: &[u64]) -> Result<u64> {
        let start = std::time::Instant::now();

        // Get compiled function
        let compiled_func = {
            let functions = self.compiled_functions.lock().unwrap();
            functions
                .get(function_name)
                .ok_or_else(|| anyhow!("Function '{}' not found or not compiled", function_name))?
                .clone()
        };

        // Execute the function
        let result = compiled_func.execute(args)?;

        // Record call for profiling
        let duration = start.elapsed();
        self.profiler.record_call(function_name, duration);

        // Check for hot functions periodically
        if self
            .profiler
            .get_metrics(function_name)
            .map(|m| m.call_count % 1000 == 0)
            .unwrap_or(false)
        {
            let hot_functions = self.profiler.check_hot_functions();
            if !hot_functions.is_empty() {
                self.optimize_hot_functions(&hot_functions)?;
            }
        }

        Ok(result)
    }

    /// Optimize hot functions by recompiling with aggressive optimizations
    fn optimize_hot_functions(
        &mut self,
        hot_functions: &[super::profiler::HotFunction],
    ) -> Result<()> {
        // Get the program
        let program = self
            .program
            .as_ref()
            .ok_or_else(|| anyhow!("No program loaded"))?;

        // Recompile with aggressive optimizations
        let lib_path = self.temp_dir.path().join("jit_program_optimized");
        let options = CodegenOptions {
            target: None,
            emit_ir: false,
            opt_level: CodegenOptLevel::Aggressive,
            enable_lto: true,
            enable_pgo: false,
            pgo_profile_file: None,
            inline_threshold: None,
        };

        let mut type_checker = TypeChecker::new().with_registry(SymbolRegistry::global());
        type_checker
            .check_program(program)
            .context("Type checking failed during optimized JIT compilation")?;
        let expr_types = type_checker.into_expr_type_map();

        let artifact = build_shared_library(program, &expr_types, &lib_path, &options)
            .context("Failed to recompile with optimizations")?;

        let lib_path = artifact.binary;

        // Load optimized library
        let library = Arc::new(unsafe {
            Library::new(&lib_path)
                .map_err(|e| anyhow!("Failed to load optimized library: {}", e))?
        });

        // Update library reference
        *self.compiled_library.lock().unwrap() = Some(library.clone());

        // Reload hot functions
        for hot_func in hot_functions {
            if let Some(func) = program.statements.iter().find_map(|stmt| match stmt {
                crate::ast::nodes::Statement::Function(f) if f.name == hot_func.name => Some(f),
                _ => None,
            }) {
                let arg_count = func.params.len();
                if let Ok(func_ptr) = self.load_function_symbol(&library, &hot_func.name, arg_count)
                {
                    let mut functions = self.compiled_functions.lock().unwrap();
                    functions.insert(
                        hot_func.name.clone(),
                        CompiledFunction {
                            library: library.clone(),
                            function_ptr: func_ptr,
                            arg_count,
                        },
                    );
                }
            }
        }

        Ok(())
    }

    /// Get profiler statistics
    pub fn get_profiler_stats(&self) -> Vec<super::profiler::FunctionMetrics> {
        self.profiler.get_all_metrics()
    }

    /// Get cache statistics
    pub fn get_cache_stats(&self) -> super::cache::function_cache::CacheStats {
        self.function_cache.stats()
    }

    /// Get list of compiled function names
    pub fn get_function_names(&self) -> Vec<String> {
        let functions = self.compiled_functions.lock().unwrap();
        functions.keys().cloned().collect()
    }
}

impl Clone for CompiledFunction {
    fn clone(&self) -> Self {
        Self {
            library: self.library.clone(),
            function_ptr: match &self.function_ptr {
                FunctionPtr::NoArgs(f) => FunctionPtr::NoArgs(*f),
                FunctionPtr::OneArg(f) => FunctionPtr::OneArg(*f),
                FunctionPtr::TwoArgs(f) => FunctionPtr::TwoArgs(*f),
                FunctionPtr::ThreeArgs(f) => FunctionPtr::ThreeArgs(*f),
                FunctionPtr::VarArgs(f) => FunctionPtr::VarArgs(*f),
            },
            arg_count: self.arg_count,
        }
    }
}
