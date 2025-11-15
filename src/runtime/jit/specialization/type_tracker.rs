use super::RuntimeType;
use ast::nodes::Expr;

/// Tracks runtime types for specialization
pub struct TypeTracker {
    type_cache: Vec<RuntimeType>,
}

impl Default for TypeTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeTracker {
    pub fn new() -> Self {
        Self {
            type_cache: Vec::new(),
        }
    }

    /// Infer runtime type from expression
    pub fn infer_type(&mut self, expr: &Expr) -> RuntimeType {
        match expr {
            Expr::Literal(lit) => match lit {
                ast::nodes::Literal::Bool(_) => RuntimeType::Bool,
                ast::nodes::Literal::Number(_) => RuntimeType::F64, // Default to float
                ast::nodes::Literal::String(_) => RuntimeType::Str,
                _ => RuntimeType::Unknown,
            },
            Expr::Identifier { .. } => RuntimeType::Unknown, // Would need symbol table lookup
            _ => RuntimeType::Unknown,
        }
    }

    /// Get cached types
    pub fn get_cached_types(&self) -> &[RuntimeType] {
        &self.type_cache
    }

    /// Clear type cache
    pub fn clear(&mut self) {
        self.type_cache.clear();
    }
}
