use std::io::{self, Write};

use anyhow::{bail, Context, Result};
use colored::Colorize;

use crate::parser::parse;
use crate::runtime::ffi;
use crate::runtime::jit::executor::JitExecutor;
use crate::runtime::symbol_registry::SymbolRegistry;
use crate::typecheck::TypeChecker;
use ast::nodes::{Expr, Program, Statement};
use lexer::tokenize;

/// REPL engine that maintains state across evaluations
pub struct ReplEngine {
    program: Program,
    symbol_registry: &'static SymbolRegistry,
    executor: Option<JitExecutor>,
    history: Vec<String>,
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
            history: Vec::new(),
        }
    }

    /// Start the REPL loop
    pub fn run(&mut self) -> Result<()> {
        println!("{}", "OtterLang REPL".bold().green());
        println!("Type 'help' for commands, 'exit' to quit.\n");

        loop {
            match self.read_line() {
                Ok(line) => {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }

                    // Handle commands
                    match line {
                        "exit" | "quit" => {
                            println!("Goodbye!");
                            break;
                        }
                        "help" => {
                            self.print_help();
                            continue;
                        }
                        "clear" => {
                            self.program = Program {
                                statements: Vec::new(),
                            };
                            self.executor = None;
                            println!("{}", "Cleared program state.".green());
                            continue;
                        }
                        "history" => {
                            self.print_history();
                            continue;
                        }
                        _ => {}
                    }

                    // Add to history
                    self.history.push(line.to_string());

                    // Try to evaluate
                    match self.evaluate(line) {
                        Ok(Some(output)) => println!("{}", output),
                        Ok(None) => {} // Expression evaluated, no output
                        Err(e) => {
                            eprintln!("{} {}", "error:".red().bold(), e);
                        }
                    }
                }
                Err(e) => {
                    if e.kind() == io::ErrorKind::UnexpectedEof {
                        // Ctrl+D pressed
                        println!("\nGoodbye!");
                        break;
                    }
                    return Err(e.into());
                }
            }
        }

        Ok(())
    }

    fn read_line(&self) -> io::Result<String> {
        print!("{} ", "otter>".green().bold());
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        Ok(line)
    }

    fn evaluate(&mut self, input: &str) -> Result<Option<String>> {
        // Try parsing as statement
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

        // If it's an expression, wrap it in a print statement
        let mut statements = parsed.statements;
        if statements.is_empty() && !tokens.is_empty() {
            // Try parsing as expression
            if let Ok(expr) = self.parse_expression(input) {
                // Create a temporary function that evaluates and prints the expression
                statements.push(Statement::Function(ast::nodes::Function {
                    name: "__repl_expr".to_string(),
                    params: Vec::new(),
                    ret_ty: None,
                    body: ast::nodes::Block {
                        statements: vec![Statement::Expr(expr)],
                    },
                    public: false,
                }));
            }
        }

        let num_statements = statements.len();

        // Add statements to program
        for stmt in statements {
            self.program.statements.push(stmt);
        }

        // Type check
        let mut type_checker = TypeChecker::new()
            .with_registry(crate::runtime::symbol_registry::SymbolRegistry::global());
        if let Err(_) = type_checker.check_program(&self.program) {
            // Remove the statements we just added
            for _ in 0..num_statements {
                self.program.statements.pop();
            }
            bail!("type checking failed");
        }

        // Compile and execute if we have a main function or if we added expressions
        if self.program.statements.iter().any(|s| {
            if let Statement::Function(f) = s {
                f.name == "main" || f.name == "__repl_expr"
            } else {
                false
            }
        }) {
            // Compile the program
            match JitExecutor::new(self.program.clone(), self.symbol_registry) {
                Ok(mut executor) => {
                    // Execute if there's a main
                    if self.program.statements.iter().any(|s| {
                        if let Statement::Function(f) = s {
                            f.name == "main"
                        } else {
                            false
                        }
                    }) {
                        executor.execute_main()?;
                    } else if let Some(Statement::Function(f)) = self.program.statements.last() {
                        if f.name == "__repl_expr" {
                            // Execute the expression function
                            executor.execute_main()?;
                            // Remove the temporary expression function
                            self.program.statements.pop();
                        }
                    }
                    self.executor = Some(executor);
                }
                Err(e) => {
                    // Remove the statements we just added on error
                    for _ in 0..num_statements {
                        self.program.statements.pop();
                    }
                    return Err(e).context("compilation failed");
                }
            }
        }

        Ok(None)
    }

    fn parse_expression(&self, input: &str) -> Result<Expr> {
        // Simple expression parser - for now, just try to parse as a statement
        // and extract the expression from it
        let tokens =
            tokenize(input).map_err(|_| anyhow::anyhow!("failed to tokenize expression"))?;
        let program = parse(&tokens).map_err(|_| anyhow::anyhow!("failed to parse expression"))?;

        if let Some(Statement::Expr(expr)) = program.statements.first() {
            Ok(expr.clone())
        } else {
            bail!("not an expression")
        }
    }

    fn print_help(&self) {
        println!("{}", "OtterLang REPL Commands:".bold());
        println!("  help       - Show this help message");
        println!("  exit/quit - Exit the REPL");
        println!("  clear     - Clear the current program state");
        println!("  history   - Show command history");
        println!();
        println!("Enter OtterLang code to evaluate it.");
        println!("Examples:");
        println!("  let x = 42");
        println!("  fn add(a: i32, b: i32) -> i32:\n    return a + b");
        println!("  print(add(10, 20))");
    }

    fn print_history(&self) {
        if self.history.is_empty() {
            println!("{}", "No history.".dimmed());
            return;
        }
        for (i, line) in self.history.iter().enumerate() {
            println!("{:3}: {}", i + 1, line);
        }
    }
}

impl Default for ReplEngine {
    fn default() -> Self {
        Self::new()
    }
}
