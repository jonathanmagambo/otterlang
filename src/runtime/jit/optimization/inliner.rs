use ast::nodes::{Block, Expr, Function, Statement};

/// Inlines function calls for optimization
pub struct Inliner {
    max_inline_size: usize,
}

impl Inliner {
    pub fn new() -> Self {
        Self {
            max_inline_size: 50, // Don't inline functions larger than 50 statements
        }
    }

    pub fn should_inline(&self, _caller: &Function, callee: &Function) -> bool {
        // Simple heuristic: inline small functions called from hot paths
        let callee_size = self.count_statements(&callee.body);
        callee_size <= self.max_inline_size
    }

    pub fn inline_call(
        &self,
        caller: &mut Function,
        callee: &Function,
        call_expr: &Expr,
    ) -> Result<(), String> {
        // Extract arguments from call expression
        let args = if let Expr::Call { args, .. } = call_expr {
            args.clone()
        } else {
            return Err("Not a call expression".to_string());
        };

        // Clone callee body
        let mut inlined_body = callee.body.clone();

        // Replace parameter references with arguments
        self.substitute_parameters(&mut inlined_body, &callee.params, &args);

        // Insert inlined body into caller
        // This is a simplified version - a full implementation would need to
        // handle return values, variable scoping, etc.
        caller.body.statements.extend(inlined_body.statements);

        Ok(())
    }

    fn substitute_parameters(
        &self,
        block: &mut Block,
        params: &[ast::nodes::Param],
        args: &[Expr],
    ) {
        // Simplified parameter substitution
        // Full implementation would need proper variable renaming to avoid conflicts
        for stmt in &mut block.statements {
            self.substitute_in_stmt(stmt, params, args);
        }
    }

    fn substitute_in_stmt(
        &self,
        stmt: &mut Statement,
        params: &[ast::nodes::Param],
        args: &[Expr],
    ) {
        // Implementation would traverse AST and replace parameter references
        // This is a placeholder for the full implementation
        if let Statement::Expr(Expr::Identifier { name, .. }) = stmt {
            if let Some((idx, _)) = params.iter().enumerate().find(|(_, p)| p.name == *name)
                && idx < args.len() {
                    // This is simplified - would need proper expression replacement
                }
        }
    }

    fn count_statements(&self, block: &Block) -> usize {
        block.statements.iter().map(|s| self.count_in_stmt(s)).sum()
    }

    fn count_in_stmt(&self, stmt: &Statement) -> usize {
        match stmt {
            Statement::If {
                then_block,
                elif_blocks,
                else_block,
                ..
            } => {
                let mut count = 1;
                count += self.count_statements(then_block);
                for (_, block) in elif_blocks {
                    count += self.count_statements(block);
                }
                if let Some(block) = else_block {
                    count += self.count_statements(block);
                }
                count
            }
            Statement::For { body, .. } | Statement::While { body, .. } => {
                1 + self.count_statements(body)
            }
            _ => 1,
        }
    }
}

impl Default for Inliner {
    fn default() -> Self {
        Self::new()
    }
}
