use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicUsize;

use anyhow::{Result, anyhow};
use inkwell::builder::Builder;
use inkwell::context::Context as InkwellContext;
use inkwell::module::Module;
use inkwell::passes::{PassBuilderOptions, PassManager};
use inkwell::targets::TargetMachine;
use inkwell::types::{BasicType, BasicTypeEnum, PointerType};
use inkwell::values::FunctionValue;

use crate::codegen::llvm::bridges::prepare_rust_bridges;
use crate::runtime::symbol_registry::SymbolRegistry;
use crate::typecheck::TypeInfo;
use ast::nodes::{Expr, Program, Statement};

pub mod expr;
pub mod stmt;
pub mod types;

use self::types::{FunctionContext, OtterType};

pub struct Compiler<'ctx> {
    pub(crate) context: &'ctx InkwellContext,
    pub(crate) builder: Builder<'ctx>,
    pub(crate) module: Module<'ctx>,
    #[allow(dead_code)]
    pub(crate) fpm: PassManager<FunctionValue<'ctx>>,
    pub(crate) symbol_registry: &'static SymbolRegistry,
    pub(crate) string_ptr_type: PointerType<'ctx>,
    pub(crate) declared_functions: HashMap<String, FunctionValue<'ctx>>,
    #[allow(dead_code)]
    pub(crate) expr_types: HashMap<usize, TypeInfo>,
    pub(crate) function_defaults: HashMap<String, Vec<Option<Expr>>>,
    #[allow(dead_code)]
    pub(crate) lambda_counter: AtomicUsize,
    pub cached_ir: Option<String>,
}

use crate::codegen::llvm::config::CodegenOptLevel;

impl<'ctx> Compiler<'ctx> {
    pub fn new(
        context: &'ctx InkwellContext,
        module: Module<'ctx>,
        builder: Builder<'ctx>,
        symbol_registry: &'static SymbolRegistry,
        expr_types: HashMap<usize, TypeInfo>,
    ) -> Self {
        let fpm = PassManager::create(&module);

        // fpm.add_instruction_combining_pass();
        // fpm.add_reassociate_pass();
        // fpm.add_gvn_pass();
        // fpm.add_cfg_simplification_pass();
        // fpm.add_basic_alias_analysis_pass();
        // fpm.add_promote_memory_to_register_pass();
        // fpm.initialize();

        let string_ptr_type = context.ptr_type(inkwell::AddressSpace::default());

        Self {
            context,
            builder,
            module,
            fpm,
            symbol_registry,
            string_ptr_type,
            declared_functions: HashMap::new(),
            expr_types,
            function_defaults: HashMap::new(),
            lambda_counter: AtomicUsize::new(0),
            cached_ir: None,
        }
    }

    pub fn lower_program(&mut self, program: &Program, _require_main: bool) -> Result<()> {
        self.compile_module(program)
    }

    pub fn compile_module(&mut self, program: &Program) -> Result<()> {
        // Prepare Rust bridges
        let _libraries = prepare_rust_bridges(program, self.symbol_registry)?;

        // First pass: register all functions and types
        for statement in &program.statements {
            match statement.as_ref() {
                Statement::Function(func) => {
                    self.register_function_prototype(func.as_ref())?;
                }
                Statement::Struct { .. } => {
                    // TODO: Register struct types
                }
                _ => {}
            }
        }

        // Second pass: compile function bodies
        for statement in &program.statements {
            if let Statement::Function(func) = statement.as_ref() {
                self.compile_function(func.as_ref())?;
            }
        }

        // Verify module
        if let Err(e) = self.module.verify() {
            self.module.print_to_stderr();
            return Err(anyhow!("Module verification failed: {}", e));
        }

        Ok(())
    }

    fn register_function_prototype(&mut self, func: &ast::nodes::Function) -> Result<()> {
        let ret_type: Option<BasicTypeEnum> = if let Some(_ret_ty) = &func.ret_ty {
            // TODO: Map AST type to LLVM type
            // For now, assume i64 for everything except unit
            Some(self.context.i64_type().into())
        } else {
            None
        };

        let mut param_types = Vec::new();
        for _param in &func.params {
            // TODO: Map param types
            param_types.push(self.context.i64_type().into());
        }

        let fn_type = if let Some(rt) = ret_type {
            rt.fn_type(&param_types, false)
        } else {
            self.context.void_type().fn_type(&param_types, false)
        };

        let function = self.module.add_function(&func.name, fn_type, None);
        self.declared_functions.insert(func.name.clone(), function);

        // Store default values
        let defaults: Vec<Option<Expr>> = func
            .params
            .iter()
            .map(|p| p.as_ref().default.as_ref().map(|e| e.as_ref().clone()))
            .collect();
        self.function_defaults.insert(func.name.clone(), defaults);

        Ok(())
    }

    fn compile_function(&mut self, func: &ast::nodes::Function) -> Result<()> {
        let function = self
            .declared_functions
            .get(&func.name)
            .ok_or_else(|| anyhow!("Function {} not found", func.name))?;

        let entry = self.context.append_basic_block(*function, "entry");
        self.builder.position_at_end(entry);

        let mut ctx = FunctionContext::new();

        // Bind arguments
        for (i, param) in func.params.iter().enumerate() {
            let arg_val = function.get_nth_param(i as u32).unwrap();
            let param_name = &param.as_ref().name;

            // Allocate stack space for parameter
            let alloca = self
                .builder
                .build_alloca(self.context.i64_type(), param_name.as_ref())?;
            self.builder.build_store(alloca, arg_val)?;

            ctx.insert(
                param_name.as_ref().to_string(),
                crate::codegen::llvm::compiler::types::Variable {
                    ptr: alloca,
                    ty: OtterType::I64, // TODO: Use real type
                },
            );
        }

        // Compile body
        self.lower_block(func.body.as_ref(), *function, &mut ctx)?;

        // Add implicit return if needed
        if self
            .builder
            .get_insert_block()
            .and_then(|b| b.get_terminator())
            .is_none()
        {
            if func.ret_ty.is_none() {
                self.builder.build_return(None)?;
            } else {
                // Return 0/unit for now
                self.builder
                    .build_return(Some(&self.context.i64_type().const_int(0, false)))?;
            }
        }

        Ok(())
    }

    pub(super) fn run_default_passes(
        &self,
        level: CodegenOptLevel,
        _enable_pgo: bool,
        _pgo_profile_file: Option<&Path>,
        _inline_threshold: Option<u32>,
        target_machine: &TargetMachine,
    ) {
        if matches!(level, CodegenOptLevel::None) {
            return;
        }

        // Simplified pass running for now
        let pass_options = PassBuilderOptions::create();
        pass_options.set_loop_interleaving(true);
        pass_options.set_loop_vectorization(true);

        let _ = self
            .module
            .run_passes("default<O2>", target_machine, pass_options);
    }
}
