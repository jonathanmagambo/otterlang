use anyhow::{Result, anyhow, bail};
use inkwell::AddressSpace;
use inkwell::IntPredicate;
use inkwell::types::{BasicTypeEnum, PointerType, StructType};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue, IntValue};
use std::collections::BTreeSet;

use crate::codegen::llvm::compiler::Compiler;
use crate::codegen::llvm::compiler::types::{EvaluatedValue, FunctionContext, OtterType, Variable};
use crate::typecheck::TypeInfo;
use otterc_ast::nodes::{BinaryOp, Block, Expr, FStringPart, Literal, Node, Statement, UnaryOp};

struct CapturedVariable<'ctx> {
    name: String,
    ty: OtterType,
    llvm_ty: BasicTypeEnum<'ctx>,
}

impl<'ctx> Compiler<'ctx> {
    fn eval_await_expr(
        &mut self,
        expr: &Expr,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        let handle = self.eval_expr(expr, ctx)?;
        let value = handle
            .value
            .ok_or_else(|| anyhow!("await expects a task handle value"))?;
        let join_fn = self.get_or_declare_ffi_function("task.join")?;
        self.builder
            .build_call(join_fn, &[value.into()], "task_join")?;
        Ok(EvaluatedValue {
            ty: OtterType::Unit,
            value: None,
        })
    }

    fn eval_spawn_expr(
        &mut self,
        expr: &Expr,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        let mut captured = BTreeSet::new();
        self.collect_captured_names(expr, ctx, &mut captured);
        let capture_names: Vec<String> = captured.into_iter().collect();

        let mut capture_fields = Vec::new();
        for name in capture_names {
            if let Some(var) = ctx.get(&name)
                && let Some(llvm_ty) = self.basic_type(var.ty.clone())?
            {
                capture_fields.push(CapturedVariable {
                    name,
                    ty: var.ty.clone(),
                    llvm_ty,
                });
            }
        }

        let context_struct = if capture_fields.is_empty() {
            None
        } else {
            let field_types: Vec<BasicTypeEnum> =
                capture_fields.iter().map(|field| field.llvm_ty).collect();
            Some(self.context.struct_type(&field_types, false))
        };

        let spawn_id = self.next_spawn_id;
        self.next_spawn_id += 1;

        let wrapper = self.build_spawn_wrapper(spawn_id, expr, context_struct, &capture_fields)?;

        if let Some(struct_type) = context_struct {
            let context_ptr = self.builder.build_malloc(struct_type, "spawn_ctx")?;
            for (index, field) in capture_fields.iter().enumerate() {
                let var = ctx.get(&field.name).ok_or_else(|| {
                    anyhow!("captured variable '{}' missing from scope", field.name)
                })?;
                let loaded = self
                    .builder
                    .build_load(field.llvm_ty, var.ptr, &field.name)?;
                let field_ptr = self.builder.build_struct_gep(
                    struct_type,
                    context_ptr,
                    index as u32,
                    &format!("spawn_ctx_field_{}", field.name),
                )?;
                self.builder.build_store(field_ptr, loaded)?;
            }
            let raw_ptr = self.builder.build_pointer_cast(
                context_ptr,
                self.raw_ptr_type(),
                "spawn_ctx_raw",
            )?;
            let push_fn = self.get_spawn_context_push_fn();
            let id_const = self.context.i64_type().const_int(spawn_id, false);
            self.builder
                .build_call(push_fn, &[id_const.into(), raw_ptr.into()], "")?;
        }

        let spawn_fn = self.get_task_spawn_fn();
        let callback_ptr = wrapper.as_global_value().as_pointer_value();
        let handle = self
            .builder
            .build_call(spawn_fn, &[callback_ptr.into()], "task_handle")?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| anyhow!("task.spawn did not return a handle"))?;

        Ok(EvaluatedValue::with_value(handle, OtterType::Opaque))
    }

    fn build_spawn_wrapper(
        &mut self,
        spawn_id: u64,
        expr: &Expr,
        context_type: Option<StructType<'ctx>>,
        captures: &[CapturedVariable<'ctx>],
    ) -> Result<FunctionValue<'ctx>> {
        let fn_name = format!("spawn_wrapper_{}", spawn_id);
        let fn_type = self.context.void_type().fn_type(&[], false);
        let function = self.module.add_function(&fn_name, fn_type, None);
        let entry = self.context.append_basic_block(function, "entry");
        let prev_block = self.builder.get_insert_block();
        self.builder.position_at_end(entry);

        let mut wrapper_ctx = FunctionContext::new();
        let mut raw_ptr: Option<inkwell::values::PointerValue<'ctx>> = None;

        if let Some(struct_type) = context_type {
            let pop_fn = self.get_spawn_context_pop_fn();
            let id_const = self.context.i64_type().const_int(spawn_id, false);
            let value = self
                .builder
                .build_call(pop_fn, &[id_const.into()], "spawn_ctx_raw")?
                .try_as_basic_value()
                .left()
                .ok_or_else(|| anyhow!("spawn context queue returned null"))?
                .into_pointer_value();
            raw_ptr = Some(value);
            let typed_ptr = self.builder.build_pointer_cast(
                value,
                self.struct_ptr_type(struct_type),
                "spawn_ctx",
            )?;

            for (index, field) in captures.iter().enumerate() {
                let field_ptr = self.builder.build_struct_gep(
                    struct_type,
                    typed_ptr,
                    index as u32,
                    &format!("spawn_capture_gep_{}", field.name),
                )?;
                let loaded = self
                    .builder
                    .build_load(field.llvm_ty, field_ptr, &field.name)?;
                let alloca =
                    self.create_entry_block_alloca(function, &field.name, field.ty.clone())?;
                self.builder.build_store(alloca, loaded)?;
                wrapper_ctx.insert(
                    field.name.clone(),
                    Variable {
                        ptr: alloca,
                        ty: field.ty.clone(),
                    },
                );
            }
        }

        let _ = self.eval_expr(expr, &mut wrapper_ctx)?;

        if let Some(ptr) = raw_ptr {
            self.builder.build_free(ptr)?;
        }

        self.builder.build_return(None)?;

        if let Some(block) = prev_block {
            self.builder.position_at_end(block);
        }

        Ok(function)
    }

    fn collect_captured_names(
        &self,
        expr: &Expr,
        ctx: &FunctionContext<'ctx>,
        captures: &mut BTreeSet<String>,
    ) {
        match expr {
            Expr::Literal(_) => {}
            Expr::Identifier(name) => {
                if ctx.get(name).is_some() {
                    captures.insert(name.clone());
                }
            }
            Expr::Member { object, .. } => {
                self.collect_captured_names(object.as_ref().as_ref(), ctx, captures);
            }
            Expr::Call { func, args } => {
                self.collect_captured_names(func.as_ref().as_ref(), ctx, captures);
                for arg in args {
                    self.collect_captured_names(arg.as_ref(), ctx, captures);
                }
            }
            Expr::Binary { left, right, .. } => {
                self.collect_captured_names(left.as_ref().as_ref(), ctx, captures);
                self.collect_captured_names(right.as_ref().as_ref(), ctx, captures);
            }
            Expr::Unary { expr, .. } => {
                self.collect_captured_names(expr.as_ref().as_ref(), ctx, captures);
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.collect_captured_names(cond.as_ref().as_ref(), ctx, captures);
                self.collect_captured_names(then_branch.as_ref().as_ref(), ctx, captures);
                if let Some(expr) = else_branch {
                    self.collect_captured_names(expr.as_ref().as_ref(), ctx, captures);
                }
            }
            Expr::Match { value, arms } => {
                self.collect_captured_names(value.as_ref().as_ref(), ctx, captures);
                for arm in arms {
                    if let Some(guard) = &arm.as_ref().guard {
                        self.collect_captured_names(guard.as_ref(), ctx, captures);
                    }
                    self.collect_captured_names_in_block(arm.as_ref().body.as_ref(), ctx, captures);
                }
            }
            Expr::Range { start, end } => {
                self.collect_captured_names(start.as_ref().as_ref(), ctx, captures);
                self.collect_captured_names(end.as_ref().as_ref(), ctx, captures);
            }
            Expr::Array(elements) => {
                for element in elements {
                    self.collect_captured_names(element.as_ref(), ctx, captures);
                }
            }
            Expr::Dict(pairs) => {
                for (key, value) in pairs {
                    self.collect_captured_names(key.as_ref(), ctx, captures);
                    self.collect_captured_names(value.as_ref(), ctx, captures);
                }
            }
            Expr::ListComprehension {
                element,
                iterable,
                condition,
                ..
            } => {
                self.collect_captured_names(element.as_ref().as_ref(), ctx, captures);
                self.collect_captured_names(iterable.as_ref().as_ref(), ctx, captures);
                if let Some(cond) = condition {
                    self.collect_captured_names(cond.as_ref().as_ref(), ctx, captures);
                }
            }
            Expr::DictComprehension {
                key,
                value,
                iterable,
                condition,
                ..
            } => {
                self.collect_captured_names(key.as_ref().as_ref(), ctx, captures);
                self.collect_captured_names(value.as_ref().as_ref(), ctx, captures);
                self.collect_captured_names(iterable.as_ref().as_ref(), ctx, captures);
                if let Some(cond) = condition {
                    self.collect_captured_names(cond.as_ref().as_ref(), ctx, captures);
                }
            }
            Expr::FString { parts } => {
                for part in parts {
                    if let FStringPart::Expr(expr) = part.as_ref() {
                        self.collect_captured_names(expr.as_ref(), ctx, captures);
                    }
                }
            }
            Expr::Struct { fields, .. } => {
                for (_, value) in fields {
                    self.collect_captured_names(value.as_ref(), ctx, captures);
                }
            }
            Expr::Await(inner) | Expr::Spawn(inner) => {
                self.collect_captured_names(inner.as_ref().as_ref(), ctx, captures);
            }
        }
    }

    fn collect_captured_names_in_block(
        &self,
        block: &Block,
        ctx: &FunctionContext<'ctx>,
        captures: &mut BTreeSet<String>,
    ) {
        for stmt in &block.statements {
            self.collect_captured_names_in_statement(stmt.as_ref(), ctx, captures);
        }
    }

    fn collect_captured_names_in_statement(
        &self,
        stmt: &Statement,
        ctx: &FunctionContext<'ctx>,
        captures: &mut BTreeSet<String>,
    ) {
        match stmt {
            Statement::Expr(expr)
            | Statement::Let { expr, .. }
            | Statement::Assignment { expr, .. }
            | Statement::Return(Some(expr)) => {
                self.collect_captured_names(expr.as_ref(), ctx, captures);
            }
            Statement::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
            } => {
                self.collect_captured_names(cond.as_ref(), ctx, captures);
                self.collect_captured_names_in_block(then_block.as_ref(), ctx, captures);
                for (elif_cond, block) in elif_blocks {
                    self.collect_captured_names(elif_cond.as_ref(), ctx, captures);
                    self.collect_captured_names_in_block(block.as_ref(), ctx, captures);
                }
                if let Some(block) = else_block {
                    self.collect_captured_names_in_block(block.as_ref(), ctx, captures);
                }
            }
            Statement::For { iterable, body, .. } => {
                self.collect_captured_names(iterable.as_ref(), ctx, captures);
                self.collect_captured_names_in_block(body.as_ref(), ctx, captures);
            }
            Statement::While { cond, body } => {
                self.collect_captured_names(cond.as_ref(), ctx, captures);
                self.collect_captured_names_in_block(body.as_ref(), ctx, captures);
            }
            Statement::Block(block) => {
                self.collect_captured_names_in_block(block.as_ref(), ctx, captures);
            }
            Statement::Return(None)
            | Statement::Break
            | Statement::Continue
            | Statement::Pass
            | Statement::Use { .. }
            | Statement::PubUse { .. }
            | Statement::Struct { .. }
            | Statement::Enum { .. }
            | Statement::TypeAlias { .. }
            | Statement::Function(_) => {}
        }
    }

    fn get_spawn_context_push_fn(&mut self) -> FunctionValue<'ctx> {
        if let Some(func) = self.declared_functions.get("__spawn_context_push") {
            return *func;
        }
        let ptr_ty = self.raw_ptr_type();
        let fn_type = self
            .context
            .void_type()
            .fn_type(&[self.context.i64_type().into(), ptr_ty.into()], false);
        let function = self
            .module
            .add_function("otter_spawn_context_push", fn_type, None);
        self.declared_functions
            .insert("__spawn_context_push".to_string(), function);
        function
    }

    fn get_spawn_context_pop_fn(&mut self) -> FunctionValue<'ctx> {
        if let Some(func) = self.declared_functions.get("__spawn_context_pop") {
            return *func;
        }
        let ptr_ty = self.raw_ptr_type();
        let fn_type = ptr_ty.fn_type(&[self.context.i64_type().into()], false);
        let function = self
            .module
            .add_function("otter_spawn_context_pop", fn_type, None);
        self.declared_functions
            .insert("__spawn_context_pop".to_string(), function);
        function
    }

    fn get_task_spawn_fn(&mut self) -> FunctionValue<'ctx> {
        if let Some(func) = self.declared_functions.get("__task_spawn") {
            return *func;
        }
        let callback_type = self.context.void_type().fn_type(&[], false);
        #[expect(deprecated, reason = "TODO: Use Context::ptr_type instead")]
        let callback_ptr = callback_type.ptr_type(AddressSpace::default());
        let fn_type = self
            .context
            .i64_type()
            .fn_type(&[callback_ptr.into()], false);
        let function = self.module.add_function("otter_task_spawn", fn_type, None);
        self.declared_functions
            .insert("__task_spawn".to_string(), function);
        function
    }

    fn raw_ptr_type(&self) -> PointerType<'ctx> {
        #[expect(deprecated, reason = "TODO: Use Context::ptr_type instead")]
        {
            self.context.i8_type().ptr_type(AddressSpace::default())
        }
    }

    fn struct_ptr_type(&self, ty: StructType<'ctx>) -> PointerType<'ctx> {
        #[expect(deprecated, reason = "TODO: Use Context::ptr_type instead")]
        {
            ty.ptr_type(AddressSpace::default())
        }
    }
    pub(crate) fn eval_expr(
        &mut self,
        expr: &Expr,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        match expr {
            Expr::Literal(lit) => {
                // Get type from type checker first
                // Clone the type info to avoid borrow conflicts with eval_literal
                let expr_id = expr as *const Expr as usize;
                let type_info_opt = self.expr_types.get(&expr_id).cloned();
                self.eval_literal(lit.as_ref(), type_info_opt.as_ref())
            }
            Expr::Identifier(name) => {
                if let Some(var) = ctx.get(name) {
                    let var_ty = var.ty.clone();
                    if let Some(basic_ty) = self.basic_type(var_ty.clone())? {
                        let val = self.builder.build_load(basic_ty, var.ptr, name)?;
                        Ok(EvaluatedValue::with_value(val, var_ty))
                    } else {
                        // Unit type - no value to load
                        Ok(EvaluatedValue {
                            ty: var_ty,
                            value: None,
                        })
                    }
                } else {
                    bail!("Variable {} not found", name);
                }
            }
            Expr::Binary { left, op, right } => {
                self.eval_binary_expr(left.as_ref().as_ref(), op, right.as_ref().as_ref(), ctx)
            }
            Expr::Unary { op, expr } => self.eval_unary_expr(op, expr.as_ref().as_ref(), ctx),
            Expr::Call { func: _, args: _ } => self.eval_call_expr(expr, ctx),
            Expr::Member { object, field } => {
                if let Some(value) =
                    self.try_build_enum_member(expr, object.as_ref().as_ref(), field, ctx)?
                {
                    Ok(value)
                } else if self
                    .module_path_from_expr(object.as_ref().as_ref())
                    .is_some()
                {
                    // Module references don't need runtime materialization.
                    Ok(EvaluatedValue {
                        ty: OtterType::Unit,
                        value: None,
                    })
                } else {
                    let object_value = self.eval_expr(object.as_ref().as_ref(), ctx)?;
                    if object_value.value.is_none() {
                        bail!("cannot access field '{}' without value", field);
                    }
                    let object_ty = object_value.ty.clone();
                    if let OtterType::Struct(struct_id) = object_ty {
                        let struct_value = object_value.value.unwrap().into_struct_value();
                        let info = self.struct_info(struct_id);
                        let idx = info.field_indices.get(field).copied().ok_or_else(|| {
                            anyhow!("struct '{}' has no field '{}'", info.name, field)
                        })?;
                        let extracted = self
                            .builder
                            .build_extract_value(struct_value, idx as u32, field)
                            .map_err(|e| anyhow!("failed to extract field '{}': {e}", field))?;
                        let field_ty = info.field_types[idx].clone();
                        Ok(EvaluatedValue::with_value(extracted, field_ty))
                    } else {
                        bail!(
                            "Complex member expressions not yet supported (expr, ty={:?})",
                            object_ty
                        );
                    }
                }
            }
            Expr::Struct { name, fields } => {
                let (struct_id, _) = self
                    .struct_info_by_name(name)
                    .ok_or_else(|| anyhow!("unknown struct type '{}'", name))?;
                let (struct_name, struct_ty) = {
                    let info = self.struct_info(struct_id);
                    (info.name.clone(), info.ty)
                };
                let mut aggregate = struct_ty.get_undef();
                for (field_name, field_expr) in fields {
                    let idx = {
                        let info = self.struct_info(struct_id);
                        info.field_indices.get(field_name).copied().ok_or_else(|| {
                            anyhow!("struct '{}' has no field '{}'", struct_name, field_name)
                        })?
                    };
                    let field_value = self.eval_expr(field_expr.as_ref(), ctx)?;
                    let raw_value = field_value
                        .value
                        .ok_or_else(|| anyhow!("field '{}' produced no value", field_name))?;
                    let expected_ty = {
                        let info = self.struct_info(struct_id);
                        info.field_types[idx].clone()
                    };
                    let coerced =
                        self.coerce_type(raw_value, field_value.ty.clone(), expected_ty)?;
                    aggregate = self
                        .builder
                        .build_insert_value(aggregate, coerced, idx as u32, field_name)
                        .map_err(|e| anyhow!("failed to insert field '{}': {e}", field_name))?
                        .into_struct_value();
                }

                Ok(EvaluatedValue::with_value(
                    aggregate.into(),
                    OtterType::Struct(struct_id),
                ))
            }
            Expr::If {
                cond: _,
                then_branch: _,
                else_branch: _,
            } => self.eval_if_expr(expr, ctx),
            Expr::Match { value: _, arms: _ } => self.eval_match_expr(expr, ctx),
            Expr::FString { parts: _ } => self.eval_fstring_expr(expr, ctx),
            Expr::Array(elements) => {
                let expr_id = expr as *const Expr as usize;
                let expr_type = self.expr_types.get(&expr_id).cloned();
                self.eval_array_expr(elements, expr_type.as_ref(), ctx)
            }
            Expr::ListComprehension {
                element,
                var,
                iterable,
                condition,
            } => self.eval_list_comprehension(
                expr,
                element.as_ref().as_ref(),
                var,
                iterable.as_ref().as_ref(),
                condition.as_ref().map(|c| c.as_ref().as_ref()),
                ctx,
            ),
            Expr::DictComprehension {
                key,
                value,
                var,
                iterable,
                condition,
            } => self.eval_dict_comprehension(
                expr,
                key.as_ref().as_ref(),
                value.as_ref().as_ref(),
                var,
                iterable.as_ref().as_ref(),
                condition.as_ref().map(|c| c.as_ref().as_ref()),
                ctx,
            ),
            Expr::Await(expr) => self.eval_await_expr(expr.as_ref().as_ref(), ctx),
            Expr::Spawn(expr) => self.eval_spawn_expr(expr.as_ref().as_ref(), ctx),
            _ => bail!("Expression type not implemented: {:?}", expr),
        }
    }

    fn eval_match_expr(
        &mut self,
        expr: &Expr,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        let Expr::Match { value, arms } = expr else {
            bail!("Expected Match expression")
        };

        let matched_val = self.eval_expr(value.as_ref().as_ref(), ctx)?;
        let function = self
            .builder
            .get_insert_block()
            .unwrap()
            .get_parent()
            .unwrap();
        let merge_bb = self.context.append_basic_block(function, "match_merge");

        let mut incoming_values = Vec::new();
        let mut incoming_blocks = Vec::new();

        let mut next_check_bb = self
            .context
            .append_basic_block(function, "match_arm_check_0");
        self.builder.build_unconditional_branch(next_check_bb)?;

        let matched_type_info = self.expr_type(value.as_ref().as_ref()).cloned();

        for (i, arm) in arms.iter().enumerate() {
            self.builder.position_at_end(next_check_bb);

            next_check_bb = if i < arms.len() - 1 {
                self.context
                    .append_basic_block(function, &format!("match_arm_check_{}", i + 1))
            } else {
                self.context.append_basic_block(function, "match_no_match")
            };

            let body_bb = self
                .context
                .append_basic_block(function, &format!("match_arm_body_{}", i));

            self.compile_pattern_match(
                &arm.as_ref().pattern,
                &matched_val,
                matched_type_info.clone(),
                body_bb,
                next_check_bb,
                ctx,
            )?;

            self.builder.position_at_end(body_bb);
            let body_val = self.lower_block_expression(&arm.as_ref().body, function, ctx)?;

            if self
                .builder
                .get_insert_block()
                .unwrap()
                .get_terminator()
                .is_none()
            {
                self.builder.build_unconditional_branch(merge_bb)?;
            }

            let current_bb = self.builder.get_insert_block().unwrap();
            if let Some(v) = body_val.value {
                incoming_values.push(v);
                incoming_blocks.push(current_bb);
            }
        }

        self.builder.position_at_end(next_check_bb);
        self.builder.build_unreachable()?;

        self.builder.position_at_end(merge_bb);

        if !incoming_values.is_empty() {
            let phi_type = incoming_values[0].get_type();
            let phi = self.builder.build_phi(phi_type, "match_result")?;

            for (val, block) in incoming_values.iter().zip(incoming_blocks.iter()) {
                phi.add_incoming(&[(val, *block)]);
            }

            // Infer the result type from the match expression's type annotation if available
            let match_expr_id = expr as *const Expr as usize;
            let result_ty = if let Some(type_info) = self.expr_types.get(&match_expr_id) {
                self.typeinfo_to_otter_type(type_info).unwrap_or_else(|| {
                    // Fallback to inferring from phi type
                    if phi_type.is_pointer_type() {
                        OtterType::Str
                    } else if phi_type.is_int_type() {
                        let int_type = phi_type.into_int_type();
                        if int_type.get_bit_width() == 64 {
                            OtterType::I64
                        } else if int_type.get_bit_width() == 1 {
                            OtterType::Bool
                        } else {
                            OtterType::I32
                        }
                    } else if phi_type.is_float_type() {
                        OtterType::F64
                    } else {
                        OtterType::Opaque
                    }
                })
            } else {
                // No type info, infer from phi type
                if phi_type.is_pointer_type() {
                    OtterType::Str
                } else if phi_type.is_int_type() {
                    let int_type = phi_type.into_int_type();
                    if int_type.get_bit_width() == 64 {
                        OtterType::I64
                    } else if int_type.get_bit_width() == 1 {
                        OtterType::Bool
                    } else {
                        OtterType::I32
                    }
                } else if phi_type.is_float_type() {
                    OtterType::F64
                } else {
                    OtterType::Opaque
                }
            };

            Ok(EvaluatedValue::with_value(phi.as_basic_value(), result_ty))
        } else {
            Ok(EvaluatedValue {
                ty: OtterType::Unit,
                value: None,
            })
        }
    }

    fn compile_pattern_match(
        &mut self,
        pattern: &Node<otterc_ast::nodes::Pattern>,
        matched_val: &EvaluatedValue<'ctx>,
        matched_type: Option<TypeInfo>,
        success_bb: inkwell::basic_block::BasicBlock<'ctx>,
        fail_bb: inkwell::basic_block::BasicBlock<'ctx>,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<()> {
        use otterc_ast::nodes::Pattern;

        match pattern.as_ref() {
            Pattern::Wildcard => {
                self.builder.build_unconditional_branch(success_bb)?;
                Ok(())
            }
            Pattern::Literal(lit) => {
                // Get type info from the pattern's expression if available
                let type_info = None; // Patterns don't have type info in expr_types, use None
                let lit_val = self.eval_literal(lit.as_ref(), type_info)?;
                let is_equal = self.build_equality_check(matched_val, &lit_val)?;
                self.builder
                    .build_conditional_branch(is_equal, success_bb, fail_bb)?;
                Ok(())
            }
            Pattern::Identifier(name) => {
                let function = self
                    .builder
                    .get_insert_block()
                    .unwrap()
                    .get_parent()
                    .unwrap();
                let matched_ty = matched_val.ty.clone();
                let alloca = self.create_entry_block_alloca(function, name, matched_ty.clone())?;

                if let Some(v) = matched_val.value {
                    self.builder.build_store(alloca, v)?;
                }

                ctx.insert(
                    name.clone(),
                    crate::codegen::llvm::compiler::types::Variable {
                        ptr: alloca,
                        ty: matched_ty,
                    },
                );

                self.builder.build_unconditional_branch(success_bb)?;
                Ok(())
            }
            Pattern::EnumVariant {
                enum_name,
                variant,
                fields,
            } => {
                let get_tag_fn = self.get_or_declare_ffi_function("runtime.enum.get_tag")?;
                let handle = matched_val
                    .value
                    .ok_or_else(|| anyhow!("Enum value is void"))?;

                let tag_val = self
                    .builder
                    .build_call(get_tag_fn, &[handle.into()], "tag")?
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                let layout = self
                    .enum_layout(enum_name)
                    .ok_or_else(|| anyhow!("Enum layout not found for {}", enum_name))?;
                let expected_tag = layout
                    .tag_of(variant)
                    .ok_or_else(|| anyhow!("Variant {} not found in {}", variant, enum_name))?;

                let expected_tag_val = self
                    .context
                    .i64_type()
                    .const_int(expected_tag as u64, false);
                let tag_match = self.builder.build_int_compare(
                    IntPredicate::EQ,
                    tag_val,
                    expected_tag_val,
                    "tag_match",
                )?;

                if fields.is_empty() {
                    self.builder
                        .build_conditional_branch(tag_match, success_bb, fail_bb)?;
                } else {
                    let field_check_bb = self.context.append_basic_block(
                        self.builder
                            .get_insert_block()
                            .unwrap()
                            .get_parent()
                            .unwrap(),
                        "enum_field_check",
                    );
                    self.builder
                        .build_conditional_branch(tag_match, field_check_bb, fail_bb)?;
                    self.builder.position_at_end(field_check_bb);

                    // Get concrete field types for this variant (respecting generic substitutions)
                    let variant_fields = self
                        .resolve_enum_variant_fields(enum_name, variant, matched_type.as_ref())
                        .ok_or_else(|| {
                            anyhow!("Variant {} not found in enum {}", variant, enum_name)
                        })?;

                    for (field_idx, field_pattern) in fields.iter().enumerate() {
                        let field_type = variant_fields
                            .get(field_idx)
                            .ok_or_else(|| {
                                anyhow!(
                                    "Field index {} out of bounds for variant {}",
                                    field_idx,
                                    variant
                                )
                            })?
                            .clone();

                        // Use type-specific getter based on field type
                        let get_field_fn_name = match enum_field_kind(&field_type) {
                            EnumFieldKind::Int => "runtime.enum.get_i64",
                            EnumFieldKind::Float => "runtime.enum.get_f64",
                            EnumFieldKind::Bool => "runtime.enum.get_bool",
                            EnumFieldKind::Ptr => "runtime.enum.get_ptr",
                        };
                        let get_field_fn = self.get_or_declare_ffi_function(get_field_fn_name)?;
                        let field_idx_val =
                            self.context.i64_type().const_int(field_idx as u64, false);
                        let field_val = self
                            .builder
                            .build_call(
                                get_field_fn,
                                &[handle.into(), field_idx_val.into()],
                                "field",
                            )?
                            .try_as_basic_value()
                            .left()
                            .unwrap();

                        // Convert to appropriate OtterType
                        let field_otter_type = match enum_field_kind(&field_type) {
                            EnumFieldKind::Int => OtterType::I64,
                            EnumFieldKind::Float => OtterType::F64,
                            EnumFieldKind::Bool => OtterType::Bool,
                            EnumFieldKind::Ptr => OtterType::Opaque,
                        };
                        let field_eval = EvaluatedValue::with_value(field_val, field_otter_type);

                        let next_field_bb = if field_idx < fields.len() - 1 {
                            self.context.append_basic_block(
                                self.builder
                                    .get_insert_block()
                                    .unwrap()
                                    .get_parent()
                                    .unwrap(),
                                &format!("enum_field_check_{}", field_idx + 1),
                            )
                        } else {
                            success_bb
                        };

                        self.compile_pattern_match(
                            field_pattern,
                            &field_eval,
                            Some(field_type.clone()),
                            next_field_bb,
                            fail_bb,
                            ctx,
                        )?;

                        if field_idx < fields.len() - 1 {
                            self.builder.position_at_end(next_field_bb);
                        }
                    }
                }

                Ok(())
            }
            Pattern::Struct {
                name: _struct_name,
                fields,
            } => {
                if fields.is_empty() {
                    self.builder.build_unconditional_branch(success_bb)?;
                } else {
                    bail!("Struct pattern matching with fields not yet fully supported");
                }
                Ok(())
            }
            Pattern::Array { patterns, rest } => {
                let get_len_fn = self.get_or_declare_ffi_function("runtime.list.length")?;
                let handle = matched_val
                    .value
                    .ok_or_else(|| anyhow!("Array value is void"))?;
                let len_val = self
                    .builder
                    .build_call(get_len_fn, &[handle.into()], "len")?
                    .try_as_basic_value()
                    .left()
                    .unwrap()
                    .into_int_value();

                let min_len = patterns.len() as u64;

                let expected_len = self.context.i64_type().const_int(min_len, false);
                let len_check = if rest.is_some() {
                    self.builder.build_int_compare(
                        IntPredicate::UGE,
                        len_val,
                        expected_len,
                        "len_check",
                    )?
                } else {
                    self.builder.build_int_compare(
                        IntPredicate::EQ,
                        len_val,
                        expected_len,
                        "len_check",
                    )?
                };

                let elem_check_bb = self.context.append_basic_block(
                    self.builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap(),
                    "array_elem_check",
                );
                self.builder
                    .build_conditional_branch(len_check, elem_check_bb, fail_bb)?;
                self.builder.position_at_end(elem_check_bb);

                // Track the element type if available so nested patterns get concrete type info
                let element_type = matched_type.as_ref().and_then(|ty| match ty {
                    TypeInfo::List(inner) => Some((**inner).clone()),
                    _ => None,
                });

                for (idx, elem_pattern) in patterns.iter().enumerate() {
                    let get_elem_fn = self.get_or_declare_ffi_function("runtime.list.get")?;
                    let idx_val = self.context.i64_type().const_int(idx as u64, false);
                    let elem_val = self
                        .builder
                        .build_call(get_elem_fn, &[handle.into(), idx_val.into()], "elem")?
                        .try_as_basic_value()
                        .left()
                        .unwrap();

                    let elem_eval = EvaluatedValue::with_value(elem_val, OtterType::Opaque);

                    let next_bb = if idx < patterns.len() - 1 || rest.is_some() {
                        self.context.append_basic_block(
                            self.builder
                                .get_insert_block()
                                .unwrap()
                                .get_parent()
                                .unwrap(),
                            &format!("array_elem_check_{}", idx + 1),
                        )
                    } else {
                        success_bb
                    };

                    self.compile_pattern_match(
                        elem_pattern,
                        &elem_eval,
                        element_type.clone(),
                        next_bb,
                        fail_bb,
                        ctx,
                    )?;

                    if idx < patterns.len() - 1 || rest.is_some() {
                        self.builder.position_at_end(next_bb);
                    }
                }

                if let Some(rest_name) = rest {
                    let function = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
                    let rest_list_type = element_type
                        .as_ref()
                        .and_then(|ty| self.typeinfo_to_otter_type(ty))
                        .unwrap_or_else(OtterType::opaque_list);
                    let alloca = self.create_entry_block_alloca(
                        function,
                        rest_name,
                        rest_list_type.clone(),
                    )?;
                    self.builder.build_store(alloca, handle)?;
                    ctx.insert(
                        rest_name.clone(),
                        crate::codegen::llvm::compiler::types::Variable {
                            ptr: alloca,
                            ty: rest_list_type,
                        },
                    );
                    self.builder.build_unconditional_branch(success_bb)?;
                }

                Ok(())
            }
        }
    }

    fn resolve_enum_variant_fields(
        &self,
        enum_name: &str,
        variant: &str,
        matched_type: Option<&TypeInfo>,
    ) -> Option<Vec<TypeInfo>> {
        if let Some(TypeInfo::Enum { name, args, .. }) = matched_type
            && name == enum_name
            && let Some(layout) = self.enum_layout(enum_name)
        {
            return layout.fields_of(variant, args);
        }

        self.enum_layout(enum_name)
            .and_then(|layout| layout.fields_of(variant, &[]))
    }

    fn build_equality_check(
        &mut self,
        lhs: &EvaluatedValue<'ctx>,
        rhs: &EvaluatedValue<'ctx>,
    ) -> Result<IntValue<'ctx>> {
        let lhs_ty = lhs.ty.clone();
        match lhs_ty {
            OtterType::I64 | OtterType::Bool => {
                let l = lhs.value.unwrap().into_int_value();
                let r = rhs.value.unwrap().into_int_value();
                Ok(self
                    .builder
                    .build_int_compare(IntPredicate::EQ, l, r, "eq")?)
            }
            OtterType::F64 => {
                let l = lhs.value.unwrap().into_float_value();
                let r = rhs.value.unwrap().into_float_value();
                Ok(self
                    .builder
                    .build_float_compare(inkwell::FloatPredicate::OEQ, l, r, "eq")?)
            }
            OtterType::Str => {
                let left_ptr = self.ensure_string_value(lhs.clone())?;
                let right_ptr = self.ensure_string_value(rhs.clone())?;
                let eq_fn = self.get_or_declare_ffi_function("std.strings.equal")?;
                let res_call = self
                    .builder
                    .build_call(eq_fn, &[left_ptr.into(), right_ptr.into()], "str_eq")?
                    .try_as_basic_value()
                    .left()
                    .unwrap();
                // Function returns i32, convert to i1 (bool) for comparison
                let res_i32 = res_call.into_int_value();
                // Check if it's i32 or i64 and convert accordingly
                let zero = if res_i32.get_type().get_bit_width() == 32 {
                    self.context.i32_type().const_zero()
                } else {
                    self.context.i64_type().const_zero()
                };
                let res_bool = self.builder.build_int_compare(
                    IntPredicate::NE,
                    res_i32,
                    zero,
                    "str_eq_bool",
                )?;
                Ok(res_bool)
            }
            _ => bail!("Equality check not implemented for type {:?}", lhs_ty),
        }
    }

    fn lower_block_expression(
        &mut self,
        block: &Node<Block>,
        function: FunctionValue<'ctx>,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        for stmt in &block.as_ref().statements {
            if let Statement::Expr(e) = stmt.as_ref() {
                if std::ptr::eq(stmt, block.as_ref().statements.last().unwrap()) {
                    return self.eval_expr(e.as_ref(), ctx);
                } else {
                    self.eval_expr(e.as_ref(), ctx)?;
                }
            } else {
                self.lower_statement(stmt.as_ref(), function, ctx)?;
            }
        }

        Ok(EvaluatedValue {
            ty: OtterType::Unit,
            value: None,
        })
    }

    fn eval_fstring_expr(
        &mut self,
        expr: &Expr,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        let Expr::FString { parts } = expr else {
            bail!("Expected FString expression")
        };

        let mut result = self.eval_literal(&Literal::String(String::new()), None)?;

        for part in parts {
            let part_val = match part.as_ref() {
                otterc_ast::nodes::FStringPart::Text(s) => {
                    self.eval_literal(&Literal::String(s.clone()), None)?
                }
                otterc_ast::nodes::FStringPart::Expr(e) => self.eval_expr(e.as_ref(), ctx)?,
            };

            result = self.build_string_concat(result, part_val)?;
        }

        Ok(result)
    }

    fn eval_literal(
        &mut self,
        lit: &Literal,
        type_info: Option<&TypeInfo>,
    ) -> Result<EvaluatedValue<'ctx>> {
        match lit {
            Literal::Number(n) => {
                // Use type checker's type information if available
                let inferred_type = if let Some(type_info) = type_info {
                    match type_info {
                        TypeInfo::I64 => OtterType::I64,
                        TypeInfo::I32 => OtterType::I32,
                        TypeInfo::F64 => OtterType::F64,
                        _ => {
                            // Fallback: use the literal's is_float_literal flag or check value
                            let is_float = n.is_float_literal || n.value.fract() != 0.0;
                            if is_float {
                                OtterType::F64
                            } else {
                                OtterType::I64
                            }
                        }
                    }
                } else {
                    // No type info from checker, use the literal's is_float_literal flag or check value
                    let is_float = n.is_float_literal || n.value.fract() != 0.0;
                    if is_float {
                        OtterType::F64
                    } else {
                        OtterType::I64
                    }
                };

                match inferred_type {
                    OtterType::I64 => {
                        // Convert f64 to i64, then to u64 for const_int
                        let i64_val = n.value as i64;
                        let val = self.context.i64_type().const_int(i64_val as u64, false);
                        Ok(EvaluatedValue::with_value(val.into(), OtterType::I64))
                    }
                    OtterType::I32 => {
                        let val = self
                            .context
                            .i32_type()
                            .const_int((n.value as i32) as u64, false);
                        Ok(EvaluatedValue::with_value(val.into(), OtterType::I32))
                    }
                    OtterType::F64 => {
                        let val = self.context.f64_type().const_float(n.value);
                        Ok(EvaluatedValue::with_value(val.into(), OtterType::F64))
                    }
                    _ => {
                        // Fallback to F64 for safety
                        let val = self.context.f64_type().const_float(n.value);
                        Ok(EvaluatedValue::with_value(val.into(), OtterType::F64))
                    }
                }
            }
            Literal::String(s) => {
                let val = self.builder.build_global_string_ptr(s, "str_lit")?;
                Ok(EvaluatedValue::with_value(
                    val.as_pointer_value().into(),
                    OtterType::Str,
                ))
            }
            Literal::Bool(b) => {
                let val = self.context.bool_type().const_int(*b as u64, false);
                Ok(EvaluatedValue::with_value(val.into(), OtterType::Bool))
            }
            // Treat None as Unit for now
            Literal::Unit | Literal::None => Ok(EvaluatedValue {
                ty: OtterType::Unit,
                value: None,
            }),
        }
    }

    fn eval_binary_expr(
        &mut self,
        left: &Expr,
        op: &BinaryOp,
        right: &Expr,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        let lhs = self.eval_expr(left, ctx)?;
        let rhs = self.eval_expr(right, ctx)?;
        let lhs_ty = lhs.ty.clone();
        let rhs_ty = rhs.ty.clone();

        if matches!(op, BinaryOp::Add) && (lhs_ty == OtterType::Str || rhs_ty == OtterType::Str) {
            return self.build_string_concat(lhs, rhs);
        }

        if lhs_ty == OtterType::Str && rhs_ty == OtterType::Str {
            return match op {
                BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Gt
                | BinaryOp::LtEq
                | BinaryOp::GtEq => {
                    let is_equal = self.build_equality_check(&lhs, &rhs)?;
                    match op {
                        BinaryOp::Eq => {
                            Ok(EvaluatedValue::with_value(is_equal.into(), OtterType::Bool))
                        }
                        BinaryOp::Ne => {
                            let not_equal = self.builder.build_not(is_equal, "ne")?;
                            Ok(EvaluatedValue::with_value(
                                not_equal.into(),
                                OtterType::Bool,
                            ))
                        }
                        _ => {
                            let cmp_fn = self.get_or_declare_ffi_function("std.strings.compare")?;
                            let left_ptr = self.ensure_string_value(lhs.clone())?;
                            let right_ptr = self.ensure_string_value(rhs.clone())?;
                            let cmp_result = self
                                .builder
                                .build_call(cmp_fn, &[left_ptr.into(), right_ptr.into()], "strcmp")?
                                .try_as_basic_value()
                                .left()
                                .unwrap()
                                .into_int_value();
                            let zero = self.context.i64_type().const_zero();

                            let result = match op {
                                BinaryOp::Lt => self.builder.build_int_compare(
                                    IntPredicate::SLT,
                                    cmp_result,
                                    zero,
                                    "lt",
                                )?,
                                BinaryOp::Gt => self.builder.build_int_compare(
                                    IntPredicate::SGT,
                                    cmp_result,
                                    zero,
                                    "gt",
                                )?,
                                BinaryOp::LtEq => self.builder.build_int_compare(
                                    IntPredicate::SLE,
                                    cmp_result,
                                    zero,
                                    "le",
                                )?,
                                BinaryOp::GtEq => self.builder.build_int_compare(
                                    IntPredicate::SGE,
                                    cmp_result,
                                    zero,
                                    "ge",
                                )?,
                                _ => unreachable!(),
                            };
                            Ok(EvaluatedValue::with_value(result.into(), OtterType::Bool))
                        }
                    }
                }
                _ => bail!("Unsupported binary operation for strings: {:?}", op),
            };
        }

        // Coerce types if needed - promote to F64 if either operand is F64
        let (lhs_val, rhs_val, result_ty) = if lhs_ty == OtterType::F64 || rhs_ty == OtterType::F64
        {
            // Promote both to F64
            let l_f64 = if lhs_ty == OtterType::F64 {
                lhs.value.unwrap().into_float_value()
            } else if lhs_ty == OtterType::I64 {
                let int_val = lhs.value.unwrap().into_int_value();
                self.builder
                    .build_signed_int_to_float(int_val, self.context.f64_type(), "itof")?
            } else if lhs_ty == OtterType::Opaque {
                let coerced = self.coerce_type(lhs.value.unwrap(), lhs_ty, OtterType::F64)?;
                coerced.into_float_value()
            } else {
                bail!("Cannot coerce {:?} to F64", lhs_ty);
            };

            let r_f64 = if rhs_ty == OtterType::F64 {
                rhs.value.unwrap().into_float_value()
            } else if rhs_ty == OtterType::I64 {
                let int_val = rhs.value.unwrap().into_int_value();
                self.builder
                    .build_signed_int_to_float(int_val, self.context.f64_type(), "itof")?
            } else if rhs_ty == OtterType::Opaque {
                let coerced = self.coerce_type(rhs.value.unwrap(), rhs_ty, OtterType::F64)?;
                coerced.into_float_value()
            } else {
                bail!("Cannot coerce {:?} to F64", rhs_ty);
            };

            (l_f64.into(), r_f64.into(), OtterType::F64)
        } else if lhs_ty == OtterType::I64 && rhs_ty == OtterType::I64 {
            (lhs.value.unwrap(), rhs.value.unwrap(), OtterType::I64)
        } else if lhs_ty == OtterType::Bool && rhs_ty == OtterType::Bool {
            (lhs.value.unwrap(), rhs.value.unwrap(), OtterType::Bool)
        } else {
            bail!(
                "Type mismatch or unsupported types for binary op: {:?} and {:?}",
                lhs_ty,
                rhs_ty
            );
        };

        // Perform the operation based on the result type
        match result_ty {
            OtterType::I64 => {
                let l = lhs_val.into_int_value();
                let r = rhs_val.into_int_value();
                match op {
                    BinaryOp::Add => Ok(EvaluatedValue::with_value(
                        self.builder.build_int_add(l, r, "add")?.into(),
                        OtterType::I64,
                    )),
                    BinaryOp::Sub => Ok(EvaluatedValue::with_value(
                        self.builder.build_int_sub(l, r, "sub")?.into(),
                        OtterType::I64,
                    )),
                    BinaryOp::Mul => Ok(EvaluatedValue::with_value(
                        self.builder.build_int_mul(l, r, "mul")?.into(),
                        OtterType::I64,
                    )),
                    BinaryOp::Div => Ok(EvaluatedValue::with_value(
                        self.builder.build_int_signed_div(l, r, "div")?.into(),
                        OtterType::I64,
                    )),
                    BinaryOp::Eq => Ok(EvaluatedValue::with_value(
                        self.builder
                            .build_int_compare(IntPredicate::EQ, l, r, "eq")?
                            .into(),
                        OtterType::Bool,
                    )),
                    BinaryOp::Ne => Ok(EvaluatedValue::with_value(
                        self.builder
                            .build_int_compare(IntPredicate::NE, l, r, "ne")?
                            .into(),
                        OtterType::Bool,
                    )),
                    BinaryOp::Lt => Ok(EvaluatedValue::with_value(
                        self.builder
                            .build_int_compare(IntPredicate::SLT, l, r, "lt")?
                            .into(),
                        OtterType::Bool,
                    )),
                    BinaryOp::Gt => Ok(EvaluatedValue::with_value(
                        self.builder
                            .build_int_compare(IntPredicate::SGT, l, r, "gt")?
                            .into(),
                        OtterType::Bool,
                    )),
                    BinaryOp::LtEq => Ok(EvaluatedValue::with_value(
                        self.builder
                            .build_int_compare(IntPredicate::SLE, l, r, "le")?
                            .into(),
                        OtterType::Bool,
                    )),
                    BinaryOp::GtEq => Ok(EvaluatedValue::with_value(
                        self.builder
                            .build_int_compare(IntPredicate::SGE, l, r, "ge")?
                            .into(),
                        OtterType::Bool,
                    )),
                    _ => bail!("Unsupported binary op for I64"),
                }
            }
            OtterType::F64 => {
                let l = lhs_val.into_float_value();
                let r = rhs_val.into_float_value();
                match op {
                    BinaryOp::Add => Ok(EvaluatedValue::with_value(
                        self.builder.build_float_add(l, r, "add")?.into(),
                        OtterType::F64,
                    )),
                    BinaryOp::Sub => Ok(EvaluatedValue::with_value(
                        self.builder.build_float_sub(l, r, "sub")?.into(),
                        OtterType::F64,
                    )),
                    BinaryOp::Mul => Ok(EvaluatedValue::with_value(
                        self.builder.build_float_mul(l, r, "mul")?.into(),
                        OtterType::F64,
                    )),
                    BinaryOp::Div => Ok(EvaluatedValue::with_value(
                        self.builder.build_float_div(l, r, "div")?.into(),
                        OtterType::F64,
                    )),
                    BinaryOp::Eq => Ok(EvaluatedValue::with_value(
                        self.builder
                            .build_float_compare(inkwell::FloatPredicate::OEQ, l, r, "eq")?
                            .into(),
                        OtterType::Bool,
                    )),
                    BinaryOp::Ne => Ok(EvaluatedValue::with_value(
                        self.builder
                            .build_float_compare(inkwell::FloatPredicate::ONE, l, r, "ne")?
                            .into(),
                        OtterType::Bool,
                    )),
                    BinaryOp::Lt => Ok(EvaluatedValue::with_value(
                        self.builder
                            .build_float_compare(inkwell::FloatPredicate::OLT, l, r, "lt")?
                            .into(),
                        OtterType::Bool,
                    )),
                    BinaryOp::Gt => Ok(EvaluatedValue::with_value(
                        self.builder
                            .build_float_compare(inkwell::FloatPredicate::OGT, l, r, "gt")?
                            .into(),
                        OtterType::Bool,
                    )),
                    BinaryOp::LtEq => Ok(EvaluatedValue::with_value(
                        self.builder
                            .build_float_compare(inkwell::FloatPredicate::OLE, l, r, "le")?
                            .into(),
                        OtterType::Bool,
                    )),
                    BinaryOp::GtEq => Ok(EvaluatedValue::with_value(
                        self.builder
                            .build_float_compare(inkwell::FloatPredicate::OGE, l, r, "ge")?
                            .into(),
                        OtterType::Bool,
                    )),
                    _ => bail!("Unsupported binary op for F64"),
                }
            }
            _ => bail!("Unsupported type for binary operation"),
        }
    }

    fn eval_unary_expr(
        &mut self,
        op: &UnaryOp,
        expr: &Expr,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        let val = self.eval_expr(expr, ctx)?;
        match op {
            UnaryOp::Neg => {
                let val_ty = val.ty.clone();
                if val_ty == OtterType::I64 {
                    let v = val.value.unwrap().into_int_value();
                    Ok(EvaluatedValue::with_value(
                        self.builder.build_int_neg(v, "neg")?.into(),
                        OtterType::I64,
                    ))
                } else if val_ty == OtterType::F64 {
                    let v = val.value.unwrap().into_float_value();
                    Ok(EvaluatedValue::with_value(
                        self.builder.build_float_neg(v, "neg")?.into(),
                        OtterType::F64,
                    ))
                } else {
                    bail!("Unsupported type for negation");
                }
            }
            UnaryOp::Not => {
                let val_ty = val.ty.clone();
                if val_ty == OtterType::Bool {
                    let v = val.value.unwrap().into_int_value();
                    Ok(EvaluatedValue::with_value(
                        self.builder.build_not(v, "not")?.into(),
                        OtterType::Bool,
                    ))
                } else {
                    bail!("Unsupported type for not");
                }
            }
        }
    }

    pub(crate) fn basic_type(&self, ty: OtterType) -> Result<Option<BasicTypeEnum<'ctx>>> {
        match ty {
            OtterType::Unit => Ok(None),
            OtterType::Bool => Ok(Some(self.context.bool_type().into())),
            OtterType::I32 => Ok(Some(self.context.i32_type().into())),
            OtterType::I64 | OtterType::Opaque | OtterType::List(_) | OtterType::Map => {
                Ok(Some(self.context.i64_type().into()))
            }
            OtterType::F64 => Ok(Some(self.context.f64_type().into())),
            OtterType::Str => Ok(Some(self.string_ptr_type.into())),
            OtterType::Struct(id) => Ok(Some(self.struct_info(id).ty.into())),
            OtterType::Tuple(fields) => {
                let mut llvm_fields = Vec::with_capacity(fields.len());
                for field in fields {
                    let llvm_ty = match self.basic_type(field)? {
                        Some(ty) => ty,
                        None => self.context.i8_type().into(),
                    };
                    llvm_fields.push(llvm_ty);
                }
                Ok(Some(self.context.struct_type(&llvm_fields, false).into()))
            }
        }
    }

    pub(crate) fn to_bool_value(&self, val: EvaluatedValue<'ctx>) -> Result<IntValue<'ctx>> {
        let EvaluatedValue { ty, value } = val;
        if ty == OtterType::Bool {
            Ok(value.unwrap().into_int_value())
        } else {
            bail!("Expected boolean value")
        }
    }

    pub(crate) fn coerce_type(
        &self,
        value: inkwell::values::BasicValueEnum<'ctx>,
        from_ty: OtterType,
        to_ty: OtterType,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>> {
        // If types match, no coercion needed
        if from_ty == to_ty {
            return Ok(value);
        }

        if matches!((&from_ty, &to_ty), (OtterType::List(_), OtterType::List(_))) {
            return Ok(value);
        }

        // Perform type coercion based on source and target types
        match (from_ty.clone(), to_ty.clone()) {
            // Numeric conversions
            (OtterType::I32, OtterType::I64) => {
                let int_val = value.into_int_value();
                Ok(self
                    .builder
                    .build_int_s_extend(int_val, self.context.i64_type(), "i32_to_i64")?
                    .into())
            }
            (OtterType::I64, OtterType::I32) => {
                let int_val = value.into_int_value();
                Ok(self
                    .builder
                    .build_int_truncate(int_val, self.context.i32_type(), "i64_to_i32")?
                    .into())
            }
            (OtterType::I32, OtterType::F64) | (OtterType::I64, OtterType::F64) => {
                let int_val = value.into_int_value();
                Ok(self
                    .builder
                    .build_signed_int_to_float(int_val, self.context.f64_type(), "int_to_f64")?
                    .into())
            }
            (OtterType::F64, OtterType::I32) => {
                let float_val = value.into_float_value();
                Ok(self
                    .builder
                    .build_float_to_signed_int(float_val, self.context.i32_type(), "f64_to_i32")?
                    .into())
            }
            (OtterType::F64, OtterType::I64) => {
                let float_val = value.into_float_value();
                Ok(self
                    .builder
                    .build_float_to_signed_int(float_val, self.context.i64_type(), "f64_to_i64")?
                    .into())
            }

            // Bool conversions
            (OtterType::Bool, OtterType::I32) => {
                let bool_val = value.into_int_value();
                Ok(self
                    .builder
                    .build_int_z_extend(bool_val, self.context.i32_type(), "bool_to_i32")?
                    .into())
            }
            (OtterType::Bool, OtterType::I64) => {
                let bool_val = value.into_int_value();
                Ok(self
                    .builder
                    .build_int_z_extend(bool_val, self.context.i64_type(), "bool_to_i64")?
                    .into())
            }
            (OtterType::I32 | OtterType::I64, OtterType::Bool) => {
                let int_val = value.into_int_value();
                let zero = int_val.get_type().const_zero();
                Ok(self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::NE, int_val, zero, "int_to_bool")?
                    .into())
            }

            // Opaque type conversions (treat as i64)
            (OtterType::Opaque, OtterType::I64) | (OtterType::I64, OtterType::Opaque)
            // List/Map conversions (treat as opaque pointers)
            | (OtterType::List(_) | OtterType::Map, OtterType::Opaque)
            | (OtterType::Opaque, OtterType::List(_) | OtterType::Map) => {
                Ok(value) // Already same representation
            }
            (OtterType::F64, OtterType::Opaque) => {
                let float_val = value.into_float_value();
                Ok(self.builder.build_bit_cast(
                    float_val,
                    self.context.i64_type(),
                    "f64_to_opaque",
                )?)
            }
            (OtterType::Opaque, OtterType::F64) => {
                let int_val = value.into_int_value();
                Ok(self.builder.build_bit_cast(
                    int_val,
                    self.context.f64_type(),
                    "opaque_to_f64",
                )?)
            }
            // Tuple to Struct coercion (for FFI compatibility)
            (OtterType::Tuple(_), OtterType::Struct(_)) => {
                // Cannot bitcast aggregates directly. Must go through memory.
                if let Some(target_llvm_ty) = self.basic_type(to_ty)? {
                    // Create stack slot for source value
                    let src_ty = value.get_type();
                    let alloca = self.builder.build_alloca(src_ty, "coercion_slot")?;
                    self.builder.build_store(alloca, value)?;

                    // Cast pointer to target type
                    let target_ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
                    // In opaque pointer world (LLVM 15+), pointer casts are no-ops or just type changes
                    // But we might need to be explicit if using typed pointers or for clarity
                    // Actually, build_bit_cast on pointer works
                    let cast_ptr = self
                        .builder
                        .build_bit_cast(alloca, target_ptr_ty, "coercion_cast")?
                        .into_pointer_value();

                    // Load as target type
                    Ok(self
                        .builder
                        .build_load(target_llvm_ty, cast_ptr, "coerced_struct")?)
                } else {
                    bail!("Cannot resolve target struct type for coercion")
                }
            }
            (OtterType::Bool, OtterType::Opaque) => {
                let bool_val = value.into_int_value();
                Ok(self
                    .builder
                    .build_int_z_extend(bool_val, self.context.i64_type(), "bool_to_opaque")?
                    .into())
            }
            (OtterType::Opaque, OtterType::Bool) => {
                let int_val = value.into_int_value();
                let zero = int_val.get_type().const_zero();
                Ok(self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::NE, int_val, zero, "opaque_to_bool")?
                    .into())
            }

            // Incompatible types
            _ => {
                bail!("Cannot coerce type {:?} to {:?}", from_ty, to_ty)
            }
        }
    }

    fn cast_argument_for_call(
        &self,
        value: BasicValueEnum<'ctx>,
        from_ty: OtterType,
        param_type: &BasicTypeEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>> {
        if from_ty == OtterType::F64 && param_type.is_int_type() {
            let float_val = value.into_float_value();
            Ok(self
                .builder
                .build_float_to_signed_int(float_val, param_type.into_int_type(), "ftoi")?
                .into())
        } else if value.get_type() != *param_type
            && value.get_type().is_struct_type()
            && param_type.is_struct_type()
        {
            // Repack aggregates whose LLVM struct names don't match (e.g., named Otter structs
            // passed to anonymous FFI structs).
            let current_function = self
                .builder
                .get_insert_block()
                .and_then(|bb| bb.get_parent())
                .ok_or_else(|| anyhow!("Cannot determine current function for argument cast"))?;
            let tmp = self.create_entry_block_alloca(current_function, "struct_cast", from_ty)?;
            self.builder.build_store(tmp, value)?;
            let generic_ptr = self.raw_ptr_type();
            let cast_ptr = self
                .builder
                .build_bit_cast(tmp, generic_ptr, "struct_cast_ptr")?
                .into_pointer_value();
            let loaded = self
                .builder
                .build_load(*param_type, cast_ptr, "struct_cast_load")?;
            Ok(loaded)
        } else {
            Ok(value)
        }
    }

    fn eval_call_expr(
        &mut self,
        expr: &Expr,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        if let Expr::Call { func, args } = expr {
            let mut implicit_self: Option<EvaluatedValue<'ctx>> = None;
            if let Some(enum_value) =
                self.try_build_enum_constructor(expr, func.as_ref().as_ref(), args, ctx)?
            {
                return Ok(enum_value);
            }

            // Evaluate function expression
            let func_name = match func.as_ref().as_ref() {
                Expr::Identifier(name) => name.clone(),
                Expr::Member { object, field } => {
                    // First, try to evaluate the object to check its runtime type
                    // This handles cases like list.append() where the object is a variable
                    if let Ok(evaluated) = self.eval_expr(object.as_ref().as_ref(), ctx) {
                        if evaluated.value.is_some() {
                            // Check if it's a list type and handle list methods
                            if matches!(evaluated.ty, OtterType::List(_)) {
                                if field == "append" && !args.is_empty() {
                                    // Determine the append function based on argument type
                                    let arg_val = self.eval_expr(args[0].as_ref(), ctx)?;
                                    let method_name: String = match arg_val.ty {
                                        OtterType::Str => "append<list,string>".to_string(),
                                        OtterType::I64 | OtterType::I32 => {
                                            "append<list,int>".to_string()
                                        }
                                        OtterType::F64 => "append<list,float>".to_string(),
                                        OtterType::Bool => "append<list,bool>".to_string(),
                                        OtterType::List(_) | OtterType::Opaque => {
                                            "append<list,list>".to_string()
                                        }
                                        _ => bail!(
                                            "unsupported append argument type: {:?}",
                                            arg_val.ty
                                        ),
                                    };
                                    implicit_self = Some(evaluated);
                                    method_name
                                } else {
                                    bail!(
                                        "list method '{}' not supported or missing arguments",
                                        field
                                    );
                                }
                            } else if let OtterType::Struct(struct_id) = evaluated.ty.clone() {
                                if let Some(method_name) =
                                    self.resolve_struct_method_name(struct_id, field)
                                {
                                    implicit_self = Some(evaluated);
                                    method_name
                                } else {
                                    bail!(
                                        "struct method '{}.{}' not found",
                                        self.struct_info(struct_id).name,
                                        field
                                    );
                                }
                            } else {
                                // Not a list or struct, continue with other checks
                                if let Expr::Identifier(enum_name) = object.as_ref().as_ref() {
                                    if let Some(enum_value) = self
                                        .try_build_enum_from_member(enum_name, field, args, ctx)?
                                    {
                                        return Ok(enum_value);
                                    }
                                    format!("{}.{}", enum_name, field)
                                } else {
                                    bail!("Complex member expressions not yet supported");
                                }
                            }
                        } else {
                            // Value is None, try enum constructor path
                            if let Expr::Identifier(enum_name) = object.as_ref().as_ref() {
                                if let Some(enum_value) =
                                    self.try_build_enum_from_member(enum_name, field, args, ctx)?
                                {
                                    return Ok(enum_value);
                                }
                                format!("{}.{}", enum_name, field)
                            } else {
                                bail!("cannot call member '{}' without value", field);
                            }
                        }
                    } else if let Expr::Identifier(enum_name) = object.as_ref().as_ref() {
                        // Fallback: try enum constructor
                        if let Some(enum_value) =
                            self.try_build_enum_from_member(enum_name, field, args, ctx)?
                        {
                            return Ok(enum_value);
                        }
                        format!("{}.{}", enum_name, field)
                    } else if let Some(OtterType::Struct(struct_id)) =
                        self.struct_type_from_expr(object.as_ref().as_ref())
                    {
                        let self_value = self.eval_expr(object.as_ref().as_ref(), ctx)?;
                        if self_value.value.is_none() {
                            bail!("cannot call method '{}' without value", field);
                        }
                        if let Some(method_name) = self.resolve_struct_method_name(struct_id, field)
                        {
                            implicit_self = Some(self_value);
                            method_name
                        } else {
                            bail!(
                                "struct method '{}.{}' not found",
                                self.struct_info(struct_id).name,
                                field
                            );
                        }
                    } else {
                        // Evaluate the object first to check its runtime type
                        let evaluated = self.eval_expr(object.as_ref().as_ref(), ctx)?;
                        if evaluated.value.is_none() {
                            bail!("cannot call member '{}' without value", field);
                        }

                        // Check if it's a list type and handle list methods
                        if matches!(evaluated.ty, OtterType::List(_)) {
                            // Handle list method calls like list.append()
                            if field == "append" && !args.is_empty() {
                                // Determine the append function based on argument type
                                let arg_val = self.eval_expr(args[0].as_ref(), ctx)?;
                                let method_name: String = match arg_val.ty {
                                    OtterType::Str => "append<list,string>".to_string(),
                                    OtterType::I64 | OtterType::I32 => {
                                        "append<list,int>".to_string()
                                    }
                                    OtterType::F64 => "append<list,float>".to_string(),
                                    OtterType::Bool => "append<list,bool>".to_string(),
                                    OtterType::List(_) | OtterType::Opaque => {
                                        "append<list,list>".to_string()
                                    }
                                    _ => {
                                        bail!("unsupported append argument type: {:?}", arg_val.ty)
                                    }
                                };
                                implicit_self = Some(evaluated);
                                method_name
                            } else {
                                bail!("list method '{}' not supported or missing arguments", field);
                            }
                        } else if let OtterType::Struct(struct_id) = evaluated.ty.clone() {
                            if let Some(method_name) =
                                self.resolve_struct_method_name(struct_id, field)
                            {
                                implicit_self = Some(evaluated);
                                method_name
                            } else {
                                bail!(
                                    "struct method '{}.{}' not found",
                                    self.struct_info(struct_id).name,
                                    field
                                );
                            }
                        } else if let Some(func_name) =
                            self.resolve_member_function_name(object.as_ref().as_ref(), field)
                        {
                            func_name
                        } else {
                            bail!("Complex member expressions not yet supported (call)");
                        }
                    }
                }
                _ => bail!("Complex function expressions not yet supported"),
            };

            // Handle overloaded builtins like len() - evaluate first arg to determine type
            let (function, resolved_func_name, first_arg_evaluated) =
                if func_name == "len" && !args.is_empty() {
                    // Evaluate the first argument to determine its type
                    let arg_val = self.eval_expr(args[0].as_ref(), ctx)?;
                    let overloaded_name = match arg_val.ty {
                        OtterType::Str => "len".to_string(),
                        OtterType::List(_) => "len<list>".to_string(),
                        OtterType::Map => "len<map>".to_string(),
                        _ => bail!("len() not supported for type {:?}", arg_val.ty),
                    };
                    if self.symbol_registry.contains(&overloaded_name) {
                        (
                            self.get_or_declare_ffi_function(&overloaded_name)?,
                            overloaded_name,
                            Some(arg_val),
                        )
                    } else {
                        bail!("Function {} not found", overloaded_name);
                    }
                } else if let Some(func) = self.declared_functions.get(&func_name) {
                    (*func, func_name.clone(), None)
                } else if self.symbol_registry.contains(&func_name) {
                    (
                        self.get_or_declare_ffi_function(&func_name)?,
                        func_name.clone(),
                        None,
                    )
                } else {
                    bail!("Function {} not found", func_name);
                };

            // Get parameter types upfront to avoid borrow issues
            let param_types: Vec<BasicTypeEnum> = function
                .get_param_iter()
                .map(|arg| arg.get_type())
                .collect();

            // Evaluate arguments and convert types as needed
            let mut arg_values: Vec<BasicMetadataValueEnum> = Vec::new();
            let mut param_offset = 0;

            if let Some(self_arg) = implicit_self {
                let v = self_arg
                    .value
                    .ok_or_else(|| anyhow!("Cannot pass unit value as self"))?;
                let param_type = param_types.first().ok_or_else(|| {
                    anyhow!("Method '{}' missing self parameter", resolved_func_name)
                })?;
                let converted = self.cast_argument_for_call(v, self_arg.ty.clone(), param_type)?;
                arg_values.push(converted.into());
                param_offset = 1;
            }

            for (i, arg) in args.iter().enumerate() {
                // Reuse first arg if it was already evaluated for len() dispatch
                let arg_val = if i == 0 {
                    if let Some(val) = first_arg_evaluated.as_ref() {
                        val.clone()
                    } else {
                        self.eval_expr(arg.as_ref(), ctx)?
                    }
                } else {
                    self.eval_expr(arg.as_ref(), ctx)?
                };
                if let Some(v) = arg_val.value {
                    let param_type = param_types.get(i + param_offset).ok_or_else(|| {
                        anyhow!("Too many arguments for function {}", resolved_func_name)
                    })?;
                    let converted =
                        self.cast_argument_for_call(v, arg_val.ty.clone(), param_type)?;
                    arg_values.push(converted.into());
                } else {
                    bail!("Cannot pass unit value as argument");
                }
            }

            // Fill in default values for missing arguments
            if arg_values.len() < param_types.len() {
                let defaults_to_eval =
                    if let Some(defaults) = self.function_defaults.get(&resolved_func_name) {
                        let mut to_eval = Vec::new();
                        for i in arg_values.len()..param_types.len() {
                            if let Some(default_expr) = defaults.get(i).and_then(|d| d.as_ref()) {
                                to_eval.push((i, default_expr.clone()));
                            } else {
                                bail!("Missing argument {} for function {}", i, resolved_func_name);
                            }
                        }
                        to_eval
                    } else {
                        bail!("Missing arguments for function {}", resolved_func_name);
                    };

                for (i, default_expr) in defaults_to_eval {
                    let val = self.eval_expr(&default_expr, ctx)?;
                    if let Some(v) = val.value {
                        let param_type = param_types[i];
                        let converted = self.cast_argument_for_call(v, val.ty, &param_type)?;
                        arg_values.push(converted.into());
                    } else {
                        bail!("Default value for argument {} evaluated to void", i);
                    }
                }
            }

            // Call the function
            let call_site = self.builder.build_call(function, &arg_values, &func_name)?;

            // Get return value
            if let Some(ret_val) = call_site.try_as_basic_value().left() {
                // Use declared return type if available, otherwise infer from LLVM type
                let return_ty = self
                    .function_return_types
                    .get(&resolved_func_name)
                    .cloned()
                    .unwrap_or_else(|| {
                        function
                            .get_type()
                            .get_return_type()
                            .map(|ty| self.otter_type_from_basic_type(ty))
                            .unwrap_or(OtterType::Opaque)
                    });
                Ok(EvaluatedValue::with_value(ret_val, return_ty))
            } else {
                // Function returns void
                Ok(EvaluatedValue {
                    ty: OtterType::Unit,
                    value: None,
                })
            }
        } else {
            bail!("Expected Call expression");
        }
    }

    fn eval_if_expr(
        &mut self,
        expr: &Expr,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        if let Expr::If {
            cond,
            then_branch,
            else_branch,
        } = expr
        {
            // Evaluate condition
            let cond_val = self.eval_expr(cond.as_ref().as_ref(), ctx)?;
            let cond_bool = self.to_bool_value(cond_val)?;

            // Get current function
            let function = self
                .builder
                .get_insert_block()
                .and_then(|bb| bb.get_parent())
                .ok_or_else(|| anyhow!("No parent function"))?;

            // Create basic blocks
            let then_bb = self.context.append_basic_block(function, "then");
            let else_bb = self.context.append_basic_block(function, "else");
            let merge_bb = self.context.append_basic_block(function, "merge");

            // Branch based on condition
            self.builder
                .build_conditional_branch(cond_bool, then_bb, else_bb)?;

            // Build then branch
            self.builder.position_at_end(then_bb);
            let then_val = self.eval_expr(then_branch.as_ref().as_ref(), ctx)?;
            let then_bb_end = self.builder.get_insert_block().unwrap();

            // Only add branch if block doesn't already terminate
            if then_bb_end.get_terminator().is_none() {
                self.builder.build_unconditional_branch(merge_bb)?;
            }

            // Build else branch
            self.builder.position_at_end(else_bb);
            let else_val = if let Some(else_br) = else_branch {
                self.eval_expr(else_br.as_ref().as_ref(), ctx)?
            } else {
                // No else branch - return unit
                EvaluatedValue {
                    ty: OtterType::Unit,
                    value: None,
                }
            };
            let else_bb_end = self.builder.get_insert_block().unwrap();

            // Only add branch if block doesn't already terminate
            if else_bb_end.get_terminator().is_none() {
                self.builder.build_unconditional_branch(merge_bb)?;
            }

            // Position at merge block
            self.builder.position_at_end(merge_bb);

            // If both branches return the same type and have values, create a phi node
            let then_ty = then_val.ty.clone();
            let else_ty = else_val.ty.clone();

            if then_ty == else_ty && then_val.value.is_some() && else_val.value.is_some() {
                if let Some(basic_ty) = self.basic_type(then_ty.clone())? {
                    let phi = self.builder.build_phi(basic_ty, "if_result")?;
                    phi.add_incoming(&[
                        (&then_val.value.unwrap(), then_bb_end),
                        (&else_val.value.unwrap(), else_bb_end),
                    ]);
                    Ok(EvaluatedValue::with_value(phi.as_basic_value(), then_ty))
                } else {
                    // Unit type
                    Ok(EvaluatedValue {
                        ty: OtterType::Unit,
                        value: None,
                    })
                }
            } else {
                // Different types or unit - return unit
                Ok(EvaluatedValue {
                    ty: OtterType::Unit,
                    value: None,
                })
            }
        } else {
            bail!("Expected If expression");
        }
    }

    fn build_string_concat(
        &mut self,
        lhs: EvaluatedValue<'ctx>,
        rhs: EvaluatedValue<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        let left_ptr = self.ensure_string_value(lhs)?;
        let right_ptr = self.ensure_string_value(rhs)?;
        let result = self.call_ffi_returning_value(
            "std.strings.concat",
            vec![left_ptr, right_ptr],
            "str_concat",
        )?;
        Ok(EvaluatedValue::with_value(result, OtterType::Str))
    }

    fn eval_array_expr(
        &mut self,
        elements: &[Node<Expr>],
        expr_type: Option<&TypeInfo>,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        // Create a new empty list
        let create_fn = self.get_or_declare_ffi_function("list.new")?;
        let handle = self
            .builder
            .build_call(create_fn, &[], "list_handle")?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| anyhow!("list creation returned void"))?
            .into_int_value();

        // Append each element to the list
        for (idx, elem) in elements.iter().enumerate() {
            let elem_val = self.eval_expr(elem.as_ref(), ctx)?;
            let elem_value = elem_val
                .value
                .ok_or_else(|| anyhow!("array element {} produced no value", idx))?;
            self.append_value_to_list(handle, elem_value, elem_val.ty, &format!("append_{}", idx))?;
        }

        let list_ty = expr_type
            .and_then(|ty| self.typeinfo_to_otter_type(ty))
            .unwrap_or_else(OtterType::opaque_list);
        Ok(EvaluatedValue::with_value(handle.into(), list_ty))
    }

    fn append_value_to_list(
        &mut self,
        list_handle: IntValue<'ctx>,
        value: BasicValueEnum<'ctx>,
        value_ty: OtterType,
        label: &str,
    ) -> Result<()> {
        let (append_fn_name, expected_ty) = self.list_append_target(&value_ty)?;
        let append_fn = self.get_or_declare_ffi_function(append_fn_name)?;
        let coerced_value = self.coerce_type(value, value_ty, expected_ty)?;
        let handle_arg: BasicMetadataValueEnum = list_handle.into();
        let value_arg: BasicMetadataValueEnum = coerced_value.into();
        self.builder
            .build_call(append_fn, &[handle_arg, value_arg], label)?;
        Ok(())
    }

    fn list_append_target(&self, ty: &OtterType) -> Result<(&'static str, OtterType)> {
        match ty {
            OtterType::Str => Ok(("append<list,string>", OtterType::Str)),
            OtterType::I32 | OtterType::I64 => Ok(("append<list,int>", OtterType::I64)),
            OtterType::F64 => Ok(("append<list,float>", OtterType::F64)),
            OtterType::Bool => Ok(("append<list,bool>", OtterType::Bool)),
            OtterType::List(_) => Ok(("append<list,list>", ty.clone())),
            OtterType::Map => Ok(("append<list,map>", OtterType::Map)),
            OtterType::Opaque => Ok(("append<list,list>", OtterType::opaque_list())),
            _ => bail!("unsupported array element type: {:?}", ty),
        }
    }

    fn eval_list_comprehension(
        &mut self,
        full_expr: &Expr,
        element: &Expr,
        var: &str,
        iterable: &Expr,
        condition: Option<&Expr>,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        let list_fn = self.get_or_declare_ffi_function("list.new")?;
        let result_handle = self
            .builder
            .build_call(list_fn, &[], "list_comp_result")?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| anyhow!("list comprehension failed to create list"))?
            .into_int_value();

        let previous_binding = ctx.remove(var);
        let mut inserted_new_binding = false;

        let element_ty = self
            .comprehension_element_type(full_expr)
            .or_else(|| self.list_element_type(iterable))
            .or_else(|| self.infer_comprehension_var_type(var, element, condition))
            .unwrap_or(OtterType::Opaque);
        let list_ty_from_element = OtterType::list_of(element_ty.clone());

        let result_list_ty = self
            .expr_type(full_expr)
            .and_then(|ty| self.typeinfo_to_otter_type(ty))
            .unwrap_or_else(|| list_ty_from_element.clone());

        let result = (|| -> Result<EvaluatedValue<'ctx>> {
            let iterable_val = self.eval_expr(iterable, ctx)?;
            if !matches!(iterable_val.ty, OtterType::List(_)) {
                bail!(
                    "list comprehension expects list iterable, got {:?}",
                    iterable_val.ty
                );
            }
            let iterable_handle = iterable_val
                .value
                .ok_or_else(|| anyhow!("list comprehension iterable produced no value"))?;

            let iter_create_fn = self.get_or_declare_ffi_function("__otter_iter_array")?;
            let iter_has_next_fn =
                self.get_or_declare_ffi_function("__otter_iter_has_next_array")?;
            let iter_next_fn = self.get_or_declare_ffi_function("__otter_iter_next_array")?;
            let iter_free_fn = self.get_or_declare_ffi_function("__otter_iter_free_array")?;

            let iter_handle = self
                .builder
                .build_call(iter_create_fn, &[iterable_handle.into()], "list_comp_iter")?
                .try_as_basic_value()
                .left()
                .ok_or_else(|| anyhow!("iterator creation failed"))?;

            let function = self
                .builder
                .get_insert_block()
                .and_then(|b| b.get_parent())
                .ok_or_else(|| anyhow!("no active function for list comprehension"))?;

            let var_alloca = self.create_entry_block_alloca(function, var, element_ty.clone())?;
            ctx.insert(
                var.to_string(),
                Variable {
                    ptr: var_alloca,
                    ty: element_ty.clone(),
                },
            );
            inserted_new_binding = true;

            let loop_cond_bb = self.context.append_basic_block(function, "listcomp_cond");
            let loop_body_bb = self.context.append_basic_block(function, "listcomp_body");
            let loop_cleanup_bb = self
                .context
                .append_basic_block(function, "listcomp_cleanup");
            let loop_exit_bb = self.context.append_basic_block(function, "listcomp_exit");

            self.builder.build_unconditional_branch(loop_cond_bb)?;

            self.builder.position_at_end(loop_cond_bb);
            let has_next_call = self.builder.build_call(
                iter_has_next_fn,
                &[iter_handle.into()],
                "listcomp_has_next",
            )?;
            let has_next = has_next_call
                .try_as_basic_value()
                .left()
                .ok_or_else(|| anyhow!("iterator has_next returned void"))?
                .into_int_value();
            self.builder
                .build_conditional_branch(has_next, loop_body_bb, loop_cleanup_bb)?;

            self.builder.position_at_end(loop_body_bb);
            let next_call =
                self.builder
                    .build_call(iter_next_fn, &[iter_handle.into()], "listcomp_next")?;
            let element_val = next_call
                .try_as_basic_value()
                .left()
                .ok_or_else(|| anyhow!("iterator next returned void"))?;
            let decoded = self.decode_and_convert_tagged_value(element_val, &element_ty)?;
            if let Some(value) = decoded {
                self.builder.build_store(var_alloca, value)?;
            }

            let loop_continue_bb = self
                .context
                .append_basic_block(function, "listcomp_continue");

            if let Some(cond_expr) = condition {
                let cond_val = self.eval_expr(cond_expr, ctx)?;
                let cond_bool = self.to_bool_value(cond_val)?;
                let append_bb = self.context.append_basic_block(function, "listcomp_append");
                let skip_bb = self.context.append_basic_block(function, "listcomp_skip");
                self.builder
                    .build_conditional_branch(cond_bool, append_bb, skip_bb)?;

                self.builder.position_at_end(append_bb);
                self.emit_list_comprehension_append(result_handle, element, ctx)?;
                self.builder.build_unconditional_branch(loop_continue_bb)?;

                self.builder.position_at_end(skip_bb);
                self.builder.build_unconditional_branch(loop_continue_bb)?;
            } else {
                self.emit_list_comprehension_append(result_handle, element, ctx)?;
                self.builder.build_unconditional_branch(loop_continue_bb)?;
            }

            self.builder.position_at_end(loop_continue_bb);
            self.builder.build_unconditional_branch(loop_cond_bb)?;

            self.builder.position_at_end(loop_cleanup_bb);
            self.builder
                .build_call(iter_free_fn, &[iter_handle.into()], "listcomp_free")?;
            self.builder.build_unconditional_branch(loop_exit_bb)?;

            self.builder.position_at_end(loop_exit_bb);

            Ok(EvaluatedValue::with_value(
                result_handle.into(),
                result_list_ty.clone(),
            ))
        })();

        if inserted_new_binding {
            ctx.remove(var);
        }
        if let Some(prev) = previous_binding {
            ctx.insert(var.to_string(), prev);
        }

        result
    }

    fn emit_list_comprehension_append(
        &mut self,
        result_handle: IntValue<'ctx>,
        element: &Expr,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<()> {
        let elem_val = self.eval_expr(element, ctx)?;
        let elem_value = elem_val
            .value
            .ok_or_else(|| anyhow!("list comprehension element produced no value"))?;
        self.append_value_to_list(
            result_handle,
            elem_value,
            elem_val.ty,
            "listcomp_append_call",
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "TODO: Create a struct to hold these args"
    )]
    fn eval_dict_comprehension(
        &mut self,
        full_expr: &Expr,
        key: &Expr,
        value: &Expr,
        var: &str,
        iterable: &Expr,
        condition: Option<&Expr>,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        let map_fn = self.get_or_declare_ffi_function("map.new")?;
        let map_handle = self
            .builder
            .build_call(map_fn, &[], "dict_comp_result")?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| anyhow!("dict comprehension failed to create map"))?
            .into_int_value();

        let previous_binding = ctx.remove(var);
        let mut inserted_new_binding = false;

        let result = (|| -> Result<EvaluatedValue<'ctx>> {
            let iterable_val = self.eval_expr(iterable, ctx)?;
            if !matches!(iterable_val.ty, OtterType::List(_)) {
                bail!(
                    "dict comprehension expects list iterable, got {:?}",
                    iterable_val.ty
                );
            }

            let iterable_handle = iterable_val
                .value
                .ok_or_else(|| anyhow!("dict comprehension iterable produced no value"))?;

            let iter_create_fn = self.get_or_declare_ffi_function("__otter_iter_array")?;
            let iter_has_next_fn =
                self.get_or_declare_ffi_function("__otter_iter_has_next_array")?;
            let iter_next_fn = self.get_or_declare_ffi_function("__otter_iter_next_array")?;
            let iter_free_fn = self.get_or_declare_ffi_function("__otter_iter_free_array")?;

            let iter_handle = self
                .builder
                .build_call(iter_create_fn, &[iterable_handle.into()], "dict_comp_iter")?
                .try_as_basic_value()
                .left()
                .ok_or_else(|| anyhow!("iterator creation failed"))?;

            let element_ty = self
                .comprehension_element_type(full_expr)
                .or_else(|| self.list_element_type(iterable))
                .or_else(|| self.infer_comprehension_var_type(var, value, condition))
                .unwrap_or(OtterType::Opaque);

            let function = self
                .builder
                .get_insert_block()
                .and_then(|b| b.get_parent())
                .ok_or_else(|| anyhow!("no active function for dict comprehension"))?;

            let var_alloca = self.create_entry_block_alloca(function, var, element_ty.clone())?;
            ctx.insert(
                var.to_string(),
                Variable {
                    ptr: var_alloca,
                    ty: element_ty.clone(),
                },
            );
            inserted_new_binding = true;

            let loop_cond_bb = self.context.append_basic_block(function, "dictcomp_cond");
            let loop_body_bb = self.context.append_basic_block(function, "dictcomp_body");
            let loop_cleanup_bb = self
                .context
                .append_basic_block(function, "dictcomp_cleanup");
            let loop_exit_bb = self.context.append_basic_block(function, "dictcomp_exit");

            self.builder.build_unconditional_branch(loop_cond_bb)?;

            self.builder.position_at_end(loop_cond_bb);
            let has_next_call = self.builder.build_call(
                iter_has_next_fn,
                &[iter_handle.into()],
                "dictcomp_has_next",
            )?;
            let has_next = has_next_call
                .try_as_basic_value()
                .left()
                .ok_or_else(|| anyhow!("iterator has_next returned void"))?
                .into_int_value();
            self.builder
                .build_conditional_branch(has_next, loop_body_bb, loop_cleanup_bb)?;

            self.builder.position_at_end(loop_body_bb);
            let next_call =
                self.builder
                    .build_call(iter_next_fn, &[iter_handle.into()], "dictcomp_next")?;
            let element_val = next_call
                .try_as_basic_value()
                .left()
                .ok_or_else(|| anyhow!("iterator next returned void"))?;
            let decoded = self.decode_and_convert_tagged_value(element_val, &element_ty)?;
            if let Some(value) = decoded {
                self.builder.build_store(var_alloca, value)?;
            }

            let loop_continue_bb = self
                .context
                .append_basic_block(function, "dictcomp_continue");

            if let Some(cond_expr) = condition {
                let cond_val = self.eval_expr(cond_expr, ctx)?;
                let cond_bool = self.to_bool_value(cond_val)?;
                let append_bb = self.context.append_basic_block(function, "dictcomp_append");
                let skip_bb = self.context.append_basic_block(function, "dictcomp_skip");
                self.builder
                    .build_conditional_branch(cond_bool, append_bb, skip_bb)?;

                self.builder.position_at_end(append_bb);
                self.emit_dict_comprehension_insert(map_handle, key, value, ctx)?;
                self.builder.build_unconditional_branch(loop_continue_bb)?;

                self.builder.position_at_end(skip_bb);
                self.builder.build_unconditional_branch(loop_continue_bb)?;
            } else {
                self.emit_dict_comprehension_insert(map_handle, key, value, ctx)?;
                self.builder.build_unconditional_branch(loop_continue_bb)?;
            }

            self.builder.position_at_end(loop_continue_bb);
            self.builder.build_unconditional_branch(loop_cond_bb)?;

            self.builder.position_at_end(loop_cleanup_bb);
            self.builder
                .build_call(iter_free_fn, &[iter_handle.into()], "dictcomp_free")?;
            self.builder.build_unconditional_branch(loop_exit_bb)?;

            self.builder.position_at_end(loop_exit_bb);

            Ok(EvaluatedValue::with_value(
                map_handle.into(),
                OtterType::Map,
            ))
        })();

        if inserted_new_binding {
            ctx.remove(var);
        }
        if let Some(prev) = previous_binding {
            ctx.insert(var.to_string(), prev);
        }

        result
    }

    fn emit_dict_comprehension_insert(
        &mut self,
        map_handle: IntValue<'ctx>,
        key_expr: &Expr,
        value_expr: &Expr,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<()> {
        let key_eval = self.eval_expr(key_expr, ctx)?;
        let key_value = self.ensure_string_value(key_eval)?;

        let value_eval = self.eval_expr(value_expr, ctx)?;
        let raw_value = value_eval
            .value
            .ok_or_else(|| anyhow!("dict comprehension value produced no value"))?;

        let (set_fn_name, expected_ty) = self.map_set_target(&value_eval.ty)?;
        let set_fn = self.get_or_declare_ffi_function(set_fn_name)?;
        let coerced_value = self.coerce_type(raw_value, value_eval.ty, expected_ty)?;

        let handle_arg: BasicMetadataValueEnum = map_handle.into();
        let key_arg: BasicMetadataValueEnum = key_value.into();
        let value_arg: BasicMetadataValueEnum = coerced_value.into();

        self.builder
            .build_call(set_fn, &[handle_arg, key_arg, value_arg], "dictcomp_set")?;
        Ok(())
    }

    fn map_set_target(&self, ty: &OtterType) -> Result<(&'static str, OtterType)> {
        match ty {
            OtterType::Str => Ok(("map.set", OtterType::Str)),
            OtterType::I32 | OtterType::I64 => Ok(("set<map,int>", OtterType::I64)),
            OtterType::F64 => Ok(("set<map,float>", OtterType::F64)),
            OtterType::Bool => Ok(("set<map,bool>", OtterType::Bool)),
            OtterType::List(_) => Ok(("set<map,list>", ty.clone())),
            OtterType::Map => Ok(("set<map,map>", OtterType::Map)),
            OtterType::Opaque => Ok(("set<map,list>", OtterType::opaque_list())),
            _ => bail!("unsupported dict comprehension value type: {:?}", ty),
        }
    }

    fn comprehension_element_type(&self, expr: &Expr) -> Option<OtterType> {
        let id = expr as *const Expr as usize;
        let span = self.expr_spans.get(&id)?;
        if let Some(ty) = self.comprehension_var_types.get(span) {
            self.typeinfo_to_otter_type(ty)
        } else {
            None
        }
    }

    fn infer_comprehension_var_type(
        &self,
        var: &str,
        element: &Expr,
        condition: Option<&Expr>,
    ) -> Option<OtterType> {
        self.find_identifier_type_in_expr(element, var)
            .or_else(|| condition.and_then(|cond| self.find_identifier_type_in_expr(cond, var)))
    }

    fn find_identifier_type_in_expr(&self, expr: &Expr, var: &str) -> Option<OtterType> {
        match expr {
            Expr::Identifier(name) if name == var => self
                .expr_type(expr)
                .and_then(|ty| self.typeinfo_to_otter_type(ty)),
            Expr::Identifier(_) | Expr::Literal(_) => None,
            Expr::Binary { left, right, .. } => self
                .find_identifier_type_in_expr(left.as_ref().as_ref(), var)
                .or_else(|| self.find_identifier_type_in_expr(right.as_ref().as_ref(), var)),
            Expr::Unary { expr, .. } | Expr::Await(expr) | Expr::Spawn(expr) => {
                self.find_identifier_type_in_expr(expr.as_ref().as_ref(), var)
            }
            Expr::Call { func, args } => self
                .find_identifier_type_in_expr(func.as_ref().as_ref(), var)
                .or_else(|| {
                    args.iter()
                        .find_map(|arg| self.find_identifier_type_in_expr(arg.as_ref(), var))
                }),
            Expr::Member { object, .. } => {
                self.find_identifier_type_in_expr(object.as_ref().as_ref(), var)
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => self
                .find_identifier_type_in_expr(cond.as_ref().as_ref(), var)
                .or_else(|| self.find_identifier_type_in_expr(then_branch.as_ref().as_ref(), var))
                .or_else(|| {
                    else_branch.as_ref().and_then(|expr| {
                        self.find_identifier_type_in_expr(expr.as_ref().as_ref(), var)
                    })
                }),
            Expr::Match { value, arms } => self
                .find_identifier_type_in_expr(value.as_ref().as_ref(), var)
                .or_else(|| {
                    arms.iter().find_map(|arm| {
                        let arm_ref = arm.as_ref();
                        if let Some(guard) = &arm_ref.guard
                            && let Some(found) =
                                self.find_identifier_type_in_expr(guard.as_ref(), var)
                        {
                            return Some(found);
                        }
                        self.find_identifier_type_in_block(arm_ref.body.as_ref(), var)
                    })
                }),
            Expr::Range { start, end } => self
                .find_identifier_type_in_expr(start.as_ref().as_ref(), var)
                .or_else(|| self.find_identifier_type_in_expr(end.as_ref().as_ref(), var)),
            Expr::Array(elements) => elements
                .iter()
                .find_map(|elem| self.find_identifier_type_in_expr(elem.as_ref(), var)),
            Expr::Dict(pairs) => pairs.iter().find_map(|(key, value)| {
                self.find_identifier_type_in_expr(key.as_ref(), var)
                    .or_else(|| self.find_identifier_type_in_expr(value.as_ref(), var))
            }),
            Expr::ListComprehension {
                element,
                var: inner_var,
                iterable,
                condition,
            } => {
                if inner_var == var {
                    None
                } else {
                    self.find_identifier_type_in_expr(iterable.as_ref().as_ref(), var)
                        .or_else(|| {
                            self.find_identifier_type_in_expr(element.as_ref().as_ref(), var)
                        })
                        .or_else(|| {
                            condition.as_ref().and_then(|cond| {
                                self.find_identifier_type_in_expr(cond.as_ref().as_ref(), var)
                            })
                        })
                }
            }
            Expr::DictComprehension {
                key,
                value,
                var: inner_var,
                iterable,
                condition,
            } => {
                if inner_var == var {
                    None
                } else {
                    self.find_identifier_type_in_expr(iterable.as_ref().as_ref(), var)
                        .or_else(|| self.find_identifier_type_in_expr(key.as_ref().as_ref(), var))
                        .or_else(|| self.find_identifier_type_in_expr(value.as_ref().as_ref(), var))
                        .or_else(|| {
                            condition.as_ref().and_then(|cond| {
                                self.find_identifier_type_in_expr(cond.as_ref().as_ref(), var)
                            })
                        })
                }
            }
            Expr::FString { parts } => parts.iter().find_map(|part| match part.as_ref() {
                FStringPart::Expr(expr) => self.find_identifier_type_in_expr(expr.as_ref(), var),
                _ => None,
            }),
            Expr::Struct { fields, .. } => fields
                .iter()
                .find_map(|(_, expr)| self.find_identifier_type_in_expr(expr.as_ref(), var)),
        }
    }

    fn find_identifier_type_in_block(&self, block: &Block, var: &str) -> Option<OtterType> {
        block
            .statements
            .iter()
            .find_map(|stmt| self.find_identifier_type_in_statement(stmt.as_ref(), var))
    }

    fn find_identifier_type_in_statement(&self, stmt: &Statement, var: &str) -> Option<OtterType> {
        match stmt {
            Statement::Expr(expr)
            | Statement::Return(Some(expr))
            | Statement::Let { expr, .. }
            | Statement::Assignment { expr, .. } => {
                self.find_identifier_type_in_expr(expr.as_ref(), var)
            }
            Statement::Return(None)
            | Statement::Break
            | Statement::Continue
            | Statement::Pass
            | Statement::Struct { .. }
            | Statement::Enum { .. }
            | Statement::TypeAlias { .. }
            | Statement::Use { .. }
            | Statement::PubUse { .. }
            | Statement::Function(_) => None,
            Statement::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
            } => self
                .find_identifier_type_in_expr(cond.as_ref(), var)
                .or_else(|| self.find_identifier_type_in_block(then_block.as_ref(), var))
                .or_else(|| {
                    elif_blocks.iter().find_map(|(cond, block)| {
                        self.find_identifier_type_in_expr(cond.as_ref(), var)
                            .or_else(|| self.find_identifier_type_in_block(block.as_ref(), var))
                    })
                })
                .or_else(|| {
                    else_block
                        .as_ref()
                        .and_then(|block| self.find_identifier_type_in_block(block.as_ref(), var))
                }),
            Statement::While { cond, body } => self
                .find_identifier_type_in_expr(cond.as_ref(), var)
                .or_else(|| self.find_identifier_type_in_block(body.as_ref(), var)),
            Statement::For { iterable, body, .. } => self
                .find_identifier_type_in_expr(iterable.as_ref(), var)
                .or_else(|| self.find_identifier_type_in_block(body.as_ref(), var)),
            Statement::Block(block) => self.find_identifier_type_in_block(block.as_ref(), var),
        }
    }

    fn ensure_string_value(&mut self, value: EvaluatedValue<'ctx>) -> Result<BasicValueEnum<'ctx>> {
        let EvaluatedValue { ty, value } = value;
        let base_value = value.ok_or_else(|| anyhow!("expected value for string operation"))?;

        match ty {
            OtterType::Str => Ok(base_value),
            OtterType::I64 => {
                self.call_ffi_returning_value("std.strings.format_int", vec![base_value], "fmt_int")
            }
            OtterType::I32 => {
                let int_val = base_value.into_int_value();
                let widened = self.builder.build_int_s_extend(
                    int_val,
                    self.context.i64_type(),
                    "i32_to_i64",
                )?;
                self.call_ffi_returning_value(
                    "std.strings.format_int",
                    vec![widened.into()],
                    "fmt_int",
                )
            }
            OtterType::F64 => self.call_ffi_returning_value(
                "std.strings.format_float",
                vec![base_value],
                "fmt_float",
            ),
            OtterType::Bool => self.call_ffi_returning_value(
                "std.strings.format_bool",
                vec![base_value],
                "fmt_bool",
            ),
            OtterType::List(_) => {
                // Try to convert list handle to string
                // Opaque types might be list handles, so try stringify
                self.call_ffi_returning_value("stringify<list>", vec![base_value], "stringify_list")
            }
            OtterType::Map => {
                self.call_ffi_returning_value("stringify<map>", vec![base_value], "stringify_map")
            }
            OtterType::Opaque => {
                self.call_ffi_returning_value("stringify<list>", vec![base_value], "stringify_list")
            }
            _ => bail!("cannot convert {:?} to string", ty),
        }
    }

    fn call_ffi_returning_value(
        &mut self,
        name: &str,
        args: Vec<BasicValueEnum<'ctx>>,
        label: &str,
    ) -> Result<BasicValueEnum<'ctx>> {
        let function = self.get_or_declare_ffi_function(name)?;
        let metadata_args: Vec<BasicMetadataValueEnum> =
            args.into_iter().map(|arg| arg.into()).collect();
        let call = self.builder.build_call(function, &metadata_args, label)?;
        call.try_as_basic_value()
            .left()
            .ok_or_else(|| anyhow!("FFI function {name} returned void"))
    }

    fn try_build_enum_constructor(
        &mut self,
        call_expr: &Expr,
        func_expr: &Expr,
        args: &[Node<Expr>],
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<Option<EvaluatedValue<'ctx>>> {
        if let (Expr::Member { object, field }, Some(enum_type @ TypeInfo::Enum { .. })) =
            (func_expr, self.expr_type(call_expr).cloned())
        {
            let TypeInfo::Enum {
                name: enum_name, ..
            } = &enum_type
            else {
                unreachable!()
            };
            if let Some(base) = self.member_base_name(object.as_ref().as_ref()) {
                if base != enum_name {
                    return Ok(None);
                }
            } else {
                return Ok(None);
            }
            let mut evaluated_args = Vec::with_capacity(args.len());
            for arg in args {
                evaluated_args.push(self.eval_expr(arg.as_ref(), ctx)?);
            }
            let value = self.build_enum_value_from_type(&enum_type, field, evaluated_args)?;
            return Ok(Some(value));
        }
        Ok(None)
    }

    fn member_base_name<'a>(&self, expr: &'a Expr) -> Option<&'a str> {
        match expr {
            Expr::Identifier(name) => Some(name.as_str()),
            Expr::Member { field, .. } => Some(field.as_str()),
            _ => None,
        }
    }

    fn try_build_enum_from_member(
        &mut self,
        enum_name: &str,
        variant_name: &str,
        args: &[Node<Expr>],
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<Option<EvaluatedValue<'ctx>>> {
        if let Some(layout) = self.enum_layout(enum_name) {
            let tag = layout.tag_of(variant_name);
            if tag.is_none() {
                return Ok(None);
            }
            let tag = tag.unwrap();

            let mut evaluated_args = Vec::with_capacity(args.len());
            for arg in args {
                evaluated_args.push(self.eval_expr(arg.as_ref(), ctx)?);
            }

            let field_types: Vec<TypeInfo> = evaluated_args
                .iter()
                .map(|val| match val.ty.clone() {
                    OtterType::I64 => TypeInfo::I64,
                    OtterType::F64 => TypeInfo::F64,
                    OtterType::Bool => TypeInfo::Bool,
                    OtterType::Str => TypeInfo::Str,
                    _ => TypeInfo::Unknown,
                })
                .collect();

            let value = self.create_enum_instance(
                enum_name,
                variant_name,
                tag,
                &field_types,
                evaluated_args,
            )?;

            return Ok(Some(value));
        }
        Ok(None)
    }

    fn try_build_enum_member(
        &mut self,
        expr: &Expr,
        _object: &Expr,
        field: &str,
        _ctx: &mut FunctionContext<'ctx>,
    ) -> Result<Option<EvaluatedValue<'ctx>>> {
        if let Some(enum_type_ref @ TypeInfo::Enum { variants, .. }) = self.expr_type(expr)
            && let Some(variant) = variants.get(field)
            && variant.fields.is_empty()
        {
            let enum_type = enum_type_ref.clone();
            return Ok(Some(self.build_enum_value_from_type(
                &enum_type,
                field,
                Vec::new(),
            )?));
        }
        Ok(None)
    }

    fn module_path_from_expr(&self, expr: &Expr) -> Option<String> {
        self.expr_type(expr).and_then(|ty| match ty {
            TypeInfo::Module(name) => Some(name.clone()),
            _ => None,
        })
    }

    fn resolve_member_function_name(&self, object: &Expr, field: &str) -> Option<String> {
        if let Some(module) = self.module_path_from_expr(object) {
            let candidate = format!("{}.{}", module, field);
            if self.symbol_registry.contains(&candidate)
                || self.declared_functions.contains_key(&candidate)
            {
                return Some(candidate);
            }
        }

        let prefix = self.flatten_member_chain(object)?;
        let candidate = format!("{}.{}", prefix, field);
        if self.symbol_registry.contains(&candidate) {
            Some(candidate)
        } else {
            None
        }
    }

    fn flatten_member_chain(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Identifier(name) => Some(name.clone()),
            Expr::Member { object, field } => {
                let prefix = self.flatten_member_chain(object.as_ref().as_ref())?;
                Some(format!("{}.{}", prefix, field))
            }
            _ => None,
        }
    }

    fn build_enum_value_from_type(
        &mut self,
        enum_type: &TypeInfo,
        variant_name: &str,
        values: Vec<EvaluatedValue<'ctx>>,
    ) -> Result<EvaluatedValue<'ctx>> {
        if let TypeInfo::Enum { name, variants, .. } = enum_type {
            let layout = self
                .enum_layout(name)
                .ok_or_else(|| anyhow!("Missing enum layout for {name}"))?;
            let tag = layout
                .tag_of(variant_name)
                .ok_or_else(|| anyhow!("Unknown variant {name}.{variant_name}"))?;
            let variant_info = variants
                .get(variant_name)
                .ok_or_else(|| anyhow!("No variant named {variant_name} for enum {name}"))?;
            if variant_info.fields.len() != values.len() {
                bail!(
                    "enum variant {}.{} expects {} field(s), got {}",
                    name,
                    variant_name,
                    variant_info.fields.len(),
                    values.len()
                );
            }

            self.create_enum_instance(name, variant_name, tag, &variant_info.fields, values)
        } else {
            bail!("expected enum type when constructing variant {variant_name}");
        }
    }

    fn create_enum_instance(
        &mut self,
        _enum_name: &str,
        _variant_name: &str,
        tag: u32,
        field_types: &[TypeInfo],
        values: Vec<EvaluatedValue<'ctx>>,
    ) -> Result<EvaluatedValue<'ctx>> {
        let i64_type = self.context.i64_type();
        let tag_value = i64_type.const_int(tag as u64, false);
        let field_count = i64_type.const_int(field_types.len() as u64, false);
        let create_fn = self.get_or_declare_ffi_function("runtime.enum.create")?;
        let handle = self
            .builder
            .build_call(
                create_fn,
                &[tag_value.into(), field_count.into()],
                "enum_create",
            )?
            .try_as_basic_value()
            .left()
            .ok_or_else(|| anyhow!("runtime.enum.create returned void"))?;

        for (index, (field_type, value)) in field_types.iter().zip(values.into_iter()).enumerate() {
            self.store_enum_field(handle, index as u32, field_type, value)?;
        }

        Ok(EvaluatedValue::with_value(handle, OtterType::Opaque))
    }

    fn store_enum_field(
        &mut self,
        handle: BasicValueEnum<'ctx>,
        index: u32,
        field_type: &TypeInfo,
        value: EvaluatedValue<'ctx>,
    ) -> Result<()> {
        let i64_type = self.context.i64_type();
        let index_val = i64_type.const_int(index as u64, false);
        let handle_arg: BasicMetadataValueEnum = handle.into();
        let index_arg: BasicMetadataValueEnum = index_val.into();

        match enum_field_kind(field_type) {
            EnumFieldKind::Int => {
                let data = self.value_as_i64(value)?;
                let setter = self.get_or_declare_ffi_function("runtime.enum.set_i64")?;
                self.builder.build_call(
                    setter,
                    &[handle_arg, index_arg, data.into()],
                    "enum_set_i64",
                )?;
            }
            EnumFieldKind::Float => {
                let data = self.value_as_f64(value)?;
                let setter = self.get_or_declare_ffi_function("runtime.enum.set_f64")?;
                self.builder.build_call(
                    setter,
                    &[handle_arg, index_arg, data.into()],
                    "enum_set_f64",
                )?;
            }
            EnumFieldKind::Bool => {
                let data = self.value_as_bool(value)?;
                let setter = self.get_or_declare_ffi_function("runtime.enum.set_bool")?;
                self.builder.build_call(
                    setter,
                    &[handle_arg, index_arg, data.into()],
                    "enum_set_bool",
                )?;
            }
            EnumFieldKind::Ptr => {
                let data = self.value_as_i64(value)?;
                let setter = self.get_or_declare_ffi_function("runtime.enum.set_ptr")?;
                self.builder.build_call(
                    setter,
                    &[handle_arg, index_arg, data.into()],
                    "enum_set_ptr",
                )?;
            }
        }

        Ok(())
    }

    fn value_as_i64(&mut self, value: EvaluatedValue<'ctx>) -> Result<IntValue<'ctx>> {
        let EvaluatedValue { ty, value } = value;
        let raw = value.ok_or_else(|| anyhow!("missing value for enum field"))?;
        let int_value = match ty {
            OtterType::I64 | OtterType::Opaque => raw.into_int_value(),
            OtterType::I32 => self.builder.build_int_s_extend(
                raw.into_int_value(),
                self.context.i64_type(),
                "i32_to_i64",
            )?,
            OtterType::Bool => self.builder.build_int_z_extend(
                raw.into_int_value(),
                self.context.i64_type(),
                "bool_to_i64",
            )?,
            OtterType::Str => self.builder.build_ptr_to_int(
                raw.into_pointer_value(),
                self.context.i64_type(),
                "str_ptr_to_int",
            )?,
            _ => {
                bail!("cannot convert {:?} to i64 for enum field", ty);
            }
        };
        Ok(int_value)
    }

    fn value_as_f64(&mut self, value: EvaluatedValue<'ctx>) -> Result<BasicValueEnum<'ctx>> {
        let EvaluatedValue { ty, value } = value;
        let raw = value.ok_or_else(|| anyhow!("missing value for enum field"))?;
        let float_value = match ty {
            OtterType::F64 => raw.into_float_value(),
            OtterType::I64 => self.builder.build_signed_int_to_float(
                raw.into_int_value(),
                self.context.f64_type(),
                "i64_to_f64",
            )?,
            OtterType::I32 => self.builder.build_signed_int_to_float(
                raw.into_int_value(),
                self.context.f64_type(),
                "i32_to_f64",
            )?,
            _ => {
                bail!("cannot convert {:?} to f64 for enum field", ty);
            }
        };
        Ok(float_value.into())
    }

    fn value_as_bool(&self, value: EvaluatedValue<'ctx>) -> Result<IntValue<'ctx>> {
        let EvaluatedValue { ty, value } = value;
        if ty == OtterType::Bool {
            Ok(value
                .ok_or_else(|| anyhow!("missing bool value for enum field"))?
                .into_int_value())
        } else {
            bail!("expected bool value for enum field, got {:?}", ty);
        }
    }
}

enum EnumFieldKind {
    Int,
    Float,
    Bool,
    Ptr,
}

fn enum_field_kind(field_type: &TypeInfo) -> EnumFieldKind {
    match field_type {
        TypeInfo::Bool => EnumFieldKind::Bool,
        TypeInfo::I32 | TypeInfo::I64 => EnumFieldKind::Int,
        TypeInfo::F64 => EnumFieldKind::Float,
        TypeInfo::Alias { underlying, .. } => enum_field_kind(underlying),
        _ => EnumFieldKind::Ptr,
    }
}
