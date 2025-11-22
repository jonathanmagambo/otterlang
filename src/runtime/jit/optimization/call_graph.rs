use ast::nodes::{Block, Expr, Function, Statement};

/// Builds call graph for optimization
pub struct CallGraph {
    calls: std::collections::HashMap<String, Vec<String>>,
}

impl CallGraph {
    pub fn new() -> Self {
        Self {
            calls: std::collections::HashMap::new(),
        }
    }

    pub fn analyze_program(&mut self, program: &ast::nodes::Program) {
        for function in program.functions() {
            self.analyze_function(function.as_ref());
        }
    }

    pub fn analyze_function(&mut self, function: &Function) {
        let callees = self.extract_callees(function.body.as_ref());
        self.calls.insert(function.name.clone(), callees);
    }

    fn extract_callees(&self, block: &Block) -> Vec<String> {
        let mut callees = Vec::new();
        for stmt in &block.statements {
            self.extract_callees_from_stmt(stmt.as_ref(), &mut callees);
        }
        callees
    }

    fn extract_callees_from_stmt(&self, stmt: &Statement, callees: &mut Vec<String>) {
        match stmt {
            Statement::Expr(expr) => {
                if let Expr::Call { func, .. } = expr.as_ref()
                    && let Expr::Identifier(name) = func.as_ref().as_ref()
                {
                    callees.push(name.clone());
                }
            }
            Statement::If {
                then_block,
                elif_blocks,
                else_block,
                ..
            } => {
                self.extract_callees_from_block(then_block.as_ref(), callees);
                for (_, block) in elif_blocks {
                    self.extract_callees_from_block(block.as_ref(), callees);
                }
                if let Some(block) = else_block {
                    self.extract_callees_from_block(block.as_ref(), callees);
                }
            }
            Statement::For { body, .. } | Statement::While { body, .. } => {
                self.extract_callees_from_block(body.as_ref(), callees);
            }
            _ => {}
        }
    }

    fn extract_callees_from_block(&self, block: &Block, callees: &mut Vec<String>) {
        for stmt in &block.statements {
            self.extract_callees_from_stmt(stmt.as_ref(), callees);
        }
    }

    pub fn get_callees(&self, function_name: &str) -> Option<&Vec<String>> {
        self.calls.get(function_name)
    }

    pub fn is_called_from(&self, callee: &str, caller: &str) -> bool {
        self.calls
            .get(caller)
            .map(|callees| callees.contains(&callee.to_string()))
            .unwrap_or(false)
    }

    /// Return the number of direct callees recorded for `function_name`.
    pub fn call_count(&self, function_name: &str) -> usize {
        self.calls
            .get(function_name)
            .map(|callees| callees.len())
            .unwrap_or(0)
    }

    /// Return the `limit` functions with the highest out-degree in the call graph.
    /// This acts as a heuristic "hot" list when no profiler guidance is available.
    pub fn hot_candidates(&self, limit: usize) -> Vec<String> {
        let mut entries: Vec<_> = self
            .calls
            .iter()
            .map(|(name, callees)| (name.clone(), callees.len()))
            .collect();

        entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        entries
            .into_iter()
            .take(limit)
            .map(|(name, _)| name)
            .collect()
    }
}

impl Default for CallGraph {
    fn default() -> Self {
        Self::new()
    }
}
