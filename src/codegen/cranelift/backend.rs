use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use cranelift_codegen::ir::InstBuilder;
use cranelift_codegen::{Context, ir, isa, settings};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};
use target_lexicon::Triple;

use crate::codegen::target::TargetTriple;
use crate::runtime::symbol_registry::SymbolRegistry;
use crate::typecheck::TypeInfo;
use ast::nodes::Function;

/// Backend-agnostic codegen trait
pub trait CodegenBackend {
    /// Declare a function with the given signature
    fn declare_function(&mut self, function: &Function) -> Result<FuncId>;

    /// Build the body of a declared function
    fn build_function(&mut self, func_id: FuncId, function: &Function) -> Result<()>;

    /// Finalize the module and return compiled artifacts
    fn finalize_module(&mut self) -> Result<CompiledModule>;

    /// Get information about the target platform
    fn get_target_info(&self) -> TargetInfo;

    /// Emit an object file to the given path
    fn emit_object_file(&self, path: &Path) -> Result<()>;

    /// Create a JIT context for runtime execution
    fn create_jit_context(&self) -> Result<JitContext>;
}

/// Information about the compilation target
#[derive(Debug, Clone)]
pub struct TargetInfo {
    pub triple: Triple,
    pub pointer_width: u32,
    pub endianness: target_lexicon::Endianness,
}

/// Compiled module containing functions and data
#[derive(Debug)]
pub struct CompiledModule {
    pub functions: HashMap<String, FuncId>,
    pub data_objects: HashMap<String, DataId>,
    pub object_bytes: Option<Vec<u8>>,
}

/// JIT context for runtime execution
#[derive(Debug)]
pub struct JitContext {
    /// The JIT module for dynamic code execution
    pub module: JITModule,
    /// Cache of declared functions in the JIT module
    pub functions: HashMap<String, FuncId>,
}

/// Cranelift-based codegen backend
pub struct CraneliftBackend {
    /// The Cranelift module for managing functions and data
    module: ObjectModule,
    /// Function builder context for reusing allocations
    fn_builder_ctx: FunctionBuilderContext,
    /// Target ISA for code generation
    isa: std::sync::Arc<dyn isa::TargetIsa>,
    /// Symbol registry for FFI functions
    _symbol_registry: &'static SymbolRegistry,
    /// Type information from type checking
    _expr_types: HashMap<usize, TypeInfo>,
    /// Cache of declared functions
    declared_functions: HashMap<String, FuncId>,
    /// Cache of declared data objects
    declared_data: HashMap<String, DataId>,
}

impl CraneliftBackend {
    /// Create a new Cranelift backend with the given configuration
    pub fn new(
        target: &TargetTriple,
        symbol_registry: &'static SymbolRegistry,
        expr_types: HashMap<usize, TypeInfo>,
    ) -> Result<Self> {
        // Convert our TargetTriple to target-lexicon Triple
        let triple = Self::target_triple_to_lexicon(target)?;

        // Create ISA builder for the target
        let isa = isa::lookup(triple.clone())
            .map_err(|e| anyhow!("Failed to create ISA for target {}: {}", triple, e))?
            .finish(settings::Flags::new(settings::builder()))
            .map_err(|e| anyhow!("Failed to finish ISA setup: {}", e))?;

        // Create object module
        let module = ObjectModule::new(
            ObjectBuilder::new(
                isa.clone(),
                "otterlang".to_string(),
                cranelift_module::default_libcall_names(),
            )
            .map_err(|e| anyhow!("Failed to create object builder: {}", e))?,
        );

        Ok(Self {
            module,
            fn_builder_ctx: FunctionBuilderContext::new(),
            isa,
            _symbol_registry: symbol_registry,
            _expr_types: expr_types,
            declared_functions: HashMap::new(),
            declared_data: HashMap::new(),
        })
    }

    /// Convert our TargetTriple to target-lexicon Triple
    fn target_triple_to_lexicon(target: &TargetTriple) -> Result<Triple> {
        let triple_str = format!("{}-{}-{}", target.arch, target.vendor, target.os);
        triple_str
            .parse()
            .map_err(|e| anyhow!("Invalid target triple {}: {}", triple_str, e))
    }
}
impl CodegenBackend for CraneliftBackend {
    fn declare_function(&mut self, function: &Function) -> Result<FuncId> {
        let sig = self.build_signature(function)?;
        let func_id = self
            .module
            .declare_function(&function.name, Linkage::Export, &sig)
            .map_err(|e| anyhow!("Failed to declare function {}: {}", function.name, e))?;

        self.declared_functions
            .insert(function.name.clone(), func_id);
        Ok(func_id)
    }

    fn build_function(&mut self, func_id: FuncId, function: &Function) -> Result<()> {
        // Create a new function context
        let mut ctx = Context::new();
        ctx.func.signature = self.build_signature(function)?;

        // Take the function builder context temporarily
        let mut fn_builder_ctx = std::mem::take(&mut self.fn_builder_ctx);

        // Create the function builder
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_builder_ctx);

        // Create entry block
        let entry_block = builder.create_block();
        builder.switch_to_block(entry_block);

        // Set up variables for function parameters
        let mut variables = HashMap::new();

        // Declare function parameters as variables
        for (i, param) in function.params.iter().enumerate() {
            let param_value = builder.block_params(entry_block)[i];
            variables.insert(param.name.clone(), param_value);
        }

        // Seal the entry block before lowering (required by Cranelift)
        builder.seal_block(entry_block);

        // Lower the function body
        let result =
            Self::lower_block_with_builder(self, &mut builder, &function.body, &mut variables)?;

        // Return the result of the last expression in the block
        builder.ins().return_(&[result]);

        // Finalize the function
        builder.finalize();

        // Restore the function builder context
        self.fn_builder_ctx = fn_builder_ctx;

        // Define the function in the module
        self.module.define_function(func_id, &mut ctx)?;

        Ok(())
    }

    fn finalize_module(&mut self) -> Result<CompiledModule> {
        // Finalize the ObjectModule to get the object bytes
        let product = self.module.finish();
        let object_bytes = product
            .object
            .write()
            .map_err(|e| anyhow!("Failed to write object: {}", e))?;

        Ok(CompiledModule {
            functions: self.declared_functions.clone(),
            data_objects: self.declared_data.clone(),
            object_bytes: Some(object_bytes),
        })
    }

    fn get_target_info(&self) -> TargetInfo {
        let triple = self.isa.triple().clone();
        let pointer_width = self.isa.pointer_bits() as u32;
        let endianness = match self.isa.endianness() {
            ir::Endianness::Little => target_lexicon::Endianness::Little,
            ir::Endianness::Big => target_lexicon::Endianness::Big,
        };

        TargetInfo {
            triple,
            pointer_width,
            endianness,
        }
    }

    fn emit_object_file(&self, path: &Path) -> Result<()> {
        // First finalize the module to get object bytes
        let product = self.module.finish();
        let object_bytes = product
            .object
            .write()
            .map_err(|e| anyhow!("Failed to write object: {}", e))?;

        // Write the object bytes to the file
        std::fs::write(path, object_bytes)
            .map_err(|e| anyhow!("Failed to write object file to {}: {}", path.display(), e))?;

        Ok(())
    }

    fn create_jit_context(&self) -> Result<JitContext> {
        // Create JIT builder for the target
        let jit_builder = JITBuilder::new(cranelift_module::default_libcall_names())
            .map_err(|e| anyhow!("Failed to create JIT builder: {}", e))?;

        // Create JIT module
        let jit_module = JITModule::new(jit_builder);

        // Copy over declared functions from the object module
        let mut functions = HashMap::new();
        for (name, &func_id) in &self.declared_functions {
            let decl = self.module.declarations().get_function(func_id);

            // Re-declare the function in the JIT module using the same linkage/signature
            let jit_func_id = jit_module
                .declare_function(name, decl.linkage, &decl.signature)
                .map_err(|e| anyhow!("Failed to declare JIT function {}: {}", name, e))?;

            functions.insert(name.clone(), jit_func_id);
        }

        Ok(JitContext {
            module: jit_module,
            functions,
        })
    }
}
