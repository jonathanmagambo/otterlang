use ast::nodes::{Block, Expr, Function, Program, Statement};

/// Formats OtterLang code
pub struct Formatter {
    indent_size: usize,
}

impl Formatter {
    pub fn new() -> Self {
        Self { indent_size: 4 }
    }

    pub fn with_indent_size(indent_size: usize) -> Self {
        Self { indent_size }
    }

    /// Format a program
    pub fn format_program(&self, program: &Program) -> String {
        let mut output = String::new();
        for (i, stmt) in program.statements.iter().enumerate() {
            if i > 0 {
                output.push('\n');
            }
            output.push_str(&self.format_statement(stmt, 0));
        }
        output
    }

    fn format_statement(&self, stmt: &Statement, indent: usize) -> String {
        match stmt {
            Statement::Let { name, expr, public } => {
                let pub_str = if *public { "pub " } else { "" };
                format!(
                    "{}{}let {} = {}\n",
                    self.indent(indent),
                    pub_str,
                    name,
                    self.format_expr(expr, indent)
                )
            }
            Statement::Assignment { name, expr } => {
                format!(
                    "{}{} = {}\n",
                    self.indent(indent),
                    name,
                    self.format_expr(expr, indent)
                )
            }
            Statement::Function(f) => self.format_function(f, indent),
            Statement::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
            } => self.format_if(cond, then_block, elif_blocks, else_block, indent),
            Statement::For {
                var,
                iterable,
                body,
            } => {
                format!(
                    "{}for {} in {}:\n{}",
                    self.indent(indent),
                    var,
                    self.format_expr(iterable, indent),
                    self.format_block(body, indent + 1)
                )
            }
            Statement::While { cond, body } => {
                format!(
                    "{}while {}:\n{}",
                    self.indent(indent),
                    self.format_expr(cond, indent),
                    self.format_block(body, indent + 1)
                )
            }
            Statement::Return(expr) => {
                if let Some(expr) = expr {
                    format!(
                        "{}return {}\n",
                        self.indent(indent),
                        self.format_expr(expr, indent)
                    )
                } else {
                    format!("{}return\n", self.indent(indent))
                }
            }
            Statement::Break => format!("{}break\n", self.indent(indent)),
            Statement::Continue => format!("{}continue\n", self.indent(indent)),
            Statement::Pass => format!("{}pass\n", self.indent(indent)),
            Statement::Expr(expr) => {
                format!(
                    "{}{}\n",
                    self.indent(indent),
                    self.format_expr(expr, indent)
                )
            }
            Statement::Struct {
                name,
                fields,
                methods,
                public,
                generics,
            } => {
                let pub_str = if *public { "pub " } else { "" };
                let gen_str = if generics.is_empty() {
                    String::new()
                } else {
                    format!("<{}>", generics.join(", "))
                };
                let mut result = format!(
                    "{}{}struct {}{}:\n",
                    self.indent(indent),
                    pub_str,
                    name,
                    gen_str
                );
                for (field_name, field_type) in fields {
                    result.push_str(&format!(
                        "{}    {}: {}\n",
                        self.indent(indent),
                        field_name,
                        self.format_type(field_type)
                    ));
                }
                for method in methods {
                    result.push_str(&self.format_function(method, indent + 1));
                }
                result
            }
            Statement::TypeAlias {
                name,
                target,
                public,
                generics,
            } => {
                let pub_str = if *public { "pub " } else { "" };
                let gen_str = if generics.is_empty() {
                    String::new()
                } else {
                    format!("<{}>", generics.join(", "))
                };
                format!(
                    "{}{}type {}{} = {}\n",
                    self.indent(indent),
                    pub_str,
                    name,
                    gen_str,
                    self.format_type(target)
                )
            }
            Statement::Use { imports } => {
                let modules: Vec<String> = imports
                    .iter()
                    .map(|import| {
                        if let Some(alias) = &import.alias {
                            format!("{} as {}", import.module, alias)
                        } else {
                            import.module.clone()
                        }
                    })
                    .collect();
                format!("{}use {}\n", self.indent(indent), modules.join(", "))
            }
            Statement::Block(block) => self.format_block(block, indent),
            Statement::Try {
                body,
                handlers,
                else_block,
                finally_block,
            } => {
                let mut output = format!("{}try:\n", self.indent(indent));
                output.push_str(&self.format_block(body, indent + 1));

                for handler in handlers {
                    output.push_str(&format!("{}except", self.indent(indent)));

                    if let Some(ty) = &handler.exception {
                        output.push_str(&format!(" {}", self.format_type(ty)));
                    }

                    if let Some(alias) = &handler.alias {
                        output.push_str(&format!(" as {}", alias));
                    }

                    output.push_str(":\n");
                    output.push_str(&self.format_block(&handler.body, indent + 1));
                }

                if let Some(else_block) = else_block {
                    output.push_str(&format!("{}else:\n", self.indent(indent)));
                    output.push_str(&self.format_block(else_block, indent + 1));
                }

                if let Some(finally_block) = finally_block {
                    output.push_str(&format!("{}finally:\n", self.indent(indent)));
                    output.push_str(&self.format_block(finally_block, indent + 1));
                }

                output
            }
            Statement::Raise(expr) => {
                let expr_str = match expr {
                    Some(e) => format!(" {}", self.format_expr(e, indent)),
                    None => String::new(),
                };
                format!("{}raise{}\n", self.indent(indent), expr_str)
            }
        }
    }

    fn format_function(&self, f: &Function, indent: usize) -> String {
        let pub_str = if f.public { "pub " } else { "" };
        let params_str = f
            .params
            .iter()
            .map(|p| {
                let base = if let Some(ref ty) = p.ty {
                    format!("{}: {}", p.name, self.format_type(ty))
                } else {
                    p.name.clone()
                };
                if let Some(default) = &p.default {
                    format!("{} = {}", base, self.format_expr(default, indent))
                } else {
                    base
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let ret_str = if let Some(ref ret_ty) = f.ret_ty {
            format!(" -> {}", self.format_type(ret_ty))
        } else {
            String::new()
        };
        format!(
            "{}{}fn {}({}){}:\n{}",
            self.indent(indent),
            pub_str,
            f.name,
            params_str,
            ret_str,
            self.format_block(&f.body, indent + 1)
        )
    }

    fn format_block(&self, block: &Block, indent: usize) -> String {
        let mut result = String::new();
        for stmt in &block.statements {
            result.push_str(&self.format_statement(stmt, indent));
        }
        result
    }

    fn format_if(
        &self,
        cond: &Box<Expr>,
        then_block: &Block,
        elif_blocks: &[(Expr, Block)],
        else_block: &Option<Block>,
        indent: usize,
    ) -> String {
        let mut result = format!(
            "{}if {}:\n{}",
            self.indent(indent),
            self.format_expr(cond, indent),
            self.format_block(then_block, indent + 1)
        );
        for (elif_cond, elif_block) in elif_blocks {
            result.push_str(&format!(
                "{}elif {}:\n{}",
                self.indent(indent),
                self.format_expr(elif_cond, indent),
                self.format_block(elif_block, indent + 1)
            ));
        }
        if let Some(else_block) = else_block {
            result.push_str(&format!(
                "{}else:\n{}",
                self.indent(indent),
                self.format_block(else_block, indent + 1)
            ));
        }
        result
    }

    fn format_expr(&self, expr: &Expr, indent: usize) -> String {
        match expr {
            Expr::Literal(lit) => self.format_literal(lit),
            Expr::Identifier(name) => name.clone(),
            Expr::Binary { op, left, right } => {
                format!(
                    "{} {} {}",
                    self.format_expr(left, indent),
                    self.format_binary_op(op),
                    self.format_expr(right, indent)
                )
            }
            Expr::Unary { op, expr } => {
                format!(
                    "{}{}",
                    self.format_unary_op(op),
                    self.format_expr(expr, indent)
                )
            }
            Expr::Call { func, args } => {
                let args_str = args
                    .iter()
                    .map(|arg| self.format_expr(arg, indent))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}({})", self.format_expr(func, indent), args_str)
            }
            Expr::Member { object, field } => {
                format!("{}.{}", self.format_expr(object, indent), field)
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let else_str = if let Some(else_expr) = else_branch {
                    format!(" else {}", self.format_expr(else_expr, indent))
                } else {
                    String::new()
                };
                format!(
                    "{} if {} else {}",
                    self.format_expr(then_branch, indent),
                    self.format_expr(cond, indent),
                    else_str
                )
            }
            Expr::Range { start, end } => {
                format!(
                    "{}..{}",
                    self.format_expr(start, indent),
                    self.format_expr(end, indent)
                )
            }
            Expr::Array(elements) => {
                let elements_str = elements
                    .iter()
                    .map(|e| self.format_expr(e, indent))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("[{}]", elements_str)
            }
            Expr::Dict(pairs) => {
                let pairs_str = pairs
                    .iter()
                    .map(|(k, v)| {
                        format!(
                            "{}: {}",
                            self.format_expr(k, indent),
                            self.format_expr(v, indent)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{{}}}", pairs_str)
            }
            Expr::ListComprehension {
                element,
                var,
                iterable,
                condition,
            } => {
                let cond_str = condition
                    .as_ref()
                    .map(|cond| format!(" if {}", self.format_expr(cond, indent)))
                    .unwrap_or_default();
                format!(
                    "[{} for {} in {}{}]",
                    self.format_expr(element, indent),
                    var,
                    self.format_expr(iterable, indent),
                    cond_str
                )
            }
            Expr::DictComprehension {
                key,
                value,
                var,
                iterable,
                condition,
            } => {
                let cond_str = condition
                    .as_ref()
                    .map(|cond| format!(" if {}", self.format_expr(cond, indent)))
                    .unwrap_or_default();
                format!(
                    "{{{}: {} for {} in {}{}}}",
                    self.format_expr(key, indent),
                    self.format_expr(value, indent),
                    var,
                    self.format_expr(iterable, indent),
                    cond_str
                )
            }
            Expr::Match { value, arms } => {
                let mut result = format!("match {}:\n", self.format_expr(value, indent));
                for arm in arms {
                    result.push_str(&format!(
                        "{}    case {} => {}\n",
                        self.indent(indent),
                        self.format_pattern(&arm.pattern),
                        self.format_expr(&arm.body, indent)
                    ));
                }
                result
            }
            Expr::Struct { name, fields } => {
                // Pythonic style: Point(x=1.0, y=2.0)
                let fields_str = fields
                    .iter()
                    .map(|(fname, val)| format!("{}={}", fname, self.format_expr(val, indent)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}({})", name, fields_str)
            }
            Expr::Lambda {
                params,
                ret_ty,
                body,
            } => {
                let params_str = params
                    .iter()
                    .map(|p| {
                        let base = if let Some(ref ty) = p.ty {
                            format!("{}: {}", p.name, self.format_type(ty))
                        } else {
                            p.name.clone()
                        };
                        if let Some(default) = &p.default {
                            format!("{} = {}", base, self.format_expr(default, indent))
                        } else {
                            base
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let ret_str = if let Some(ret_ty_val) = ret_ty {
                    format!(" -> {}", self.format_type(ret_ty_val))
                } else {
                    String::new()
                };
                format!(
                    "fn({}){} => {}",
                    params_str,
                    ret_str,
                    self.format_block(body, indent + 1)
                )
            }
            Expr::Await(expr) => format!("await {}", self.format_expr(expr, indent)),
            Expr::Spawn(expr) => format!("spawn {}", self.format_expr(expr, indent)),
            Expr::FString { parts } => {
                let parts_str = parts
                    .iter()
                    .map(|part| match part {
                        ast::nodes::FStringPart::Text(s) => s.clone(),
                        ast::nodes::FStringPart::Expr(e) => {
                            format!("{{{}}}", self.format_expr(e, indent))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("");
                format!("f\"{}\"", parts_str)
            }
        }
    }

    fn format_pattern(&self, pattern: &ast::nodes::Pattern) -> String {
        match pattern {
            ast::nodes::Pattern::Wildcard => "_".to_string(),
            ast::nodes::Pattern::Literal(lit) => self.format_literal(lit),
            ast::nodes::Pattern::Identifier(name) => name.clone(),
            ast::nodes::Pattern::Struct { name, fields } => {
                let fields_str = fields
                    .iter()
                    .map(|(f, p_opt)| {
                        if let Some(p) = p_opt {
                            format!("{}: {}", f, self.format_pattern(p))
                        } else {
                            f.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}({})", name, fields_str)
            }
            ast::nodes::Pattern::Array { patterns, rest } => {
                let patterns_str = patterns
                    .iter()
                    .map(|p| self.format_pattern(p))
                    .collect::<Vec<_>>()
                    .join(", ");
                let rest_str = if let Some(rest_var) = rest {
                    format!(", ..{}", rest_var)
                } else {
                    String::new()
                };
                format!("[{}{}]", patterns_str, rest_str)
            }
        }
    }

    fn format_literal(&self, lit: &ast::nodes::Literal) -> String {
        match lit {
            ast::nodes::Literal::Number(n) => {
                if !n.is_float_literal && n.value.fract() == 0.0 {
                    format!("{}", n.value as i64)
                } else {
                    n.value.to_string()
                }
            }
            ast::nodes::Literal::Bool(b) => b.to_string(),
            ast::nodes::Literal::String(s) => format!("\"{}\"", s),
            ast::nodes::Literal::None => "None".to_string(),
            ast::nodes::Literal::Unit => "()".to_string(),
        }
    }

    fn format_type(&self, ty: &ast::nodes::Type) -> String {
        match ty {
            ast::nodes::Type::Simple(name) => name.clone(),
            ast::nodes::Type::Generic { base, args } => {
                if args.is_empty() {
                    base.clone()
                } else {
                    let args_str = args
                        .iter()
                        .map(|g| self.format_type(g))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{}<{}>", base, args_str)
                }
            }
            ast::nodes::Type::Option(inner) => {
                format!("Option<{}>", self.format_type(inner))
            }
            ast::nodes::Type::Result { ok, err } => {
                format!(
                    "Result<{}, {}>",
                    self.format_type(ok),
                    self.format_type(err)
                )
            }
        }
    }

    fn format_binary_op(&self, op: &ast::nodes::BinaryOp) -> &str {
        match op {
            ast::nodes::BinaryOp::Add => "+",
            ast::nodes::BinaryOp::Mul => "*",
            ast::nodes::BinaryOp::Sub => "-",
            ast::nodes::BinaryOp::Div => "/",
            ast::nodes::BinaryOp::Mod => "%",
            ast::nodes::BinaryOp::Eq => "==",
            ast::nodes::BinaryOp::Ne => "!=",
            ast::nodes::BinaryOp::Lt => "<",
            ast::nodes::BinaryOp::LtEq => "<=",
            ast::nodes::BinaryOp::Gt => ">",
            ast::nodes::BinaryOp::GtEq => ">=",
            ast::nodes::BinaryOp::Is => "is",
            ast::nodes::BinaryOp::IsNot => "is not",
            ast::nodes::BinaryOp::And => "and",
            ast::nodes::BinaryOp::Or => "or",
        }
    }

    fn format_unary_op(&self, op: &ast::nodes::UnaryOp) -> &str {
        match op {
            ast::nodes::UnaryOp::Not => "not ",
            ast::nodes::UnaryOp::Neg => "-",
        }
    }

    fn indent(&self, level: usize) -> String {
        " ".repeat(level * self.indent_size)
    }
}

impl Default for Formatter {
    fn default() -> Self {
        Self::new()
    }
}
