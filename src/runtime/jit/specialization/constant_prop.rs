use super::RuntimeConstant;
use ast::nodes::{Expr, Literal};

/// Propagates constant values through expressions
pub struct ConstantPropagator;

impl ConstantPropagator {
    /// Analyze expression to extract constant values
    pub fn extract_constants(&self, expr: &Expr) -> Vec<Option<RuntimeConstant>> {
        match expr {
            Expr::Call { args, .. } => args
                .iter()
                .map(|arg| self.extract_constant_from_expr(arg.as_ref()))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn extract_constant_from_expr(&self, expr: &Expr) -> Option<RuntimeConstant> {
        match expr {
            Expr::Literal(lit) => self.literal_to_constant(lit.as_ref()),
            _ => None,
        }
    }

    fn literal_to_constant(&self, lit: &Literal) -> Option<RuntimeConstant> {
        match lit {
            Literal::Bool(b) => Some(RuntimeConstant::Bool(*b)),
            Literal::Number(n) => {
                if n.value.fract() == 0.0 {
                    // Integer
                    if n.value >= i32::MIN as f64 && n.value <= i32::MAX as f64 {
                        Some(RuntimeConstant::I32(n.value as i32))
                    } else {
                        Some(RuntimeConstant::I64(n.value as i64))
                    }
                } else {
                    // Float
                    Some(RuntimeConstant::from_f64(n.value))
                }
            }
            Literal::String(s) => Some(RuntimeConstant::Str(s.clone())),
            _ => None,
        }
    }

    /// Check if an expression can be constant-folded
    pub fn can_fold(&self, expr: &Expr) -> bool {
        matches!(expr, Expr::Literal(_))
    }

    /// Fold a constant expression to its value
    pub fn fold(&self, expr: &Expr) -> Option<RuntimeConstant> {
        match expr {
            Expr::Literal(lit) => self.literal_to_constant(lit.as_ref()),
            _ => None,
        }
    }
}
