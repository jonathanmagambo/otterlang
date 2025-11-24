use anyhow::{Result, anyhow, bail};
use inkwell::IntPredicate;
use inkwell::types::BasicTypeEnum;
use inkwell::values::IntValue;

use crate::codegen::llvm::compiler::Compiler;
use crate::codegen::llvm::compiler::types::{EvaluatedValue, FunctionContext, OtterType};
use ast::nodes::{BinaryOp, Expr, Literal, UnaryOp};

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn eval_expr(
        &mut self,
        expr: &Expr,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        match expr {
            Expr::Literal(lit) => self.eval_literal(lit.as_ref()),
            Expr::Identifier(name) => {
                if let Some(var) = ctx.get(name) {
                    if let Some(basic_ty) = self.basic_type(var.ty)? {
                        let val = self.builder.build_load(basic_ty, var.ptr, name)?;
                        Ok(EvaluatedValue::with_value(val, var.ty))
                    } else {
                        // Unit type - no value to load
                        Ok(EvaluatedValue {
                            ty: OtterType::Unit,
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
            Expr::If {
                cond: _,
                then_branch: _,
                else_branch: _,
            } => self.eval_if_expr(expr, ctx),
            _ => bail!("Expression type not implemented: {:?}", expr),
        }
    }

    fn eval_literal(&mut self, lit: &Literal) -> Result<EvaluatedValue<'ctx>> {
        match lit {
            Literal::Number(n) => {
                let val = self.context.f64_type().const_float(n.value);
                Ok(EvaluatedValue::with_value(val.into(), OtterType::F64))
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
            Literal::Unit => Ok(EvaluatedValue {
                ty: OtterType::Unit,
                value: None,
            }),
            Literal::None => Ok(EvaluatedValue {
                ty: OtterType::Unit,
                value: None,
            }), // Treat None as Unit for now
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

        match (lhs.ty, rhs.ty) {
            (OtterType::I64, OtterType::I64) => {
                let l = lhs.value.unwrap().into_int_value();
                let r = rhs.value.unwrap().into_int_value();
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
                    _ => bail!("Unsupported binary op for I64"),
                }
            }
            (OtterType::F64, OtterType::F64) => {
                let l = lhs.value.unwrap().into_float_value();
                let r = rhs.value.unwrap().into_float_value();
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
            _ => bail!("Type mismatch or unsupported types for binary op"),
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
                if val.ty == OtterType::I64 {
                    let v = val.value.unwrap().into_int_value();
                    Ok(EvaluatedValue::with_value(
                        self.builder.build_int_neg(v, "neg")?.into(),
                        OtterType::I64,
                    ))
                } else if val.ty == OtterType::F64 {
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
                if val.ty == OtterType::Bool {
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
            OtterType::I64 => Ok(Some(self.context.i64_type().into())),
            OtterType::F64 => Ok(Some(self.context.f64_type().into())),
            OtterType::Str => Ok(Some(self.string_ptr_type.into())),
            OtterType::Opaque => Ok(Some(self.context.i64_type().into())),
            OtterType::List => Ok(Some(self.context.i64_type().into())),
            OtterType::Map => Ok(Some(self.context.i64_type().into())),
        }
    }

    pub(crate) fn to_bool_value(&self, val: EvaluatedValue<'ctx>) -> Result<IntValue<'ctx>> {
        if val.ty == OtterType::Bool {
            Ok(val.value.unwrap().into_int_value())
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

        // Perform type coercion based on source and target types
        match (from_ty, to_ty) {
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
            (OtterType::Opaque, OtterType::I64) | (OtterType::I64, OtterType::Opaque) => {
                Ok(value) // Already same representation
            }

            // List/Map conversions (treat as opaque pointers)
            (OtterType::List | OtterType::Map, OtterType::Opaque)
            | (OtterType::Opaque, OtterType::List | OtterType::Map) => {
                Ok(value) // Already same representation
            }

            // Incompatible types
            _ => {
                bail!("Cannot coerce type {:?} to {:?}", from_ty, to_ty)
            }
        }
    }

    fn eval_call_expr(
        &mut self,
        expr: &Expr,
        ctx: &mut FunctionContext<'ctx>,
    ) -> Result<EvaluatedValue<'ctx>> {
        if let Expr::Call { func, args } = expr {
            // Evaluate function expression (usually an identifier)
            let func_name = if let Expr::Identifier(name) = func.as_ref().as_ref() {
                name.clone()
            } else {
                bail!("Complex function expressions not yet supported");
            };

            // Look up the function and clone it to avoid borrow issues
            let function = *self
                .declared_functions
                .get(&func_name)
                .ok_or_else(|| anyhow!("Function {} not found", func_name))?;

            // Get parameter types upfront to avoid borrow issues
            let param_types = function.get_type().get_param_types();

            // Evaluate arguments and convert types as needed
            let mut arg_values = Vec::new();
            for (i, arg) in args.iter().enumerate() {
                let arg_val = self.eval_expr(arg.as_ref(), ctx)?;
                if let Some(v) = arg_val.value {
                    // Get expected parameter type from function signature
                    let param_type = param_types
                        .get(i)
                        .ok_or_else(|| anyhow!("Too many arguments for function {}", func_name))?;

                    // Convert if needed (e.g., F64 to I64)
                    let converted_val = if arg_val.ty == OtterType::F64 && param_type.is_int_type()
                    {
                        // Convert F64 to I64
                        self.builder
                            .build_float_to_signed_int(
                                v.into_float_value(),
                                self.context.i64_type(),
                                "ftoi",
                            )?
                            .into()
                    } else {
                        v.into()
                    };

                    arg_values.push(converted_val);
                } else {
                    bail!("Cannot pass unit value as argument");
                }
            }

            // Call the function
            let call_site = self.builder.build_call(function, &arg_values, &func_name)?;

            // Get return value
            if let Some(ret_val) = call_site.try_as_basic_value().left() {
                // Function returns a value - assume F64 for now
                Ok(EvaluatedValue::with_value(ret_val, OtterType::F64))
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
            if then_val.ty == else_val.ty && then_val.value.is_some() && else_val.value.is_some() {
                if let Some(basic_ty) = self.basic_type(then_val.ty)? {
                    let phi = self.builder.build_phi(basic_ty, "if_result")?;
                    phi.add_incoming(&[
                        (&then_val.value.unwrap(), then_bb_end),
                        (&else_val.value.unwrap(), else_bb_end),
                    ]);
                    Ok(EvaluatedValue::with_value(
                        phi.as_basic_value(),
                        then_val.ty,
                    ))
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
}
