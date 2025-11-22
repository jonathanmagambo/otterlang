use anyhow::{Context, Result, bail};

use crate::runtime::ffi;
use crate::runtime::jit::executor::JitExecutor;
use crate::runtime::symbol_registry::SymbolRegistry;
use crate::typecheck::TypeChecker;
use ast::nodes::{Block, Expr, Function, Node, Program, Statement};
use common::Span;
use lexer::tokenize;
use parser::parse;

/// Result of an evaluation
#[derive(Debug, Clone)]
pub struct EvaluationResult {
    pub output: Option<String>,
    pub kind: EvaluationKind,
}

/// Kind of evaluation result
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluationKind {
    Success,
    Info,
    Error,
}

/// REPL engine that maintains state across evaluations
pub struct ReplEngine {
    program: Program,
    symbol_registry: &'static SymbolRegistry,
    executor: Option<JitExecutor>,
}

impl ReplEngine {
    pub fn new() -> Self {
        ffi::bootstrap_stdlib();
        Self {
            program: Program {
                statements: Vec::new(),
            },
            symbol_registry: ffi::bootstrap_stdlib(),
            executor: None,
        }
    }

    pub fn evaluate(&mut self, input: &str) -> Result<EvaluationResult> {
        if input.trim() == "clear" {
            self.program = Program {
                statements: Vec::new(),
            };
            self.executor = None;
            return Ok(EvaluationResult {
                output: Some("Cleared program state.".to_string()),
                kind: EvaluationKind::Info,
            });
        }
        let tokens = tokenize(input).map_err(|errors| {
            anyhow::anyhow!(
                "lexing error: {}",
                errors
                    .first()
                    .map(|e| format!("{:?}", e))
                    .unwrap_or_default()
            )
        })?;

        let parsed = parse(&tokens).map_err(|errors| {
            anyhow::anyhow!(
                "parsing error: {}",
                errors
                    .first()
                    .map(|e| format!("{:?}", e))
                    .unwrap_or_default()
            )
        })?;

        let mut statements = parsed.statements;
        if statements.is_empty()
            && !tokens.is_empty()
            && let Ok(expr) = self.parse_expression(input)
        {
            let span = Span::new(0, input.len());
            statements.push(Node::new(
                Statement::Function(Node::new(
                    Function {
                        name: "__repl_expr".to_string(),
                        params: Vec::new(),
                        ret_ty: None,
                        body: Node::new(
                            Block {
                                statements: vec![Node::new(
                                    Statement::Expr(Node::new(expr, span)),
                                    span,
                                )],
                            },
                            span,
                        ),
                        public: false,
                    },
                    span,
                )),
                span,
            ));
        }

        let num_statements = statements.len();

        for stmt in statements {
            self.program.statements.push(stmt);
        }

        let mut type_checker = TypeChecker::new()
            .with_registry(crate::runtime::symbol_registry::SymbolRegistry::global());
        if let Err(e) = type_checker.check_program(&self.program) {
            for _ in 0..num_statements {
                self.program.statements.pop();
            }
            return Err(anyhow::anyhow!("type checking failed: {:?}", e));
        }

        if self.program.statements.iter().any(|s| {
            if let Statement::Function(f) = s.as_ref() {
                f.as_ref().name == "main" || f.as_ref().name == "__repl_expr"
            } else {
                false
            }
        }) {
            match JitExecutor::new(self.program.clone(), self.symbol_registry) {
                Ok(mut executor) => {
                    if self.program.statements.iter().any(|s| {
                        if let Statement::Function(f) = s.as_ref() {
                            f.as_ref().name == "main"
                        } else {
                            false
                        }
                    }) {
                        executor.execute_main()?;
                    } else if let Some(stmt) = self.program.statements.last()
                        && let Statement::Function(f) = stmt.as_ref()
                        && f.as_ref().name == "__repl_expr"
                    {
                        executor.execute_main()?;
                        self.program.statements.pop();
                    }
                    self.executor = Some(executor);
                }
                Err(e) => {
                    for _ in 0..num_statements {
                        self.program.statements.pop();
                    }
                    return Err(e).context("compilation failed");
                }
            }
        }

        Ok(EvaluationResult {
            output: None,
            kind: EvaluationKind::Success,
        })
    }

    fn parse_expression(&self, input: &str) -> Result<Expr> {
        let tokens =
            tokenize(input).map_err(|_| anyhow::anyhow!("failed to tokenize expression"))?;
        let program = parse(&tokens).map_err(|_| anyhow::anyhow!("failed to parse expression"))?;

        if let Some(stmt) = program.statements.first()
            && let Statement::Expr(expr) = stmt.as_ref()
        {
            Ok(expr.as_ref().clone())
        } else {
            bail!("not an expression")
        }
    }
}

impl Default for ReplEngine {
    fn default() -> Self {
        Self::new()
    }
}
