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
            self.analyze_function(function);
        }
    }

    pub fn analyze_function(&mut self, function: &Function) {
        let callees = self.extract_callees(&function.body);
        self.calls.insert(function.name.clone(), callees);
    }

    fn extract_callees(&self, block: &Block) -> Vec<String> {
        let mut callees = Vec::new();
        for stmt in &block.statements {
            self.extract_callees_from_stmt(stmt, &mut callees);
        }
        callees
    }

    fn extract_callees_from_stmt(&self, stmt: &Statement, callees: &mut Vec<String>) {
        match stmt {
            Statement::Expr(Expr::Call { func, .. }) => {
                if let Expr::Identifier(name) = func.as_ref() {
                    callees.push(name.clone());
                }
            }
            Statement::If {
                then_block,
                elif_blocks,
                else_block,
                ..
            } => {
                self.extract_callees_from_block(then_block, callees);
                for (_, block) in elif_blocks {
                    self.extract_callees_from_block(block, callees);
                }
                if let Some(block) = else_block {
                    self.extract_callees_from_block(block, callees);
                }
            }
            Statement::For { body, .. } | Statement::While { body, .. } => {
                self.extract_callees_from_block(body, callees);
            }
            _ => {}
        }
    }

    fn extract_callees_from_block(&self, block: &Block, callees: &mut Vec<String>) {
        for stmt in &block.statements {
            self.extract_callees_from_stmt(stmt, callees);
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
}

impl Default for CallGraph {
    fn default() -> Self {
        Self::new()
    }
}
