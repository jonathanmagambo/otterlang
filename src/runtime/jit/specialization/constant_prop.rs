use super::RuntimeConstant;
use crate::ast::nodes::Expr;

/// Propagates constant values through expressions
pub struct ConstantPropagator;

impl ConstantPropagator {
    pub fn new() -> Self {
        Self
    }

    /// Analyze expression to extract constant values
    pub fn extract_constants(&self, expr: &Expr) -> Vec<Option<RuntimeConstant>> {
        match expr {
            Expr::Literal(lit) => {
                vec![self.literal_to_constant(lit)]
            }
            Expr::Binary { left, right, .. } => {
                let mut result = self.extract_constants(left);
                result.extend(self.extract_constants(right));
                result
            }
            Expr::Call { args, .. } => args
                .iter()
                .flat_map(|arg| self.extract_constants(arg))
                .collect(),
            _ => vec![None],
        }
    }

    fn literal_to_constant(&self, lit: &crate::ast::nodes::Literal) -> Option<RuntimeConstant> {
        match lit {
            crate::ast::nodes::Literal::Bool(b) => Some(RuntimeConstant::Bool(*b)),
            crate::ast::nodes::Literal::Number(n) => {
                // Try to determine if it's an integer or float
                if !n.is_float_literal && n.value.fract() == 0.0 {
                    if n.value >= i32::MIN as f64 && n.value <= i32::MAX as f64 {
                        Some(RuntimeConstant::I32(n.value as i32))
                    } else {
                        Some(RuntimeConstant::I64(n.value as i64))
                    }
                } else {
                    Some(RuntimeConstant::F64(n.value))
                }
            }
            crate::ast::nodes::Literal::String(s) => Some(RuntimeConstant::Str(s.clone())),
            crate::ast::nodes::Literal::Unit => {
                // Unit type has no runtime constant representation
                None
            }
            crate::ast::nodes::Literal::None => None,
        }
    }
}

impl Default for ConstantPropagator {
    fn default() -> Self {
        Self::new()
    }
}
