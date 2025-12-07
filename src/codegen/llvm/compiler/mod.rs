use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicUsize;

use anyhow::{Result, anyhow};
use inkwell::builder::Builder;
use inkwell::context::Context as InkwellContext;
use inkwell::module::Module;
use inkwell::passes::{PassBuilderOptions, PassManager};
use inkwell::targets::TargetMachine;
use inkwell::types::{BasicType, BasicTypeEnum, PointerType, StructType};
use inkwell::values::{FunctionValue, PointerValue};

use crate::codegen::llvm::bridges::prepare_rust_bridges;
use crate::codegen::target::TargetTriple;
use crate::runtime::symbol_registry::SymbolRegistry;
use crate::typecheck::{EnumLayout, TypeInfo};
use otterc_ast::nodes::{Block, Expr, FStringPart, Function, Node, Program, Statement};
use otterc_span::Span;

pub mod expr;
pub mod stmt;
pub mod types;

use self::types::{FunctionContext, OtterType};

struct StructInfo<'ctx> {
    name: String,
    ty: StructType<'ctx>,
    field_indices: HashMap<String, usize>,
    field_types: Vec<OtterType>,
}

pub struct Compiler<'ctx> {
    pub(crate) context: &'ctx InkwellContext,
    pub(crate) builder: Builder<'ctx>,
    pub(crate) module: Module<'ctx>,
    #[expect(dead_code, reason = "Work in progress")]
    pub(crate) fpm: PassManager<FunctionValue<'ctx>>,
    pub(crate) symbol_registry: &'static SymbolRegistry,
    pub(crate) string_ptr_type: PointerType<'ctx>,
    pub(crate) declared_functions: HashMap<String, FunctionValue<'ctx>>,
    pub(crate) function_return_types: HashMap<String, OtterType>,
    pub(crate) expr_types: HashMap<usize, TypeInfo>,
    expr_types_by_span: HashMap<Span, TypeInfo>,
    pub(crate) comprehension_var_types: HashMap<Span, TypeInfo>,
    expr_spans: HashMap<usize, Span>,
    pub(crate) enum_layouts: HashMap<String, EnumLayout>,
    pub(crate) function_defaults: HashMap<String, Vec<Option<Expr>>>,
    #[expect(dead_code, reason = "Work in progress")]
    pub(crate) lambda_counter: AtomicUsize,
    next_spawn_id: u64,
    struct_ids: HashMap<String, u32>,
    struct_infos: Vec<StructInfo<'ctx>>,
    pub cached_ir: Option<String>,
    /// Target triple for platform-specific ABI handling
    target_triple: Option<TargetTriple>,
}

use crate::codegen::llvm::config::CodegenOptLevel;

impl<'ctx> Compiler<'ctx> {
    fn record_function_spans(&mut self, func: &Function) {
        self.record_block_spans(func.body.as_ref());
    }

    fn record_block_spans(&mut self, block: &Block) {
        for stmt in &block.statements {
            self.record_statement_spans(stmt.as_ref());
        }
    }

    fn record_statement_spans(&mut self, stmt: &Statement) {
        match stmt {
            Statement::Expr(expr)
            | Statement::Let { expr, .. }
            | Statement::Assignment { expr, .. }
            | Statement::Return(Some(expr)) => self.record_expr_spans(expr),
            Statement::Return(None)
            | Statement::Break
            | Statement::Continue
            | Statement::Pass
            | Statement::Use { .. }
            | Statement::PubUse { .. }
            | Statement::Enum { .. }
            | Statement::TypeAlias { .. } => {}
            Statement::Struct { methods, .. } => {
                for method in methods {
                    self.record_function_spans(method.as_ref());
                }
            }
            Statement::Function(func) => self.record_function_spans(func.as_ref()),
            Statement::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
            } => {
                self.record_expr_spans(cond);
                self.record_block_spans(then_block.as_ref());
                for (elif_cond, block) in elif_blocks {
                    self.record_expr_spans(elif_cond);
                    self.record_block_spans(block.as_ref());
                }
                if let Some(block) = else_block {
                    self.record_block_spans(block.as_ref());
                }
            }
            Statement::For { iterable, body, .. } => {
                self.record_expr_spans(iterable);
                self.record_block_spans(body.as_ref());
            }
            Statement::While { cond, body } => {
                self.record_expr_spans(cond);
                self.record_block_spans(body.as_ref());
            }
            Statement::Block(block) => self.record_block_spans(block.as_ref()),
        }
    }

    fn record_expr_spans(&mut self, expr: &Node<Expr>) {
        let ptr = expr.as_ref() as *const Expr as usize;
        self.expr_spans.insert(ptr, *expr.span());

        match expr.as_ref() {
            Expr::Literal(_) | Expr::Identifier(_) => {}
            Expr::Binary { left, right, .. } => {
                self.record_expr_spans(left);
                self.record_expr_spans(right);
            }
            Expr::Unary { expr, .. } | Expr::Await(expr) | Expr::Spawn(expr) => {
                self.record_expr_spans(expr);
            }
            Expr::Call { func, args } => {
                self.record_expr_spans(func);
                for arg in args {
                    self.record_expr_spans(arg);
                }
            }
            Expr::Member { object, .. } => self.record_expr_spans(object),
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.record_expr_spans(cond);
                self.record_expr_spans(then_branch);
                if let Some(expr) = else_branch {
                    self.record_expr_spans(expr);
                }
            }
            Expr::Match { value, arms } => {
                self.record_expr_spans(value);
                for arm in arms {
                    if let Some(guard) = &arm.as_ref().guard {
                        self.record_expr_spans(guard);
                    }
                    self.record_block_spans(arm.as_ref().body.as_ref());
                }
            }
            Expr::Range { start, end } => {
                self.record_expr_spans(start);
                self.record_expr_spans(end);
            }
            Expr::Array(elements) => {
                for elem in elements {
                    self.record_expr_spans(elem);
                }
            }
            Expr::Dict(pairs) => {
                for (key, value) in pairs {
                    self.record_expr_spans(key);
                    self.record_expr_spans(value);
                }
            }
            Expr::ListComprehension {
                element,
                iterable,
                condition,
                ..
            } => {
                self.record_expr_spans(element);
                self.record_expr_spans(iterable);
                if let Some(cond) = condition {
                    self.record_expr_spans(cond);
                }
            }
            Expr::DictComprehension {
                key,
                value,
                iterable,
                condition,
                ..
            } => {
                self.record_expr_spans(key);
                self.record_expr_spans(value);
                self.record_expr_spans(iterable);
                if let Some(cond) = condition {
                    self.record_expr_spans(cond);
                }
            }
            Expr::FString { parts } => {
                for part in parts {
                    if let FStringPart::Expr(expr) = part.as_ref() {
                        self.record_expr_spans(expr);
                    }
                }
            }
            Expr::Struct { fields, .. } => {
                for (_, value) in fields {
                    self.record_expr_spans(value);
                }
            }
        }
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "TODO: Create a struct to hold these args"
    )]
    pub fn new(
        context: &'ctx InkwellContext,
        module: Module<'ctx>,
        builder: Builder<'ctx>,
        symbol_registry: &'static SymbolRegistry,
        expr_types: HashMap<usize, TypeInfo>,
        expr_types_by_span: HashMap<Span, TypeInfo>,
        comprehension_var_types: HashMap<Span, TypeInfo>,
        enum_layouts: HashMap<String, EnumLayout>,
        target_triple: Option<TargetTriple>,
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
            function_return_types: HashMap::new(),
            expr_types,
            expr_types_by_span,
            comprehension_var_types,
            expr_spans: HashMap::new(),
            enum_layouts,
            function_defaults: HashMap::new(),
            lambda_counter: AtomicUsize::new(0),
            next_spawn_id: 0,
            struct_ids: HashMap::new(),
            struct_infos: Vec::new(),
            cached_ir: None,
            target_triple,
        }
    }

    /// Check if we're targeting Windows x64, which has different struct passing ABI
    fn is_windows_x64(&self) -> bool {
        self.target_triple
            .as_ref()
            .map(|t| t.is_windows() && t.arch == "x86_64")
            .unwrap_or(false)
    }

    pub fn lower_program(&mut self, program: &Program, _require_main: bool) -> Result<()> {
        self.compile_module(program)
    }

    pub(crate) fn expr_type(&self, expr: &Expr) -> Option<&TypeInfo> {
        let id = expr as *const Expr as usize;
        self.expr_types.get(&id).or_else(|| {
            self.expr_spans
                .get(&id)
                .and_then(|span| self.expr_types_by_span.get(span))
        })
    }

    pub(crate) fn enum_layout(&self, name: &str) -> Option<&EnumLayout> {
        self.enum_layouts.get(name)
    }

    fn ensure_struct_info(&mut self, name: &str) -> (u32, StructType<'ctx>) {
        if let Some(&id) = self.struct_ids.get(name) {
            let ty = self.struct_infos[id as usize].ty;
            return (id, ty);
        }

        let struct_type = self.context.opaque_struct_type(name);
        let id = self.struct_infos.len() as u32;
        self.struct_ids.insert(name.to_string(), id);
        self.struct_infos.push(StructInfo {
            name: name.to_string(),
            ty: struct_type,
            field_indices: HashMap::new(),
            field_types: Vec::new(),
        });
        (id, struct_type)
    }

    fn struct_info(&self, id: u32) -> &StructInfo<'ctx> {
        &self.struct_infos[id as usize]
    }

    fn struct_id(&self, name: &str) -> Option<u32> {
        self.struct_ids.get(name).copied()
    }

    fn struct_info_by_name(&self, name: &str) -> Option<(u32, &StructInfo<'ctx>)> {
        let id = self.struct_id(name)?;
        Some((id, self.struct_info(id)))
    }

    pub(crate) fn struct_type_from_expr(&self, expr: &Expr) -> Option<OtterType> {
        if let Some(TypeInfo::Struct { name, .. }) = self.expr_type(expr) {
            return self.struct_id(name).map(OtterType::Struct);
        }
        None
    }

    fn rewrite_method_self_param(
        &self,
        method_func: &mut otterc_ast::nodes::Function,
        struct_name: &str,
    ) {
        let Some(first_param) = method_func.params.first_mut() else {
            return;
        };

        // If the first parameter explicitly references `Self`, rewrite it to the
        // concrete struct name. This keeps the existing behavior but allows the
        // later logic to assume `self` always has a known type.
        if let Some(ty) = first_param.as_ref().ty.as_ref()
            && matches!(ty.as_ref(), otterc_ast::nodes::Type::Simple(name) if name == "Self")
        {
            let span = *ty.span();
            let replacement = otterc_ast::nodes::Type::Simple(struct_name.to_string());
            first_param.as_mut().ty = Some(Node::new(replacement, span));
            return;
        }

        // Methods commonly omit an explicit type on `self`. Ensure it is treated as the
        // enclosing struct so later codegen knows the correct OtterType instead of
        // defaulting to i64.
        let param_name_is_self = first_param.as_ref().name.as_ref() == "self";
        let has_type = first_param.as_ref().ty.is_some();
        if param_name_is_self && !has_type {
            let span = *first_param.as_ref().name.span();
            let replacement = otterc_ast::nodes::Type::Simple(struct_name.to_string());
            first_param.as_mut().ty = Some(Node::new(replacement, span));
        }
    }

    fn resolve_struct_method_name(&self, struct_id: u32, method: &str) -> Option<String> {
        let info = self.struct_info(struct_id);
        let candidate = format!("{}_{}", info.name, method);
        if self.declared_functions.contains_key(&candidate) {
            Some(candidate)
        } else {
            None
        }
    }

    pub fn compile_module(&mut self, program: &Program) -> Result<()> {
        for statement in &program.statements {
            self.record_statement_spans(statement.as_ref());
        }

        // Prepare Rust bridges
        let _libraries = prepare_rust_bridges(program, self.symbol_registry)?;

        // First pass: register all functions and types
        for statement in &program.statements {
            match statement.as_ref() {
                Statement::Function(func) => {
                    self.register_function_prototype(func.as_ref())?;
                }
                Statement::Struct {
                    name,
                    fields,
                    methods,
                    ..
                } => {
                    let (struct_id, struct_type) = self.ensure_struct_info(name);

                    let mut field_layout = Vec::new();
                    let mut field_indices = HashMap::new();
                    let mut field_types = Vec::new();
                    for (idx, (field_name, ty)) in fields.iter().enumerate() {
                        field_layout.push(self.map_ast_type(ty.as_ref())?);
                        field_indices.insert(field_name.clone(), idx);
                        field_types.push(self.otter_type_from_annotation(ty.as_ref()));
                    }

                    struct_type.set_body(&field_layout, false);
                    if let Some(info) = self.struct_infos.get_mut(struct_id as usize) {
                        info.field_indices = field_indices;
                        info.field_types = field_types;
                    }

                    // Register methods
                    for method in methods {
                        let mut method_func = method.as_ref().clone();
                        method_func.name = format!("{}_{}", name, method_func.name);
                        self.rewrite_method_self_param(&mut method_func, name);
                        self.register_function_prototype(&method_func)?;
                    }
                }
                _ => {}
            }
        }

        // Second pass: compile function bodies
        for statement in &program.statements {
            match statement.as_ref() {
                Statement::Function(func) => {
                    self.record_function_spans(func.as_ref());
                    self.compile_function(func.as_ref())?;
                }
                Statement::Struct { name, methods, .. } => {
                    for method in methods {
                        let mut method_func = method.as_ref().clone();
                        method_func.name = format!("{}_{}", name, method_func.name);
                        self.rewrite_method_self_param(&mut method_func, name);
                        self.record_function_spans(&method_func);
                        self.compile_function(&method_func)?;
                    }
                }
                _ => {}
            }
        }

        // Verify module
        if let Err(e) = self.module.verify() {
            self.module.print_to_stderr();
            return Err(anyhow!("Module verification failed: {}", e));
        }

        Ok(())
    }

    /// Declare an external function from the symbol registry
    fn declare_external_function(
        &mut self,
        name: &str,
        ffi_func: &crate::runtime::symbol_registry::FfiFunction,
    ) -> Result<FunctionValue<'ctx>> {
        use crate::runtime::symbol_registry::FfiType;

        // Helper to calculate struct size in bytes
        fn ffi_type_size(ffi_ty: &FfiType) -> usize {
            match ffi_ty {
                FfiType::Unit | FfiType::Bool => 1,
                FfiType::I32 => 4,
                FfiType::I64
                | FfiType::F64
                | FfiType::Str
                | FfiType::Opaque
                | FfiType::List
                | FfiType::Map => 8,
                FfiType::Struct { fields } | FfiType::Tuple(fields) => {
                    fields.iter().map(ffi_type_size).sum()
                }
            }
        }

        // Check if a struct type needs pointer passing on Windows x64
        fn needs_ptr_passing(ffi_ty: &FfiType, is_windows_x64: bool) -> bool {
            if !is_windows_x64 {
                return false;
            }
            match ffi_ty {
                FfiType::Struct { .. } | FfiType::Tuple(_) => ffi_type_size(ffi_ty) > 8,
                _ => false,
            }
        }

        let is_win64 = self.is_windows_x64();

        // Map FFI types to LLVM types
        fn map_ffi_type<'ctx>(
            context: &'ctx InkwellContext,
            string_ptr_type: PointerType<'ctx>,
            ffi_ty: &FfiType,
        ) -> BasicTypeEnum<'ctx> {
            match ffi_ty {
                FfiType::Unit => context.i8_type().into(),
                FfiType::Bool => context.bool_type().into(),
                FfiType::I32 => context.i32_type().into(),
                FfiType::I64 | FfiType::Opaque | FfiType::List | FfiType::Map => {
                    context.i64_type().into()
                }
                FfiType::F64 => context.f64_type().into(),
                FfiType::Str => string_ptr_type.into(),
                FfiType::Struct { fields } | FfiType::Tuple(fields) => {
                    let field_types: Vec<BasicTypeEnum> = fields
                        .iter()
                        .map(|f| map_ffi_type(context, string_ptr_type, f))
                        .collect();
                    context.struct_type(&field_types, false).into()
                }
            }
        }

        let map_type = |ffi_ty: &FfiType| map_ffi_type(self.context, self.string_ptr_type, ffi_ty);

        // Check if return type needs sret on Windows x64
        let ret_needs_sret = needs_ptr_passing(&ffi_func.signature.result, is_win64);

        // Build parameter types
        let mut param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = Vec::new();

        // If return needs sret, add hidden first parameter (pointer to return struct)
        if ret_needs_sret {
            param_types.push(self.string_ptr_type.into());
        }

        // Map regular parameter types
        for param_ty in &ffi_func.signature.params {
            if needs_ptr_passing(param_ty, is_win64) {
                // Pass large struct by pointer
                param_types.push(self.string_ptr_type.into());
            } else {
                param_types.push(map_type(param_ty).into());
            }
        }

        // Create function type
        let fn_type = if ret_needs_sret {
            // With sret, return void (result goes through hidden first param)
            self.context.void_type().fn_type(&param_types, false)
        } else {
            match &ffi_func.signature.result {
                FfiType::Unit => self.context.void_type().fn_type(&param_types, false),
                result_ty => {
                    let ret_type = map_type(result_ty);
                    ret_type.fn_type(&param_types, false)
                }
            }
        };

        // Declare the function using the symbol name (not the user-facing name)
        let function = self.module.add_function(&ffi_func.symbol, fn_type, None);

        // Add sret attribute if needed
        if ret_needs_sret {
            let ret_type: inkwell::types::AnyTypeEnum = match map_type(&ffi_func.signature.result) {
                BasicTypeEnum::StructType(s) => s.into(),
                BasicTypeEnum::IntType(i) => i.into(),
                BasicTypeEnum::FloatType(f) => f.into(),
                BasicTypeEnum::PointerType(p) => p.into(),
                BasicTypeEnum::ArrayType(a) => a.into(),
                BasicTypeEnum::VectorType(v) => v.into(),
                BasicTypeEnum::ScalableVectorType(v) => v.into(),
            };
            function.add_attribute(
                inkwell::attributes::AttributeLoc::Param(0),
                self.context.create_type_attribute(
                    inkwell::attributes::Attribute::get_named_enum_kind_id("sret"),
                    ret_type,
                ),
            );
        }

        // Cache it under the user-facing name
        self.declared_functions.insert(name.to_string(), function);

        Ok(function)
    }

    fn get_or_declare_ffi_function(&mut self, name: &str) -> Result<FunctionValue<'ctx>> {
        if let Some(function) = self.declared_functions.get(name) {
            return Ok(*function);
        }

        let ffi_func = self
            .symbol_registry
            .resolve(name)
            .ok_or_else(|| anyhow!("{name} not found in registry"))?;
        self.declare_external_function(name, &ffi_func)
    }

    /// Map AST type to LLVM type
    fn map_ast_type(&self, ty: &otterc_ast::nodes::Type) -> Result<BasicTypeEnum<'ctx>> {
        match ty {
            otterc_ast::nodes::Type::Simple(name) => match name.as_str() {
                // Lists and Maps are opaque handles
                "int" | "i64" | "list" | "List" | "map" | "Map" => {
                    Ok(self.context.i64_type().into())
                }
                "float" | "f64" => Ok(self.context.f64_type().into()),
                "bool" => Ok(self.context.bool_type().into()),
                "string" | "str" => Ok(self.string_ptr_type.into()),
                "void" | "unit" => Ok(self.context.i8_type().into()), // Unit as i8 (or void for return)
                other => {
                    if let Some(id) = self.struct_id(other) {
                        Ok(self.struct_info(id).ty.into())
                    } else {
                        Ok(self.context.i64_type().into())
                    }
                }
            },
            otterc_ast::nodes::Type::Generic { .. } => Ok(self.context.i64_type().into()), // Treat generics as opaque handles
        }
    }

    fn otter_type_from_annotation(&self, ty: &otterc_ast::nodes::Type) -> OtterType {
        match ty {
            otterc_ast::nodes::Type::Simple(name) => match name.as_str() {
                "int" | "i64" => OtterType::I64,
                "float" | "f64" => OtterType::F64,
                "bool" => OtterType::Bool,
                "string" | "str" => OtterType::Str,
                "unit" | "void" => OtterType::Unit,
                "list" | "List" => OtterType::opaque_list(),
                "map" | "Map" => OtterType::Map,
                other => self
                    .struct_id(other)
                    .map(OtterType::Struct)
                    .unwrap_or(OtterType::Opaque),
            },
            otterc_ast::nodes::Type::Generic { base, args, .. } => {
                // Handle generic types like list<str>, map<str, int>, etc.
                match base.as_str() {
                    "list" | "List" => {
                        let element = args
                            .first()
                            .map(|arg| self.otter_type_from_annotation(arg.as_ref()))
                            .unwrap_or(OtterType::Opaque);
                        OtterType::list_of(element)
                    }
                    "map" | "Map" => OtterType::Map,
                    _ => OtterType::Opaque,
                }
            }
        }
    }

    fn otter_type_from_basic_type(&self, ty: BasicTypeEnum<'ctx>) -> OtterType {
        match ty {
            BasicTypeEnum::IntType(int_type) if int_type == self.context.bool_type() => {
                OtterType::Bool
            }
            BasicTypeEnum::IntType(int_type) if int_type == self.context.i32_type() => {
                OtterType::I32
            }
            BasicTypeEnum::IntType(int_type) if int_type == self.context.i64_type() => {
                OtterType::I64
            }
            BasicTypeEnum::IntType(int_type) if int_type == self.context.i8_type() => {
                OtterType::Unit
            }
            BasicTypeEnum::FloatType(float_type) if float_type == self.context.f64_type() => {
                OtterType::F64
            }
            BasicTypeEnum::PointerType(ptr_type) if ptr_type == self.string_ptr_type => {
                OtterType::Str
            }
            BasicTypeEnum::StructType(struct_type) => {
                if let Some(id) = struct_type
                    .get_name()
                    .and_then(|name| self.struct_id(name.to_str().unwrap_or_default()))
                {
                    return OtterType::Struct(id);
                }
                // Anonymous struct or tuple
                let field_types: Vec<OtterType> = struct_type
                    .get_field_types()
                    .iter()
                    .map(|&ty| self.otter_type_from_basic_type(ty))
                    .collect();
                OtterType::Tuple(field_types)
            }
            _ => OtterType::Opaque,
        }
    }

    fn register_function_prototype(&mut self, func: &otterc_ast::nodes::Function) -> Result<()> {
        let ret_type: Option<BasicTypeEnum> = if let Some(ret_ty) = &func.ret_ty {
            let mapped_ty = self.map_ast_type(ret_ty.as_ref())?;
            // Check if it's effectively unit/void
            if let otterc_ast::nodes::Type::Simple(name) = ret_ty.as_ref() {
                if name == "void" || name == "unit" {
                    None
                } else {
                    Some(mapped_ty)
                }
            } else {
                Some(mapped_ty)
            }
        } else {
            None
        };

        let mut param_types = Vec::new();
        for param in &func.params {
            if let Some(ty) = &param.as_ref().ty {
                param_types.push(self.map_ast_type(ty.as_ref())?.into());
            } else {
                // Default to i64 if no type specified
                param_types.push(self.context.i64_type().into());
            }
        }

        let fn_type = if let Some(rt) = ret_type {
            rt.fn_type(&param_types, false)
        } else {
            self.context.void_type().fn_type(&param_types, false)
        };

        let llvm_name = if func.name == "main" {
            "otter_entry"
        } else {
            &func.name
        };
        let function = self.module.add_function(llvm_name, fn_type, None);
        self.declared_functions.insert(func.name.clone(), function);

        // Store return type for later use in eval_call_expr
        let ret_otter_type = if let Some(ret_ty) = &func.ret_ty {
            self.otter_type_from_annotation(ret_ty.as_ref())
        } else {
            OtterType::Unit
        };
        self.function_return_types
            .insert(func.name.clone(), ret_otter_type);

        // Store default values
        let defaults: Vec<Option<Expr>> = func
            .params
            .iter()
            .map(|p| p.as_ref().default.as_ref().map(|e| e.as_ref().clone()))
            .collect();
        self.function_defaults.insert(func.name.clone(), defaults);

        Ok(())
    }

    fn compile_function(&mut self, func: &otterc_ast::nodes::Function) -> Result<()> {
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

            // Determine type from AST or default to I64
            let (_llvm_type, otter_type) = if let Some(ty) = &param.as_ref().ty {
                let llvm_ty = self.map_ast_type(ty.as_ref())?;
                let otter_ty = self.otter_type_from_annotation(ty.as_ref());
                (llvm_ty, otter_ty)
            } else {
                (self.context.i64_type().into(), OtterType::I64)
            };

            // Allocate stack space for parameter
            let alloca = self.create_entry_block_alloca(
                *function,
                param_name.as_ref().as_str(),
                otter_type.clone(),
            )?;
            self.builder.build_store(alloca, arg_val)?;

            // Add to context
            ctx.insert(
                param_name.as_ref().to_string(),
                crate::codegen::llvm::compiler::types::Variable {
                    ptr: alloca,
                    ty: otter_type,
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
                let ret_ty = func.ret_ty.as_ref().unwrap();
                let llvm_ty = self.map_ast_type(ret_ty.as_ref())?;

                let default_val: inkwell::values::BasicValueEnum = match llvm_ty {
                    BasicTypeEnum::IntType(t) => t.const_zero().into(),
                    BasicTypeEnum::FloatType(t) => t.const_zero().into(),
                    BasicTypeEnum::PointerType(t) => t.const_null().into(),
                    BasicTypeEnum::StructType(t) => t.const_zero().into(), // For unit/void which might be mapped to struct or i8
                    BasicTypeEnum::ArrayType(t) => t.const_zero().into(),
                    BasicTypeEnum::VectorType(t) => t.const_zero().into(),
                    _ => {
                        // For other types (like pointers to structs/arrays), return null/zero
                        // This is safer than crashing, though ideally we'd have specific default values
                        llvm_ty.const_zero()
                    }
                };

                self.builder.build_return(Some(&default_val))?;
            }
        }

        Ok(())
    }

    /// Creates a new stack allocation instruction in the entry block of the function.
    pub(super) fn create_entry_block_alloca(
        &self,
        function: FunctionValue<'ctx>,
        name: &str,
        otter_type: OtterType,
    ) -> Result<PointerValue<'ctx>> {
        let builder = self.context.create_builder();
        let entry_block = function.get_first_basic_block().unwrap();

        // If the entry block has terminators, insert before the first one.
        // Otherwise, insert at the end of the block.
        match entry_block.get_first_instruction() {
            Some(first_instr) => builder.position_before(&first_instr),
            None => builder.position_at_end(entry_block),
        }

        let llvm_type: BasicTypeEnum = self
            .basic_type(otter_type)?
            .unwrap_or_else(|| self.context.i8_type().into());

        Ok(builder.build_alloca(llvm_type, name)?)
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

    /// Build a heap allocation using the GC
    pub fn build_heap_alloc(&mut self, size: u64) -> Result<PointerValue<'ctx>> {
        let alloc_func = self.get_or_declare_ffi_function("gc.alloc")?;

        let size_val = self.context.i64_type().const_int(size, false);
        let call = self
            .builder
            .build_call(alloc_func, &[size_val.into()], "alloc")?;

        Ok(call
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value())
    }

    /// Register a root with the GC
    pub fn build_gc_add_root(&mut self, ptr: PointerValue<'ctx>) -> Result<()> {
        let add_root_func = self.get_or_declare_ffi_function("gc.add_root")?;

        let ptr_as_int = self
            .builder
            .build_ptr_to_int(ptr, self.context.i64_type(), "ptr_int")?;
        self.builder
            .build_call(add_root_func, &[ptr_as_int.into()], "")?;

        Ok(())
    }

    /// Remove a root from the GC
    pub fn build_gc_remove_root(&mut self, ptr: PointerValue<'ctx>) -> Result<()> {
        let remove_root_func = self.get_or_declare_ffi_function("gc.remove_root")?;

        let ptr_as_int = self
            .builder
            .build_ptr_to_int(ptr, self.context.i64_type(), "ptr_int")?;
        self.builder
            .build_call(remove_root_func, &[ptr_as_int.into()], "")?;

        Ok(())
    }
}
