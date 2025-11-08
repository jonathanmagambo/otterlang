use chumsky::prelude::*;
use chumsky::Stream;

use ast::nodes::{
    BinaryOp, Block, ExceptHandler, Expr, FStringPart, Function, Literal, MatchArm, NumberLiteral,
    Param, Pattern, Program, Statement, Type, UnaryOp,
};

use common::Span;
use lexer::token::{Token, TokenKind};
use utils::errors::{Diagnostic, DiagnosticSeverity};

#[derive(Debug, Clone)]
pub struct ParserError {
    pub message: String,
    pub span: Span,
}

impl ParserError {
    pub fn to_diagnostic(&self, source_id: &str) -> Diagnostic {
        let mut diag = Diagnostic::new(
            DiagnosticSeverity::Error,
            source_id,
            self.span,
            self.message.clone(),
        );

        // Add suggestions based on error message
        if self.message.contains("unexpected token") {
            diag = diag.with_suggestion("Check for missing or extra tokens, or syntax errors")
                .with_help("Ensure all statements are properly terminated and parentheses/brackets are balanced.");
        } else if self.message.contains("unexpected end of input") {
            diag = diag
                .with_suggestion("Check for missing closing brackets, parentheses, or quotes")
                .with_help("The parser reached the end of the file while expecting more tokens.");
        }

        diag
    }
}

impl From<Simple<TokenKind>> for ParserError {
    fn from(value: Simple<TokenKind>) -> Self {
        let span_range = value.span();
        let span = Span::new(span_range.start, span_range.end);
        let message = if let Some(found) = value.found() {
            format!("unexpected token: {:?}", found)
        } else {
            "unexpected end of input".to_string()
        };
        Self { message, span }
    }
}

pub fn parse(tokens: &[Token]) -> Result<Program, Vec<ParserError>> {
    let parser = program_parser();
    let eof_span = tokens
        .last()
        .map(|token| token.span)
        .unwrap_or_else(|| Span::new(0, 0));

    let end = eof_span.end();
    let stream = Stream::from_iter(
        end..end + 1,
        tokens
            .iter()
            .cloned()
            .map(|token| (token.kind, token.span.into())),
    );

    parser
        .parse(stream)
        .map_err(|errors| errors.into_iter().map(ParserError::from).collect())
}

fn identifier_parser() -> impl Parser<TokenKind, String, Error = Simple<TokenKind>> {
    select! { TokenKind::Identifier(name) => name }
}

fn identifier_or_keyword_parser() -> impl Parser<TokenKind, String, Error = Simple<TokenKind>> {
    select! {
        TokenKind::Identifier(name) => name,
        TokenKind::Def => "def".to_string(),
        TokenKind::Lambda => "lambda".to_string(),
        TokenKind::Return => "return".to_string(),
        TokenKind::If => "if".to_string(),
        TokenKind::Else => "else".to_string(),
        TokenKind::Elif => "elif".to_string(),
        TokenKind::For => "for".to_string(),
        TokenKind::While => "while".to_string(),
        TokenKind::Break => "break".to_string(),
        TokenKind::Continue => "continue".to_string(),
        TokenKind::Pass => "pass".to_string(),
        TokenKind::In => "in".to_string(),
        TokenKind::Is => "is".to_string(),
        TokenKind::Not => "not".to_string(),
        TokenKind::Use => "use".to_string(),
        TokenKind::From => "from".to_string(),
        TokenKind::As => "as".to_string(),
        TokenKind::Async => "async".to_string(),
        TokenKind::Await => "await".to_string(),
        TokenKind::Spawn => "spawn".to_string(),
        TokenKind::Match => "match".to_string(),
        TokenKind::Case => "case".to_string(),
        TokenKind::True => "true".to_string(),
        TokenKind::False => "false".to_string(),
        TokenKind::Print => "print".to_string(),
        TokenKind::None => "None".to_string(),
    }
}

fn type_parser() -> impl Parser<TokenKind, Type, Error = Simple<TokenKind>> {
    recursive(|ty| {
        identifier_parser()
            .then(
                ty.separated_by(just(TokenKind::Comma))
                    .allow_trailing()
                    .delimited_by(just(TokenKind::Lt), just(TokenKind::Gt))
                    .or_not(),
            )
            .map(|(base, args)| match args {
                Some(args) => Type::Generic { base, args },
                None => Type::Simple(base),
            })
    })
}

fn parse_fstring(content: String) -> Expr {
    use chumsky::Parser;

    // Parse f-string by splitting on braces and parsing expressions
    let mut parts = Vec::new();
    let mut current_text = String::new();
    let mut chars = content.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '{' => {
                if let Some(&'{') = chars.peek() {
                    // Escaped {{
                    chars.next();
                    current_text.push('{');
                } else {
                    // Expression start
                    if !current_text.is_empty() {
                        parts.push(FStringPart::Text(current_text));
                        current_text = String::new();
                    }

                    // Parse expression until }
                    let mut expr_content = String::new();
                    while let Some(ch) = chars.next() {
                        if ch == '}' {
                            break;
                        }
                        expr_content.push(ch);
                    }

                    if !expr_content.is_empty() {
                        // Parse the expression content using the full expression parser
                        let trimmed = expr_content.trim();
                        if !trimmed.is_empty() {
                            match lexer::tokenize(trimmed) {
                                Ok(tokens) => {
                                    // Create a stream from tokens for the parser
                                    use chumsky::Stream;
                                    let end_span = tokens.last().map(|t| t.span.end()).unwrap_or(0);
                                    let stream = Stream::from_iter(
                                        end_span..end_span + 1,
                                        tokens
                                            .iter()
                                            .map(|token| (token.kind.clone(), token.span.into())),
                                    );
                                    match expr_parser().parse(stream) {
                                        Ok(expr) => {
                                            parts.push(FStringPart::Expr(Box::new(expr)));
                                        }
                                        Err(_) => {
                                            // Fallback to simple identifier if parsing fails
                                            parts.push(FStringPart::Expr(Box::new(
                                                Expr::Identifier(trimmed.to_string()),
                                            )));
                                        }
                                    }
                                }
                                Err(_) => {
                                    // Fallback to simple identifier if tokenization fails
                                    parts.push(FStringPart::Expr(Box::new(Expr::Identifier(
                                        trimmed.to_string(),
                                    ))));
                                }
                            }
                        }
                    }
                }
            }
            '}' => {
                if let Some(&'}') = chars.peek() {
                    // Escaped }}
                    chars.next();
                    current_text.push('}');
                } else {
                    current_text.push('}');
                }
            }
            _ => current_text.push(ch),
        }
    }

    // Add remaining text
    if !current_text.is_empty() {
        parts.push(FStringPart::Text(current_text));
    }

    // If no expressions found, treat as regular string
    if parts
        .iter()
        .all(|part| matches!(part, FStringPart::Text(_)))
    {
        if let Some(FStringPart::Text(text)) = parts.first() {
            return Expr::Literal(Literal::String(text.clone()));
        }
    }

    Expr::FString { parts }
}

fn literal_expr_parser() -> impl Parser<TokenKind, Expr, Error = Simple<TokenKind>> {
    let string_lit =
        select! { TokenKind::StringLiteral(value) => Expr::Literal(Literal::String(value)) };
    let number_lit = select! { TokenKind::Number(value) => {
        // Remove underscores from the number
        let clean_value = value.replace('_', "");
        let is_float_literal = value.contains('.') || value.contains('e') || value.contains('E');
        // Check if it contains a decimal point or is an integer
        if clean_value.contains('.') {
            Expr::Literal(Literal::Number(NumberLiteral::new(
                clean_value.parse().unwrap_or_default(),
                true,
            )))
        } else {
            // Parse as integer
            match clean_value.parse::<i64>() {
                Ok(int_val) => Expr::Literal(Literal::Number(NumberLiteral::new(int_val as f64, is_float_literal))),
                Err(_) => Expr::Literal(Literal::Number(NumberLiteral::new(0.0, is_float_literal))),
            }
        }
    }};
    let bool_lit = select! {
        TokenKind::True => Expr::Literal(Literal::Bool(true)),
        TokenKind::False => Expr::Literal(Literal::Bool(false)),
    };
    let none_lit = just(TokenKind::None).to(Expr::Literal(Literal::None));
    let fstring_lit = select! { TokenKind::FString(content) => parse_fstring(content) };
    let unit_lit = just(TokenKind::LParen)
        .then(just(TokenKind::RParen))
        .map(|_| Expr::Literal(Literal::Unit));
    choice((
        fstring_lit,
        string_lit,
        number_lit,
        bool_lit,
        none_lit,
        unit_lit,
    ))
}

fn expr_parser() -> impl Parser<TokenKind, Expr, Error = Simple<TokenKind>> {
    recursive(|expr| {
        let lambda_param = identifier_parser()
            .then(just(TokenKind::Colon).ignore_then(type_parser()).or_not())
            .then(just(TokenKind::Equals).ignore_then(expr.clone()).or_not())
            .map(|((name, ty), default)| Param::new(name, ty, default));

        let lambda_params = lambda_param
            .separated_by(just(TokenKind::Comma))
            .allow_trailing()
            .delimited_by(just(TokenKind::LParen), just(TokenKind::RParen))
            .or_not()
            .map(|params| params.unwrap_or_default());

        let lambda_ret_type = just(TokenKind::Arrow).ignore_then(type_parser()).or_not();

        let lambda_block = recursive(|_block| {
            let lambda_stmt = recursive(|_stmt| {
                // Simplified statement parser for lambdas - just expressions and returns
                let lambda_return_stmt = just(TokenKind::Return)
                    .ignore_then(expr.clone().or_not())
                    .map(Statement::Return);

                choice((lambda_return_stmt, expr.clone().map(Statement::Expr)))
                    .then_ignore(just(TokenKind::Newline).or_not())
                    .boxed()
            });

            lambda_stmt.repeated().at_least(1).map(Block::new)
        });

        // Pythonic: only 'lambda' for lambda expressions
        let lambda_keyword = just(TokenKind::Lambda);

        let lambda_expr = lambda_keyword
            .ignore_then(lambda_params)
            .then(lambda_ret_type)
            .then_ignore(just(TokenKind::Colon))
            .then(
                just(TokenKind::Newline).ignore_then(lambda_block).or(expr
                    .clone()
                    .map(|expr| Block::new(vec![Statement::Expr(expr)]))),
            )
            .map(|((params, ret_ty), body)| Expr::Lambda {
                params,
                ret_ty,
                body,
            });

        // Pythonic struct initialization: Point(x=1.0, y=2.0)
        let struct_init_pythonic = identifier_parser()
            .then(
                // Keyword argument: name=value
                identifier_parser()
                    .then_ignore(just(TokenKind::Equals))
                    .then(expr.clone())
                    .separated_by(just(TokenKind::Comma))
                    .allow_trailing()
                    .delimited_by(just(TokenKind::LParen), just(TokenKind::RParen)),
            )
            .map(|(name, fields)| Expr::Struct {
                name,
                fields: fields
                    .into_iter()
                    .map(|(fname, val)| (fname, val))
                    .collect(),
            });

        let list_comprehension = expr
            .clone()
            .then_ignore(just(TokenKind::For))
            .then(identifier_parser())
            .then_ignore(just(TokenKind::In))
            .then(expr.clone())
            .then(just(TokenKind::If).ignore_then(expr.clone()).or_not())
            .map(
                |(((element, var), iterable), condition)| Expr::ListComprehension {
                    element: Box::new(element),
                    var,
                    iterable: Box::new(iterable),
                    condition: condition.map(Box::new),
                },
            )
            .delimited_by(just(TokenKind::LBracket), just(TokenKind::RBracket));

        let dict_comprehension = expr
            .clone()
            .then_ignore(just(TokenKind::Colon))
            .then(expr.clone())
            .then_ignore(just(TokenKind::For))
            .then(identifier_parser())
            .then_ignore(just(TokenKind::In))
            .then(expr.clone())
            .then(just(TokenKind::If).ignore_then(expr.clone()).or_not())
            .map(
                |((((key, value), var), iterable), condition)| Expr::DictComprehension {
                    key: Box::new(key),
                    value: Box::new(value),
                    var,
                    iterable: Box::new(iterable),
                    condition: condition.map(Box::new),
                },
            )
            .delimited_by(just(TokenKind::LBrace), just(TokenKind::RBrace));

        let atom = choice((
            literal_expr_parser(),
            lambda_expr,
            struct_init_pythonic,
            identifier_parser().map(Expr::Identifier),
            expr.clone()
                .delimited_by(just(TokenKind::LParen), just(TokenKind::RParen)),
            list_comprehension,
            // Array literal [expr, expr, ...]
            expr.clone()
                .separated_by(just(TokenKind::Comma))
                .allow_trailing()
                .delimited_by(just(TokenKind::LBracket), just(TokenKind::RBracket))
                .map(Expr::Array),
            dict_comprehension,
            // Dictionary literal {key: value, ...}
            expr.clone()
                .then_ignore(just(TokenKind::Colon))
                .then(expr.clone())
                .separated_by(just(TokenKind::Comma))
                .allow_trailing()
                .delimited_by(just(TokenKind::LBrace), just(TokenKind::RBrace))
                .map(|pairs| Expr::Dict(pairs)),
        ))
        .boxed();

        let member_access = atom
            .clone()
            .then(
                just(TokenKind::Dot)
                    .ignore_then(identifier_or_keyword_parser())
                    .repeated(),
            )
            .foldl(|object, field| Expr::Member {
                object: Box::new(object),
                field,
            })
            .boxed();

        let call_suffix = just(TokenKind::LParen)
            .ignore_then(
                expr.clone()
                    .separated_by(just(TokenKind::Comma))
                    .allow_trailing()
                    .or_not()
                    .map(|args| args.unwrap_or_default()),
            )
            .then_ignore(just(TokenKind::RParen));

        let call = member_access
            .clone()
            .then(call_suffix.repeated())
            .foldl(|func, args| Expr::Call {
                func: Box::new(func),
                args,
            })
            .boxed();

        let await_expr = just(TokenKind::Await)
            .ignore_then(call.clone())
            .map(|expr| Expr::Await(Box::new(expr)));

        let spawn_expr = just(TokenKind::Spawn)
            .ignore_then(call.clone())
            .map(|expr| Expr::Spawn(Box::new(expr)));

        let unary = choice((
            just(TokenKind::Minus).to(UnaryOp::Neg),
            just(TokenKind::Bang).to(UnaryOp::Not),
            just(TokenKind::Not).to(UnaryOp::Not),
        ))
        .then(choice((
            await_expr.clone(),
            spawn_expr.clone(),
            call.clone(),
        )))
        .map(|(op, expr)| Expr::Unary {
            op,
            expr: Box::new(expr),
        })
        .or(await_expr)
        .or(spawn_expr)
        .or(call.clone())
        .boxed();

        let product = unary
            .clone()
            .then(
                choice((
                    just(TokenKind::Star).to(BinaryOp::Mul),
                    just(TokenKind::Slash).to(BinaryOp::Div),
                    just(TokenKind::Percent).to(BinaryOp::Mod),
                ))
                .then(unary.clone())
                .repeated(),
            )
            .foldl(|left, (op, right)| Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
            });

        let sum = product
            .clone()
            .then(
                choice((
                    just(TokenKind::Plus).to(BinaryOp::Add),
                    just(TokenKind::Minus).to(BinaryOp::Sub),
                ))
                .then(product)
                .repeated(),
            )
            .foldl(|left, (op, right)| Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
            });

        let range = sum
            .clone()
            .then(just(TokenKind::DoubleDot).ignore_then(sum.clone()).or_not())
            .map(|(start, end)| {
                if let Some(end) = end {
                    Expr::Range {
                        start: Box::new(start),
                        end: Box::new(end),
                    }
                } else {
                    start
                }
            });

        let is_operator = just(TokenKind::Is)
            .ignore_then(just(TokenKind::Not).or_not())
            .map(|not_opt| {
                if not_opt.is_some() {
                    BinaryOp::IsNot
                } else {
                    BinaryOp::Is
                }
            });

        let comparison_op = choice((
            just(TokenKind::EqEq).to(BinaryOp::Eq),
            just(TokenKind::Neq).to(BinaryOp::Ne),
            just(TokenKind::Lt).to(BinaryOp::Lt),
            just(TokenKind::Gt).to(BinaryOp::Gt),
            just(TokenKind::LtEq).to(BinaryOp::LtEq),
            just(TokenKind::GtEq).to(BinaryOp::GtEq),
            is_operator,
        ));

        let comparison = range
            .clone()
            .then(comparison_op.then(range.clone()).repeated())
            .foldl(|left, (op, right)| Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
            });

        let logical = comparison
            .clone()
            .then(
                choice((
                    just(TokenKind::Amp).to(BinaryOp::And),
                    just(TokenKind::Pipe).to(BinaryOp::Or),
                ))
                .then(comparison)
                .repeated(),
            )
            .foldl(|left, (op, right)| Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
            });

        // Pythonic match expression: match expr: case pattern: body
        let newline = just(TokenKind::Newline).repeated().at_least(1);
        let match_expr = just(TokenKind::Match)
            .ignore_then(logical.clone())
            .then(
                just(TokenKind::Colon)
                    .ignore_then(newline.clone())
                    .ignore_then(just(TokenKind::Indent))
                    .ignore_then(
                        just(TokenKind::Case)
                            .ignore_then(pattern_parser(expr.clone()))
                            .then_ignore(just(TokenKind::Colon))
                            .then_ignore(newline.clone())
                            .then(
                                logical
                                    .clone()
                                    .then_ignore(newline.clone())
                                    .repeated()
                                    .at_least(1)
                                    .map(|exprs| {
                                        // Take the last expression as the body
                                        exprs
                                            .last()
                                            .cloned()
                                            .unwrap_or_else(|| Expr::Literal(Literal::Unit))
                                    }),
                            )
                            .map(|(pattern, body)| MatchArm {
                                pattern,
                                guard: None,
                                body,
                            })
                            .separated_by(newline.clone())
                            .at_least(1),
                    )
                    .then_ignore(just(TokenKind::Dedent)),
            )
            .map(|(value, arms)| Expr::Match {
                value: Box::new(value),
                arms,
            })
            .or(logical);

        match_expr
    })
}

/// Pattern parser for match expressions
fn pattern_parser(
    _expr: Recursive<'_, TokenKind, Expr, Simple<TokenKind>>,
) -> impl Parser<TokenKind, Pattern, Error = Simple<TokenKind>> {
    recursive(|pattern| {
        let wildcard = just(TokenKind::Identifier("_".to_string())).map(|_| Pattern::Wildcard);

        let literal_pattern = literal_expr_parser().map(|expr| match expr {
            Expr::Literal(lit) => Pattern::Literal(lit),
            _ => Pattern::Wildcard, // Fallback
        });

        let identifier_pattern = identifier_parser().map(Pattern::Identifier);

        let struct_pattern = identifier_parser()
            .then(
                just(TokenKind::LBrace)
                    .ignore_then(
                        identifier_parser()
                            .then(just(TokenKind::Colon).ignore_then(pattern.clone()).or_not())
                            .separated_by(just(TokenKind::Comma))
                            .allow_trailing(),
                    )
                    .then_ignore(just(TokenKind::RBrace))
                    .or_not(),
            )
            .map(|(name, fields)| Pattern::Struct {
                name,
                fields: fields
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(fname, pat)| (fname, pat))
                    .collect(),
            });

        let array_pattern = pattern
            .clone()
            .separated_by(just(TokenKind::Comma))
            .allow_trailing()
            .delimited_by(just(TokenKind::LBracket), just(TokenKind::RBracket))
            .then(
                just(TokenKind::DoubleDot)
                    .ignore_then(identifier_parser())
                    .or_not(),
            )
            .map(|(patterns, rest)| Pattern::Array { patterns, rest });

        choice((
            wildcard,
            literal_pattern,
            struct_pattern,
            array_pattern,
            identifier_pattern,
        ))
    })
}

fn program_parser() -> impl Parser<TokenKind, Program, Error = Simple<TokenKind>> {
    let newline = just(TokenKind::Newline).repeated().at_least(1);
    let expr = expr_parser().boxed();

    let print_stmt = just(TokenKind::Print)
        .ignore_then(
            expr.clone()
                .delimited_by(just(TokenKind::LParen), just(TokenKind::RParen)),
        )
        .map(|arg| {
            Statement::Expr(Expr::Call {
                func: Box::new(Expr::Identifier("print".to_string())),
                args: vec![arg],
            })
        });

    let return_stmt = just(TokenKind::Return)
        .ignore_then(expr.clone().or_not())
        .map(Statement::Return);

    let pub_keyword = just(TokenKind::Pub).or_not();

    // Support both direct assignment (Pythonic) and explicit let declarations
    let let_stmt = pub_keyword
        .clone()
        .then(just(TokenKind::Let).or_not())
        .then(identifier_parser())
        .then_ignore(just(TokenKind::Equals))
        .then(expr.clone())
        .map(|(((pub_kw, _let), name), expr)| Statement::Let {
            name,
            expr,
            public: pub_kw.is_some(),
        });

    // Augmented assignments only (Pythonic: x += y, x -= y, etc.)
    // Simple assignments (=) are handled by let_stmt (declaration or reassignment)
    let assignment_stmt = identifier_parser()
        .then(choice((
            just(TokenKind::PlusEq).to(BinaryOp::Add),
            just(TokenKind::MinusEq).to(BinaryOp::Sub),
            just(TokenKind::StarEq).to(BinaryOp::Mul),
            just(TokenKind::SlashEq).to(BinaryOp::Div),
        )))
        .then(expr.clone())
        .map(|((name, op), rhs)| {
            // Desugar: x += y becomes x = x + y
            let expr = Expr::Binary {
                op,
                left: Box::new(Expr::Identifier(name.clone())),
                right: Box::new(rhs),
            };
            Statement::Assignment { name, expr }
        });

    let path_segment = choice((
        just(TokenKind::Dot).to(".".to_string()),
        just(TokenKind::DoubleDot).to("..".to_string()),
        identifier_parser(),
    ))
    .boxed();

    let path_separator = choice((
        just(TokenKind::Slash).to("/".to_string()),
        just(TokenKind::Colon).to(":".to_string()),
    ));

    let module_path = path_segment
        .clone()
        .then(path_separator.then(path_segment.clone()).repeated())
        .map(|(first, rest)| {
            let mut module = first;
            for (sep, segment) in rest {
                module.push_str(&sep);
                module.push_str(&segment);
            }
            module
        });

    let use_stmt = just(TokenKind::Use)
        .ignore_then(module_path)
        .then(
            just(TokenKind::As)
                .ignore_then(identifier_parser())
                .or_not(),
        )
        .map(|(module, alias)| Statement::Use { module, alias });

    let break_stmt = just(TokenKind::Break).map(|_| Statement::Break);
    let continue_stmt = just(TokenKind::Continue).map(|_| Statement::Continue);
    let pass_stmt = just(TokenKind::Pass).map(|_| Statement::Pass);

    // Create a recursive parser for statements
    let statement = recursive(|stmt| {
        let elif_block = just(TokenKind::Elif)
            .ignore_then(expr.clone())
            .then_ignore(just(TokenKind::Colon))
            .then_ignore(newline.clone())
            .then(
                stmt.clone()
                    .repeated()
                    .at_least(1)
                    .delimited_by(just(TokenKind::Indent), just(TokenKind::Dedent))
                    .map(Block::new),
            )
            .map(|(cond, block)| (cond, block));

        let if_stmt = just(TokenKind::If)
            .ignore_then(expr.clone())
            .then_ignore(just(TokenKind::Colon))
            .then_ignore(newline.clone())
            .then(
                stmt.clone()
                    .repeated()
                    .at_least(1)
                    .delimited_by(just(TokenKind::Indent), just(TokenKind::Dedent))
                    .map(Block::new),
            )
            .then(elif_block.repeated())
            .then(
                just(TokenKind::Else)
                    .ignore_then(just(TokenKind::Colon))
                    .ignore_then(newline.clone())
                    .then(
                        stmt.clone()
                            .repeated()
                            .at_least(1)
                            .delimited_by(just(TokenKind::Indent), just(TokenKind::Dedent))
                            .map(Block::new),
                    )
                    .or_not(),
            )
            .map(
                |(((cond, then_block), elif_blocks), else_block)| Statement::If {
                    cond: Box::new(cond),
                    then_block,
                    elif_blocks,
                    else_block: else_block.map(|(_, block)| block),
                },
            );

        let for_stmt = just(TokenKind::For)
            .ignore_then(identifier_parser())
            .then_ignore(just(TokenKind::In))
            .then(expr.clone())
            .then_ignore(just(TokenKind::Colon))
            .then_ignore(newline.clone())
            .then(
                stmt.clone()
                    .repeated()
                    .at_least(1)
                    .delimited_by(just(TokenKind::Indent), just(TokenKind::Dedent))
                    .map(Block::new),
            )
            .map(|((var, iterable), body)| Statement::For {
                var,
                iterable,
                body,
            });

        let while_stmt = just(TokenKind::While)
            .ignore_then(expr.clone())
            .then_ignore(just(TokenKind::Colon))
            .then_ignore(newline.clone())
            .then(
                stmt.clone()
                    .repeated()
                    .at_least(1)
                    .delimited_by(just(TokenKind::Indent), just(TokenKind::Dedent))
                    .map(Block::new),
            )
            .map(|(cond, body)| Statement::While { cond, body });

        let except_handler = just(TokenKind::Except)
            .ignore_then(
                // Optional type specification
                type_parser().or_not(),
            )
            .then(
                // Optional 'as name' clause
                just(TokenKind::As)
                    .ignore_then(identifier_parser())
                    .or_not(),
            )
            .then_ignore(just(TokenKind::Colon))
            .then_ignore(newline.clone())
            .then(
                stmt.clone()
                    .repeated()
                    .at_least(1)
                    .delimited_by(just(TokenKind::Indent), just(TokenKind::Dedent))
                    .map(Block::new),
            )
            .map(|((exception_type, alias), body)| ExceptHandler::new(exception_type, alias, body));

        let try_stmt = just(TokenKind::Try)
            .ignore_then(just(TokenKind::Colon))
            .ignore_then(newline.clone())
            .then(
                stmt.clone()
                    .repeated()
                    .at_least(1)
                    .delimited_by(just(TokenKind::Indent), just(TokenKind::Dedent))
                    .map(Block::new),
            )
            .then(except_handler.repeated())
            .then(
                just(TokenKind::Else)
                    .ignore_then(just(TokenKind::Colon))
                    .ignore_then(newline.clone())
                    .then(
                        stmt.clone()
                            .repeated()
                            .at_least(1)
                            .delimited_by(just(TokenKind::Indent), just(TokenKind::Dedent))
                            .map(Block::new),
                    )
                    .or_not(),
            )
            .then(
                just(TokenKind::Finally)
                    .ignore_then(just(TokenKind::Colon))
                    .ignore_then(newline.clone())
                    .then(
                        stmt.clone()
                            .repeated()
                            .at_least(1)
                            .delimited_by(just(TokenKind::Indent), just(TokenKind::Dedent))
                            .map(Block::new),
                    )
                    .or_not(),
            )
            .map(
                |((((_try, body), handlers), else_block), finally_block)| Statement::Try {
                    body,
                    handlers,
                    else_block: else_block.map(|(_else, block)| block),
                    finally_block: finally_block.map(|(_finally, block)| block),
                },
            );

        let raise_stmt = just(TokenKind::Raise)
            .ignore_then(expr.clone().or_not())
            .map(Statement::Raise);

        choice((
            print_stmt,
            return_stmt,
            let_stmt,
            assignment_stmt,
            use_stmt,
            if_stmt,
            for_stmt,
            while_stmt,
            break_stmt,
            continue_stmt,
            pass_stmt,
            try_stmt,
            raise_stmt,
            expr.clone().map(Statement::Expr),
        ))
        .then_ignore(newline.clone().or_not())
        .boxed()
    });

    let block = statement
        .clone()
        .repeated()
        .at_least(1)
        .delimited_by(just(TokenKind::Indent), just(TokenKind::Dedent))
        .map(Block::new);

    let function_param = identifier_parser()
        .then(just(TokenKind::Colon).ignore_then(type_parser()).or_not())
        .then(just(TokenKind::Equals).ignore_then(expr.clone()).or_not())
        .map(|((name, ty), default)| Param::new(name, ty, default));

    let function_params = function_param
        .separated_by(just(TokenKind::Comma))
        .allow_trailing()
        .delimited_by(just(TokenKind::LParen), just(TokenKind::RParen))
        .or_not()
        .map(|params| params.unwrap_or_default());

    let function_ret_type = just(TokenKind::Arrow).ignore_then(type_parser()).or_not();

    // Support both 'def' (Pythonic) and 'fn' (legacy) for function definitions
    let function_keyword = just(TokenKind::Def).or(just(TokenKind::Fn));

    let function = pub_keyword
        .clone()
        .then(function_keyword.clone())
        .then(identifier_parser())
        .then(function_params)
        .then(function_ret_type)
        .then_ignore(just(TokenKind::Colon))
        .then_ignore(newline.clone())
        .then(block.clone())
        .map(|(((((pub_kw, _fn), name), params), ret_ty), body)| {
            if pub_kw.is_some() {
                Function::new_public(name, params, ret_ty, body)
            } else {
                Function::new(name, params, ret_ty, body)
            }
        })
        .map(Statement::Function)
        .then_ignore(newline.clone().or_not());

    // Pythonic struct definition: struct Name<T>:
    //     field: Type
    //     def method(self, ...) -> ReturnType:
    //         ...
    let struct_generics = identifier_parser()
        .separated_by(just(TokenKind::Comma))
        .allow_trailing()
        .delimited_by(just(TokenKind::Lt), just(TokenKind::Gt))
        .or_not()
        .map(|params| params.unwrap_or_default());

    let struct_field = identifier_parser()
        .then_ignore(just(TokenKind::Colon))
        .then(type_parser())
        .map(|(name, ty)| (name, ty));

    // Parse struct body: fields and methods (indented)
    // Pythonic: struct Point:
    //     x: float
    //     y: float
    //     def distance(self) -> float:
    //         return math.sqrt(self.x * self.x + self.y * self.y)

    // Field definition
    let struct_field_def = struct_field
        .then_ignore(newline.clone().or_not())
        .map(|field| (Some(field), None::<Function>));

    // Method definition (def method(self, ...) -> ReturnType: ...)
    // Recreate parsers for method definition
    let method_function_param = identifier_parser()
        .then(just(TokenKind::Colon).ignore_then(type_parser()).or_not())
        .then(just(TokenKind::Equals).ignore_then(expr.clone()).or_not())
        .map(|((name, ty), default)| Param::new(name, ty, default));

    let method_function_params = method_function_param
        .separated_by(just(TokenKind::Comma))
        .allow_trailing()
        .delimited_by(just(TokenKind::LParen), just(TokenKind::RParen))
        .or_not()
        .map(|params| params.unwrap_or_default());

    let method_function_ret_type = just(TokenKind::Arrow).ignore_then(type_parser()).or_not();

    let struct_method_def = function_keyword
        .clone()
        .then(identifier_parser())
        .then(method_function_params)
        .then(method_function_ret_type)
        .then_ignore(just(TokenKind::Colon))
        .then_ignore(newline.clone())
        .then(block.clone())
        .map(|((((_kw, name), params), ret_ty), body)| {
            // Methods automatically get 'self' as first parameter if not present
            let mut method_params = params;
            if method_params.is_empty() || method_params[0].name != "self" {
                // Add self parameter at the beginning
                let self_type = Type::Simple("Self".to_string());
                let self_param = Param::new("self".to_string(), Some(self_type), None);
                method_params.insert(0, self_param);
            }
            Function::new(name, method_params, ret_ty, body)
        })
        .map(|method| (None::<(String, Type)>, Some(method)))
        .then_ignore(newline.clone().or_not());

    let struct_body = choice((struct_field_def, struct_method_def))
        .repeated()
        .at_least(0)
        .then_ignore(newline.clone().or_not())
        .map(|items| {
            let mut fields = Vec::new();
            let mut methods = Vec::new();
            for (field, method) in items {
                if let Some(f) = field {
                    fields.push(f);
                }
                if let Some(m) = method {
                    methods.push(m);
                }
            }
            (fields, methods)
        });

    let struct_def = pub_keyword
        .clone()
        .then(just(TokenKind::Struct))
        .then(identifier_parser())
        .then(struct_generics)
        .then_ignore(just(TokenKind::Colon))
        .then_ignore(newline.clone())
        .then(struct_body.delimited_by(just(TokenKind::Indent), just(TokenKind::Dedent)))
        .then_ignore(newline.clone().or_not())
        .map(
            |((((pub_kw, _), name), generics), (fields, methods))| Statement::Struct {
                name,
                fields,
                methods,
                public: pub_kw.is_some(),
                generics,
            },
        );

    // Type alias: type Name<T> = Type
    let type_alias_generics = identifier_parser()
        .separated_by(just(TokenKind::Comma))
        .allow_trailing()
        .delimited_by(just(TokenKind::Lt), just(TokenKind::Gt))
        .or_not()
        .map(|params| params.unwrap_or_default());

    let type_alias_def = pub_keyword
        .clone()
        .then(just(TokenKind::Identifier("type".to_string()))) // Using identifier since "type" isn't a keyword yet
        .then(identifier_parser())
        .then(type_alias_generics)
        .then_ignore(just(TokenKind::Equals))
        .then(type_parser())
        .then_ignore(newline.clone().or_not())
        .map(
            |((((pub_kw, _), name), generics), target)| Statement::TypeAlias {
                name,
                target,
                public: pub_kw.is_some(),
                generics,
            },
        );

    newline
        .clone()
        .or_not()
        .ignore_then(choice((struct_def, type_alias_def, function, statement)).repeated())
        .then_ignore(newline.repeated().or_not())
        .then_ignore(just(TokenKind::Eof))
        .map(Program::new)
}
