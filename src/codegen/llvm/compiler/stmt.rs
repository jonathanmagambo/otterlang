use anyhow::{Result, bail};
use inkwell::values::FunctionValue;

use crate::codegen::llvm::compiler::Compiler;
use crate::codegen::llvm::compiler::types::{FunctionContext, OtterType, Variable};
use ast::nodes::{Block, Expr, Statement};

struct IteratorRuntime<'ctx> {
    create_fn: FunctionValue<'ctx>,
    has_next_fn: FunctionValue<'ctx>,
    next_fn: FunctionValue<'ctx>,
    free_fn: FunctionValue<'ctx>,
    element_type: OtterType,
}

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn lower_block(
        &mut self,
        block: &Block,
        function: FunctionValue<'ctx>,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<()> {
        for stmt in &block.statements {
            self.lower_statement(stmt.as_ref(), function, ctx)?;
        }
        Ok(())
    }

    pub(crate) fn lower_statement(
        &mut self,
        stmt: &Statement,
        function: FunctionValue<'ctx>,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<()> {
        match stmt {
            Statement::Expr(expr) => {
                self.eval_expr(expr.as_ref(), ctx)?;
                Ok(())
            }
            Statement::Return(expr) => {
                if let Some(expr) = expr {
                    let val = self.eval_expr(expr.as_ref(), ctx)?;
                    if let Some(v) = val.value {
                        self.builder.build_return(Some(&v))?;
                    } else {
                        self.builder.build_return(None)?;
                    }
                } else {
                    self.builder.build_return(None)?;
                }
                Ok(())
            }
            Statement::Let {
                name,
                ty: _,
                expr,
                public: _,
            } => {
                let val = self.eval_expr(expr.as_ref(), ctx)?;

                // Skip allocation for Unit types
                if let Some(basic_ty) = self.basic_type(val.ty)? {
                    let alloca = self.builder.build_alloca(basic_ty, name.as_ref())?;
                    if let Some(v) = val.value {
                        self.builder.build_store(alloca, v)?;
                    }
                    ctx.insert(
                        name.as_ref().to_string(),
                        Variable {
                            ptr: alloca,
                            ty: val.ty,
                        },
                    );
                }
                // For Unit types, we don't create a variable
                Ok(())
            }
            Statement::Assignment { name, expr } => {
                let val = self.eval_expr(expr.as_ref(), ctx)?;
                if let Some(var) = ctx.get(name.as_ref()) {
                    if let Some(v) = val.value {
                        // Type checking and coercion
                        let coerced_val = self.coerce_type(v, val.ty, var.ty)?;
                        self.builder.build_store(var.ptr, coerced_val)?;
                    } else if val.ty != OtterType::Unit {
                        bail!(
                            "Cannot assign non-unit expression with no value to variable {}",
                            name.as_ref()
                        );
                    }
                    // Unit type assignments are no-ops
                } else {
                    bail!("Variable {} not declared", name.as_ref());
                }
                Ok(())
            }
            Statement::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
            } => self.lower_if_statement(
                function,
                ctx,
                cond.as_ref(),
                then_block.as_ref(),
                elif_blocks,
                else_block.as_ref().map(|b| b.as_ref()),
            ),
            Statement::While { cond, body } => {
                self.lower_while_loop(function, ctx, cond.as_ref(), body.as_ref())
            }
            Statement::Break => {
                if let Some(loop_ctx) = ctx.current_loop() {
                    self.builder.build_unconditional_branch(loop_ctx.exit_bb)?;
                } else {
                    bail!("break statement outside of loop");
                }
                Ok(())
            }
            Statement::Continue => {
                if let Some(loop_ctx) = ctx.current_loop() {
                    self.builder.build_unconditional_branch(loop_ctx.cond_bb)?;
                } else {
                    bail!("continue statement outside of loop");
                }
                Ok(())
            }
            Statement::Pass => Ok(()),
            Statement::Struct { .. } => Ok(()), // Handled at module level
            Statement::Enum { .. } => Ok(()),   // Handled at module level
            Statement::TypeAlias { .. } => Ok(()), // Handled at module level
            Statement::Function(_) => Ok(()),   // Handled at module level
            Statement::Use { .. } => Ok(()),    // Handled at module level
            Statement::Try {
                body,
                handlers,
                else_block,
                finally_block,
            } => self.lower_try_catch(
                body.as_ref(),
                handlers,
                else_block.as_ref().map(|b| b.as_ref()),
                finally_block.as_ref().map(|b| b.as_ref()),
                function,
                ctx,
            ),
            Statement::Raise(expr) => {
                self.lower_raise(expr.as_ref().map(|e| e.as_ref()), function, ctx)
            }
            Statement::For {
                var,
                iterable,
                body,
            } => self.lower_for_loop(
                var.as_ref(),
                iterable.as_ref(),
                body.as_ref(),
                function,
                ctx,
            ),
            Statement::PubUse { .. } => Ok(()), // Handled at module level
            Statement::Block(block) => self.lower_block(block.as_ref(), function, ctx),
        }
    }

    fn lower_if_statement(
        &mut self,
        function: FunctionValue<'ctx>,
        ctx: &mut FunctionContext<'ctx>,
        cond: &Expr,
        then_block: &Block,
        elif_blocks: &[(ast::nodes::Node<Expr>, ast::nodes::Node<Block>)],
        else_block: Option<&Block>,
    ) -> Result<()> {
        let cond_val = self.eval_expr(cond, ctx)?;
        let cond_bool = self.to_bool_value(cond_val)?;

        let then_bb = self.context.append_basic_block(function, "then");
        let else_bb = self.context.append_basic_block(function, "else");
        let merge_bb = self.context.append_basic_block(function, "merge");

        self.builder
            .build_conditional_branch(cond_bool, then_bb, else_bb)?;

        // Then block
        self.builder.position_at_end(then_bb);
        self.lower_block(then_block, function, ctx)?;
        if self
            .builder
            .get_insert_block()
            .and_then(|b| b.get_terminator())
            .is_none()
        {
            self.builder.build_unconditional_branch(merge_bb)?;
        }

        // Else block (handle elifs recursively or iteratively)
        self.builder.position_at_end(else_bb);
        if !elif_blocks.is_empty() {
            let (next_cond, next_block) = &elif_blocks[0];
            self.lower_if_statement(
                function,
                ctx,
                next_cond.as_ref(),
                next_block.as_ref(),
                &elif_blocks[1..],
                else_block,
            )?;
        } else if let Some(block) = else_block {
            self.lower_block(block, function, ctx)?;
            if self
                .builder
                .get_insert_block()
                .and_then(|b| b.get_terminator())
                .is_none()
            {
                self.builder.build_unconditional_branch(merge_bb)?;
            }
        } else {
            self.builder.build_unconditional_branch(merge_bb)?;
        }

        self.builder.position_at_end(merge_bb);
        Ok(())
    }

    fn lower_while_loop(
        &mut self,
        function: FunctionValue<'ctx>,
        ctx: &mut FunctionContext<'ctx>,
        cond: &Expr,
        body: &Block,
    ) -> Result<()> {
        let cond_bb = self.context.append_basic_block(function, "while_cond");
        let body_bb = self.context.append_basic_block(function, "while_body");
        let exit_bb = self.context.append_basic_block(function, "while_exit");

        self.builder.build_unconditional_branch(cond_bb)?;

        // Condition
        self.builder.position_at_end(cond_bb);
        let cond_val = self.eval_expr(cond, ctx)?;
        let cond_bool = self.to_bool_value(cond_val)?;
        self.builder
            .build_conditional_branch(cond_bool, body_bb, exit_bb)?;

        // Body
        self.builder.position_at_end(body_bb);
        ctx.push_loop(cond_bb, exit_bb);
        self.lower_block(body, function, ctx)?;
        ctx.pop_loop();

        if self
            .builder
            .get_insert_block()
            .and_then(|b| b.get_terminator())
            .is_none()
        {
            self.builder.build_unconditional_branch(cond_bb)?;
        }

        self.builder.position_at_end(exit_bb);
        Ok(())
    }

    fn lower_for_loop(
        &mut self,
        var: &str,
        iterable: &Expr,
        body: &Block,
        function: FunctionValue<'ctx>,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<()> {
        use ast::nodes::Expr;

        // Iterator protocol implementation for range-based for loops

        // Check if iterable is a range expression
        if let Expr::Range { start, end } = iterable {
            // Evaluate start and end
            let start_val = self.eval_expr(start.as_ref().as_ref(), ctx)?;
            let end_val = self.eval_expr(end.as_ref().as_ref(), ctx)?;

            // Determine if we're using I64 or F64 iterators
            let is_float = start_val.ty == OtterType::F64;

            // Get runtime iterator functions (copy to avoid borrow issues)
            let (iter_create_fn, iter_has_next_fn, iter_next_fn, iter_free_fn) = if is_float {
                (
                    *self
                        .declared_functions
                        .get("__otter_iter_range_f64")
                        .ok_or_else(|| anyhow::anyhow!("Iterator runtime not available"))?,
                    *self
                        .declared_functions
                        .get("__otter_iter_has_next_f64")
                        .ok_or_else(|| anyhow::anyhow!("Iterator runtime not available"))?,
                    *self
                        .declared_functions
                        .get("__otter_iter_next_f64")
                        .ok_or_else(|| anyhow::anyhow!("Iterator runtime not available"))?,
                    *self
                        .declared_functions
                        .get("__otter_iter_free_f64")
                        .ok_or_else(|| anyhow::anyhow!("Iterator runtime not available"))?,
                )
            } else {
                (
                    *self
                        .declared_functions
                        .get("__otter_iter_range")
                        .ok_or_else(|| anyhow::anyhow!("Iterator runtime not available"))?,
                    *self
                        .declared_functions
                        .get("__otter_iter_has_next")
                        .ok_or_else(|| anyhow::anyhow!("Iterator runtime not available"))?,
                    *self
                        .declared_functions
                        .get("__otter_iter_next")
                        .ok_or_else(|| anyhow::anyhow!("Iterator runtime not available"))?,
                    *self
                        .declared_functions
                        .get("__otter_iter_free")
                        .ok_or_else(|| anyhow::anyhow!("Iterator runtime not available"))?,
                )
            };

            // Create iterator
            let iter_ptr = self
                .builder
                .build_call(
                    iter_create_fn,
                    &[
                        start_val.value.unwrap().into(),
                        end_val.value.unwrap().into(),
                    ],
                    "iter",
                )?
                .try_as_basic_value()
                .left()
                .unwrap();

            // Create loop variable
            let loop_var_alloca = self.builder.build_alloca(
                self.basic_type(start_val.ty)?
                    .ok_or_else(|| anyhow::anyhow!("Cannot iterate over unit type"))?,
                var,
            )?;

            ctx.insert(
                var.to_string(),
                Variable {
                    ptr: loop_var_alloca,
                    ty: start_val.ty,
                },
            );

            // Create basic blocks
            let cond_bb = self.context.append_basic_block(function, "for_cond");
            let body_bb = self.context.append_basic_block(function, "for_body");
            let cleanup_bb = self.context.append_basic_block(function, "for_cleanup");
            let exit_bb = self.context.append_basic_block(function, "for_exit");

            self.builder.build_unconditional_branch(cond_bb)?;

            // Condition: has_next(iter)
            self.builder.position_at_end(cond_bb);
            let has_next = self
                .builder
                .build_call(iter_has_next_fn, &[iter_ptr.into()], "has_next")?
                .try_as_basic_value()
                .left()
                .unwrap()
                .into_int_value();

            self.builder
                .build_conditional_branch(has_next, body_bb, cleanup_bb)?;

            // Body
            self.builder.position_at_end(body_bb);

            // Get next value and store in loop variable
            let next_val = self
                .builder
                .build_call(iter_next_fn, &[iter_ptr.into()], "next")?
                .try_as_basic_value()
                .left()
                .unwrap();

            self.builder.build_store(loop_var_alloca, next_val)?;

            // Execute loop body
            ctx.push_loop(cond_bb, cleanup_bb);
            self.lower_block(body, function, ctx)?;
            ctx.pop_loop();

            if self
                .builder
                .get_insert_block()
                .and_then(|b| b.get_terminator())
                .is_none()
            {
                self.builder.build_unconditional_branch(cond_bb)?;
            }

            // Cleanup: free iterator
            self.builder.position_at_end(cleanup_bb);
            self.builder
                .build_call(iter_free_fn, &[iter_ptr.into()], "")?;
            self.builder.build_unconditional_branch(exit_bb)?;

            self.builder.position_at_end(exit_bb);
            Ok(())
        } else {
            // Handle other iterable types (arrays, strings, etc.)
            let iterable_val = self.eval_expr(iterable, ctx)?;

            match iterable_val.ty {
                OtterType::Str => {
                    // String iteration (character by character)
                    let (iter_create_fn, iter_has_next_fn, iter_next_fn, iter_free_fn) = (
                        *self
                            .declared_functions
                            .get("__otter_iter_string")
                            .ok_or_else(|| {
                                anyhow::anyhow!("String iterator runtime not available")
                            })?,
                        *self
                            .declared_functions
                            .get("__otter_iter_has_next_string")
                            .ok_or_else(|| {
                                anyhow::anyhow!("String iterator runtime not available")
                            })?,
                        *self
                            .declared_functions
                            .get("__otter_iter_next_string")
                            .ok_or_else(|| {
                                anyhow::anyhow!("String iterator runtime not available")
                            })?,
                        *self
                            .declared_functions
                            .get("__otter_iter_free_string")
                            .ok_or_else(|| {
                                anyhow::anyhow!("String iterator runtime not available")
                            })?,
                    );

                    self.lower_collection_for_loop(
                        var,
                        iterable_val,
                        body,
                        function,
                        ctx,
                        IteratorRuntime {
                            create_fn: iter_create_fn,
                            has_next_fn: iter_has_next_fn,
                            next_fn: iter_next_fn,
                            free_fn: iter_free_fn,
                            element_type: OtterType::I64,
                        },
                    )
                }
                OtterType::List => {
                    // Array/list iteration
                    let (iter_create_fn, iter_has_next_fn, iter_next_fn, iter_free_fn) = (
                        *self
                            .declared_functions
                            .get("__otter_iter_array")
                            .ok_or_else(|| {
                                anyhow::anyhow!("Array iterator runtime not available")
                            })?,
                        *self
                            .declared_functions
                            .get("__otter_iter_has_next_array")
                            .ok_or_else(|| {
                                anyhow::anyhow!("Array iterator runtime not available")
                            })?,
                        *self
                            .declared_functions
                            .get("__otter_iter_next_array")
                            .ok_or_else(|| {
                                anyhow::anyhow!("Array iterator runtime not available")
                            })?,
                        *self
                            .declared_functions
                            .get("__otter_iter_free_array")
                            .ok_or_else(|| {
                                anyhow::anyhow!("Array iterator runtime not available")
                            })?,
                    );

                    self.lower_collection_for_loop(
                        var,
                        iterable_val,
                        body,
                        function,
                        ctx,
                        IteratorRuntime {
                            create_fn: iter_create_fn,
                            has_next_fn: iter_has_next_fn,
                            next_fn: iter_next_fn,
                            free_fn: iter_free_fn,
                            element_type: OtterType::Opaque,
                        },
                    )
                }
                OtterType::Map => {
                    // Map iteration is not yet implemented
                    bail!("Map iteration is not yet supported")
                }
                _ => {
                    bail!(
                        "For loops over type {:?} are not supported yet",
                        iterable_val.ty
                    )
                }
            }
        }
    }

    fn lower_collection_for_loop(
        &mut self,
        var: &str,
        iterable_val: crate::codegen::llvm::compiler::types::EvaluatedValue<'ctx>,
        body: &Block,
        function: FunctionValue<'ctx>,
        ctx: &mut FunctionContext<'ctx>,
        iter_runtime: IteratorRuntime<'ctx>,
    ) -> Result<()> {
        let IteratorRuntime {
            create_fn,
            has_next_fn,
            next_fn,
            free_fn,
            element_type,
        } = iter_runtime;
        // Create iterator from collection
        let iter_handle = self.builder.build_call(
            create_fn,
            &[iterable_val
                .value
                .ok_or_else(|| anyhow::anyhow!("Iterable has no value"))?
                .into()],
            "iter_handle",
        )?;

        let iter_ptr = iter_handle
            .try_as_basic_value()
            .left()
            .ok_or_else(|| anyhow::anyhow!("Iterator creation failed"))?
            .into_pointer_value();

        // Create loop variable allocation
        let var_alloca = self.builder.build_alloca(
            self.basic_type(element_type)?
                .ok_or_else(|| anyhow::anyhow!("Cannot iterate over unit type"))?,
            var,
        )?;

        // Insert variable into context
        ctx.insert(
            var.to_string(),
            Variable {
                ptr: var_alloca,
                ty: element_type,
            },
        );

        // Create basic blocks
        let loop_cond_bb = self.context.append_basic_block(function, "loop_cond");
        let loop_body_bb = self.context.append_basic_block(function, "loop_body");
        let cleanup_bb = self.context.append_basic_block(function, "loop_cleanup");
        let exit_bb = self.context.append_basic_block(function, "loop_exit");

        // Jump to condition check
        self.builder.build_unconditional_branch(loop_cond_bb)?;

        // Condition check block
        self.builder.position_at_end(loop_cond_bb);

        // Check if iterator has next element
        let has_next_call = self
            .builder
            .build_call(has_next_fn, &[iter_ptr.into()], "has_next")?;

        let has_next = has_next_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| anyhow::anyhow!("has_next check failed"))?
            .into_int_value();

        self.builder
            .build_conditional_branch(has_next, loop_body_bb, cleanup_bb)?;

        // Loop body block
        self.builder.position_at_end(loop_body_bb);

        // Get next element
        let next_call = self
            .builder
            .build_call(next_fn, &[iter_ptr.into()], "next_element")?;

        let element_val = next_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| anyhow::anyhow!("next element failed"))?;

        // Store element in variable
        self.builder.build_store(var_alloca, element_val)?;

        // Execute loop body
        ctx.push_loop(loop_cond_bb, cleanup_bb);
        self.lower_block(body, function, ctx)?;
        ctx.pop_loop();

        // Jump back to condition check
        if self.builder.get_insert_block().is_some()
            && self
                .builder
                .get_insert_block()
                .unwrap()
                .get_terminator()
                .is_none()
        {
            self.builder.build_unconditional_branch(loop_cond_bb)?;
        }

        // Cleanup block - free iterator
        self.builder.position_at_end(cleanup_bb);
        self.builder.build_call(free_fn, &[iter_ptr.into()], "")?;
        self.builder.build_unconditional_branch(exit_bb)?;

        self.builder.position_at_end(exit_bb);

        Ok(())
    }

    fn lower_try_catch(
        &mut self,
        body: &Block,
        handlers: &[ast::nodes::Node<ast::nodes::ExceptHandler>],
        else_block: Option<&Block>,
        finally_block: Option<&Block>,
        function: FunctionValue<'ctx>,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<()> {
        // Create basic blocks
        let try_bb = self.context.append_basic_block(function, "try_body");
        let handlers_bb = self.context.append_basic_block(function, "handlers");
        let else_bb = self.context.append_basic_block(function, "try_else");
        let finally_bb = self.context.append_basic_block(function, "finally");
        let exit_bb = self.context.append_basic_block(function, "try_exit");

        // Jump to try block
        self.builder.build_unconditional_branch(try_bb)?;

        // Try body
        self.builder.position_at_end(try_bb);
        self.lower_block(body, function, ctx)?;

        // If no exception, jump to else or finally
        if self
            .builder
            .get_insert_block()
            .and_then(|b| b.get_terminator())
            .is_none()
        {
            if else_block.is_some() {
                self.builder.build_unconditional_branch(else_bb)?;
            } else if finally_block.is_some() {
                self.builder.build_unconditional_branch(finally_bb)?;
            } else {
                self.builder.build_unconditional_branch(exit_bb)?;
            }
        }

        // Exception handlers
        self.builder.position_at_end(handlers_bb);
        if !handlers.is_empty() {
            // For each handler, create a block and check exception type
            for (i, handler) in handlers.iter().enumerate() {
                let handler_bb = self
                    .context
                    .append_basic_block(function, &format!("handler_{}", i));
                let next_handler_bb = if i < handlers.len() - 1 {
                    self.context
                        .append_basic_block(function, &format!("handler_check_{}", i + 1))
                } else {
                    finally_bb
                };

                // Execute handler body (exception type checking can be added later)
                self.builder.position_at_end(handler_bb);
                self.lower_block(handler.as_ref().body.as_ref(), function, ctx)?;

                if self
                    .builder
                    .get_insert_block()
                    .and_then(|b| b.get_terminator())
                    .is_none()
                {
                    if finally_block.is_some() {
                        self.builder.build_unconditional_branch(finally_bb)?;
                    } else {
                        self.builder.build_unconditional_branch(exit_bb)?;
                    }
                }

                self.builder.position_at_end(next_handler_bb);
            }
        } else {
            // No handlers, jump to finally or exit
            if finally_block.is_some() {
                self.builder.build_unconditional_branch(finally_bb)?;
            } else {
                self.builder.build_unconditional_branch(exit_bb)?;
            }
        }

        // Else block (executed if no exception)
        if let Some(else_blk) = else_block {
            self.builder.position_at_end(else_bb);
            self.lower_block(else_blk, function, ctx)?;

            if self
                .builder
                .get_insert_block()
                .and_then(|b| b.get_terminator())
                .is_none()
            {
                if finally_block.is_some() {
                    self.builder.build_unconditional_branch(finally_bb)?;
                } else {
                    self.builder.build_unconditional_branch(exit_bb)?;
                }
            }
        }

        // Finally block (always executed)
        if let Some(finally) = finally_block {
            self.builder.position_at_end(finally_bb);
            self.lower_block(finally, function, ctx)?;

            if self
                .builder
                .get_insert_block()
                .and_then(|b| b.get_terminator())
                .is_none()
            {
                self.builder.build_unconditional_branch(exit_bb)?;
            }
        }

        // Exit block
        self.builder.position_at_end(exit_bb);

        Ok(())
    }

    fn lower_raise(
        &mut self,
        expr: Option<&Expr>,
        _function: FunctionValue<'ctx>,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<()> {
        // Exception raising with runtime support

        // Evaluate the exception expression if provided
        if let Some(exception_expr) = expr {
            let exception_val = self.eval_expr(exception_expr, ctx)?;

            // Look up the runtime throw function
            if let Some(throw_fn) = self.declared_functions.get("__otter_throw") {
                if let Some(exc_val) = exception_val.value {
                    // Call the throw function with the exception message
                    self.builder
                        .build_call(*throw_fn, &[exc_val.into()], "throw")?;
                }
            } else {
                // Fallback: if runtime not available, just print error
                eprintln!("Warning: Exception runtime not available, using unreachable");
            }
        }

        // After throwing, mark as unreachable
        // This tells LLVM that execution doesn't continue past this point
        self.builder.build_unreachable()?;

        Ok(())
    }
}
