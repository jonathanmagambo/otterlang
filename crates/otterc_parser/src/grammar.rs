use chumsky::Stream;
use chumsky::prelude::*;

use otterc_ast::nodes::{
    BinaryOp, Block, EnumVariant, Expr, FStringPart, Function, Literal, MatchArm, Node,
    NumberLiteral, Param, Pattern, Program, Statement, Type, UnaryOp, UseImport,
};

use otterc_lexer::token::{Token, TokenKind};
use otterc_span::Span;
use otterc_utils::errors::{Diagnostic, DiagnosticSeverity};
use std::ops::Range;

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
        .map(|token| token.span())
        .unwrap_or_else(|| Span::new(0, 0));

    let end = eof_span.end();
    let stream = Stream::from_iter(
        end..end + 1,
        tokens
            .iter()
            .map(|token| (token.kind().clone(), token.span().into())),
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
        TokenKind::Fn => "fn".to_string(),
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
        TokenKind::As => "as".to_string(),
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

fn type_parser() -> impl Parser<TokenKind, Node<Type>, Error = Simple<TokenKind>> {
    recursive(|ty| {
        identifier_parser()
            .then(
                ty.separated_by(just(TokenKind::Comma))
                    .allow_trailing()
                    .delimited_by(just(TokenKind::Lt), just(TokenKind::Gt))
                    .or_not(),
            )
            .map_with_span(|(base, args), span| {
                Node::new(
                    match args {
                        Some(args) => Type::Generic { base, args },
                        None => Type::Simple(base),
                    },
                    span,
                )
            })
    })
}

fn parse_fstring(content: String, span: impl Into<Span>) -> Node<Expr> {
    use chumsky::Parser;

    // Parse f-string by splitting on braces and parsing expressions
    let mut parts = Vec::new();
    let mut current_text = String::new();
    let mut chars = content.chars().enumerate().peekable();
    let span: Span = span.into();
    let span_start = span.start();

    while let Some((i, ch)) = chars.next() {
        let span_start = span_start + i;
        match ch {
            '{' => {
                if let Some((_, '{')) = chars.peek() {
                    // Escaped {{
                    chars.next();
                    current_text.push('{');
                } else {
                    // Expression start
                    if !current_text.is_empty() {
                        let s = Span::new(span_start, span_start + current_text.len());
                        parts.push(Node::new(FStringPart::Text(current_text), s));
                        current_text = String::new();
                    }

                    // Parse expression until }
                    let mut expr_content = String::new();

                    for (_, ch) in chars.by_ref() {
                        if ch == '}' {
                            break;
                        }

                        expr_content.push(ch);
                    }

                    if !expr_content.is_empty() {
                        // Parse the expression content using the full expression parser
                        let trimmed = expr_content.trim();
                        if !trimmed.is_empty() {
                            match otterc_lexer::tokenize(trimmed) {
                                Ok(tokens) => {
                                    // Create a stream from tokens for the parser
                                    use chumsky::Stream;
                                    let end_span =
                                        tokens.last().map(|t| t.span().end()).unwrap_or(0);
                                    let stream = Stream::from_iter(
                                        end_span..end_span + 1,
                                        tokens.iter().map(|token| {
                                            (token.kind().clone(), token.span().into())
                                        }),
                                    );
                                    match expr_parser().parse(stream) {
                                        Ok(expr) => {
                                            let s = Span::new(
                                                span_start,
                                                span_start + expr.span().end(),
                                            );
                                            parts.push(Node::new(FStringPart::Expr(expr), s));
                                        }
                                        Err(_) => {
                                            // Fallback to simple identifier if parsing fails
                                            let s =
                                                Span::new(span_start, span_start + trimmed.len());
                                            parts.push(Node::new(
                                                FStringPart::Expr(Node::new(
                                                    Expr::Identifier(trimmed.to_string()),
                                                    s,
                                                )),
                                                s,
                                            ));
                                        }
                                    }
                                }
                                Err(_) => {
                                    // Fallback to simple identifier if parsing fails
                                    let s = Span::new(span_start, span_start + trimmed.len());
                                    parts.push(Node::new(
                                        FStringPart::Expr(Node::new(
                                            Expr::Identifier(trimmed.to_string()),
                                            s,
                                        )),
                                        s,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            '}' => {
                if let Some((_, '}')) = chars.peek() {
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
        parts.push(Node::new(FStringPart::Text(current_text), span));
    }

    // If no expressions found, treat as regular string
    if parts
        .iter()
        .all(|part| matches!(part.as_ref(), FStringPart::Text(_)))
        && let Some(FStringPart::Text(text)) = parts.first().map(|p| p.as_ref())
    {
        return Node::new(
            Expr::Literal(Node::new(Literal::String(text.clone()), span)),
            span,
        );
    }

    Node::new(Expr::FString { parts }, span)
}

fn literal_expr_parser() -> impl Parser<TokenKind, Node<Expr>, Error = Simple<TokenKind>> {
    let string_lit = select! { TokenKind::StringLiteral(value) => Literal::String(value) }
        .map_with_span(|lit, span: Range<usize>| {
            let span: Span = span.into();
            Node::new(Expr::Literal(Node::new(lit, span)), span)
        })
        .boxed();
    let number_lit = select! { TokenKind::Number(value) => {
        // Remove underscores from the number
        let clean_value = value.replace('_', "");
        let is_float_literal = value.contains('.') || value.contains('e') || value.contains('E');
        // Check if it contains a decimal point or is an integer
        if clean_value.contains('.') {
            NumberLiteral::new(
                clean_value.parse().unwrap_or_default(),
                true,
            )
        } else {
            // Parse as integer
            match clean_value.parse::<i64>() {
                Ok(int_val) => NumberLiteral::new(int_val as f64, is_float_literal),
                Err(_) => NumberLiteral::new(0.0, is_float_literal),
            }
        }
    }}
    .map_with_span(|num_lit, span: Range<usize>| {
        let span: Span = span.into();
        Node::new(
            Expr::Literal(Node::new(Literal::Number(num_lit), span)),
            span,
        )
    })
    .boxed();
    let bool_lit = select! {
        TokenKind::True => Literal::Bool(true),
        TokenKind::False => Literal::Bool(false),
    }
    .map_with_span(|lit, span: Range<usize>| {
        let span: Span = span.into();
        Node::new(Expr::Literal(Node::new(lit, span)), span)
    })
    .boxed();
    let none_lit = just(TokenKind::None)
        .to(Literal::None)
        .map_with_span(|lit, span: Range<usize>| {
            let span: Span = span.into();
            Node::new(Expr::Literal(Node::new(lit, span)), span)
        })
        .boxed();
    let fstring_lit =
        select! { |span| TokenKind::FString(content) => parse_fstring(content, span) }.boxed();
    let unit_lit = just(TokenKind::LParen)
        .then(just(TokenKind::RParen))
        .map_with_span(|_, span: Range<usize>| {
            let span: Span = span.into();
            Node::new(Expr::Literal(Node::new(Literal::Unit, span)), span)
        })
        .boxed();
    choice((
        fstring_lit,
        string_lit,
        number_lit,
        bool_lit,
        none_lit,
        unit_lit,
    ))
}

#[derive(Debug, Clone)]
enum PostfixOp {
    Member { field: String, span: Span },
    Call { args: Vec<Node<Expr>>, span: Span },
}

fn expr_parser() -> impl Parser<TokenKind, Node<Expr>, Error = Simple<TokenKind>> {
    recursive(|expr| {
        // Lambda expressions removed - use anonymous fn syntax instead
        // fn(<args>) expr or fn(<args>): <stmts>

        let struct_init_pythonic = identifier_parser()
            .then(
                // Keyword argument: name=value
                identifier_parser()
                    .then_ignore(just(TokenKind::Equals))
                    .then(expr.clone())
                    .separated_by(just(TokenKind::Comma))
                    .at_least(1)
                    .allow_trailing()
                    .delimited_by(just(TokenKind::LParen), just(TokenKind::RParen)),
            )
            .map_with_span(|(name, fields), span| {
                Node::new(
                    Expr::Struct {
                        name,
                        fields: fields.into_iter().collect(),
                    },
                    span,
                )
            })
            .boxed();

        let list_comprehension = expr
            .clone()
            .then_ignore(just(TokenKind::For))
            .then(identifier_parser())
            .then_ignore(just(TokenKind::In))
            .then(expr.clone())
            .then(just(TokenKind::If).ignore_then(expr.clone()).or_not())
            .map_with_span(|(((element, var), iterable), condition), span| {
                Node::new(
                    Expr::ListComprehension {
                        element: Box::new(element),
                        var,
                        iterable: Box::new(iterable),
                        condition: condition.map(Box::new),
                    },
                    span,
                )
            })
            .delimited_by(just(TokenKind::LBracket), just(TokenKind::RBracket))
            .boxed();

        let dict_comprehension = expr
            .clone()
            .then_ignore(just(TokenKind::Colon))
            .then(expr.clone())
            .then_ignore(just(TokenKind::For))
            .then(identifier_parser())
            .then_ignore(just(TokenKind::In))
            .then(expr.clone())
            .then(just(TokenKind::If).ignore_then(expr.clone()).or_not())
            .map_with_span(|((((key, value), var), iterable), condition), span| {
                Node::new(
                    Expr::DictComprehension {
                        key: Box::new(key),
                        value: Box::new(value),
                        var,
                        iterable: Box::new(iterable),
                        condition: condition.map(Box::new),
                    },
                    span,
                )
            })
            .delimited_by(just(TokenKind::LBrace), just(TokenKind::RBrace))
            .boxed();

        let atom = choice((
            literal_expr_parser(),
            struct_init_pythonic,
            identifier_parser().map_with_span(|name, span| Node::new(Expr::Identifier(name), span)),
            expr.clone()
                .delimited_by(just(TokenKind::LParen), just(TokenKind::RParen)),
            list_comprehension,
            // Array literal [expr, expr, ...]
            expr.clone()
                .separated_by(just(TokenKind::Comma))
                .allow_trailing()
                .delimited_by(just(TokenKind::LBracket), just(TokenKind::RBracket))
                .map_with_span(|array, span| Node::new(Expr::Array(array), span)),
            dict_comprehension,
            // Dictionary literal {key: value, ...}
            expr.clone()
                .then_ignore(just(TokenKind::Colon))
                .then(expr.clone())
                .separated_by(just(TokenKind::Comma))
                .allow_trailing()
                .delimited_by(just(TokenKind::LBrace), just(TokenKind::RBrace))
                .map_with_span(|dict, span| Node::new(Expr::Dict(dict), span)),
        ))
        .boxed();

        let member_suffix = just(TokenKind::Dot)
            .ignore_then(identifier_or_keyword_parser())
            .map_with_span(|field, span: Range<usize>| PostfixOp::Member {
                field,
                span: span.into(),
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
            .then_ignore(just(TokenKind::RParen))
            .map_with_span(|args, span: Range<usize>| PostfixOp::Call {
                args,
                span: span.into(),
            })
            .boxed();

        let call = atom
            .clone()
            .then(choice((member_suffix.clone(), call_suffix.clone())).repeated())
            .foldl(|expr, suffix| match suffix {
                PostfixOp::Member { field, span } => {
                    let span = expr.span().merge(&span);
                    Node::new(
                        Expr::Member {
                            object: Box::new(expr),
                            field,
                        },
                        span,
                    )
                }
                PostfixOp::Call { args, span } => {
                    let span = expr.span().merge(&span);
                    Node::new(
                        Expr::Call {
                            func: Box::new(expr),
                            args,
                        },
                        span,
                    )
                }
            })
            .boxed();

        let await_expr = just(TokenKind::Await)
            .ignore_then(call.clone())
            .map_with_span(|expr, span| Node::new(Expr::Await(Box::new(expr)), span))
            .boxed();

        let spawn_expr = just(TokenKind::Spawn)
            .ignore_then(call.clone())
            .map_with_span(|expr, span| Node::new(Expr::Spawn(Box::new(expr)), span))
            .boxed();

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
        .map_with_span(|(op, expr), span| {
            Node::new(
                Expr::Unary {
                    op,
                    expr: Box::new(expr),
                },
                span,
            )
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
            .foldl(|left, (op, right)| {
                let span = left.span().merge(right.span());
                Node::new(
                    Expr::Binary {
                        left: Box::new(left),
                        op,
                        right: Box::new(right),
                    },
                    span,
                )
            })
            .boxed();

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
            .foldl(|left, (op, right)| {
                let span = left.span().merge(right.span());
                Node::new(
                    Expr::Binary {
                        left: Box::new(left),
                        op,
                        right: Box::new(right),
                    },
                    span,
                )
            })
            .boxed();

        let range = sum
            .clone()
            .then(just(TokenKind::DoubleDot).ignore_then(sum.clone()).or_not())
            .map_with_span(|(start, end), span| {
                if let Some(end) = end {
                    Node::new(
                        Expr::Range {
                            start: Box::new(start),
                            end: Box::new(end),
                        },
                        span,
                    )
                } else {
                    start
                }
            })
            .boxed();

        let is_operator = just(TokenKind::Is)
            .ignore_then(just(TokenKind::Not).or_not())
            .map(|not_opt| {
                if not_opt.is_some() {
                    BinaryOp::IsNot
                } else {
                    BinaryOp::Is
                }
            })
            .boxed();

        let comparison_op = choice((
            just(TokenKind::EqEq).to(BinaryOp::Eq),
            just(TokenKind::Neq).to(BinaryOp::Ne),
            just(TokenKind::Lt).to(BinaryOp::Lt),
            just(TokenKind::Gt).to(BinaryOp::Gt),
            just(TokenKind::LtEq).to(BinaryOp::LtEq),
            just(TokenKind::GtEq).to(BinaryOp::GtEq),
            is_operator,
        ))
        .boxed();

        let comparison = range
            .clone()
            .then(comparison_op.then(range.clone()).repeated())
            .foldl(|left, (op, right)| {
                let span = left.span().merge(right.span());
                Node::new(
                    Expr::Binary {
                        left: Box::new(left),
                        op,
                        right: Box::new(right),
                    },
                    span,
                )
            })
            .boxed();

        let logical = comparison
            .clone()
            .then(
                choice((
                    just(TokenKind::And).to(BinaryOp::And),
                    just(TokenKind::Or).to(BinaryOp::Or),
                ))
                .then(comparison)
                .repeated(),
            )
            .foldl(|left, (op, right)| {
                let span = left.span().merge(right.span());
                Node::new(
                    Expr::Binary {
                        left: Box::new(left),
                        op,
                        right: Box::new(right),
                    },
                    span,
                )
            })
            .boxed();

        let newline = just(TokenKind::Newline).repeated().at_least(1);

        // Define a local statement parser for match arms to avoid circular dependency
        // This duplicates some logic from program_parser but is necessary because expr_parser
        // cannot easily access the recursive statement parser from program_parser.
        let match_stmt = recursive(|_stmt| {
            let print_stmt = just(TokenKind::Print)
                .ignore_then(
                    expr.clone()
                        .delimited_by(just(TokenKind::LParen), just(TokenKind::RParen)),
                )
                .map_with_span(|arg, span| {
                    let span: Span = span.into();
                    Node::new(
                        Statement::Expr(Node::new(
                            Expr::Call {
                                func: Box::new(Node::new(
                                    Expr::Identifier("print".to_string()),
                                    span,
                                )),
                                args: vec![arg],
                            },
                            span,
                        )),
                        span,
                    )
                })
                .boxed();

            let return_stmt = just(TokenKind::Return)
                .ignore_then(expr.clone().or_not())
                .map_with_span(|expr, span| Node::new(Statement::Return(expr), span))
                .boxed();

            let let_stmt = just(TokenKind::Let)
                .or_not()
                .then(
                    identifier_parser()
                        .map_with_span(Node::new)
                        .then(just(TokenKind::Colon).ignore_then(type_parser()).or_not()),
                )
                .then_ignore(just(TokenKind::Equals))
                .then(expr.clone())
                .map_with_span(|((_let, (name, ty)), expr), span| {
                    Node::new(
                        Statement::Let {
                            name,
                            ty,
                            expr,
                            public: false, // Match arms are local scopes
                        },
                        span,
                    )
                });

            let assignment_stmt = identifier_parser()
                .map_with_span(|name, span| (name, Span::new(span.start, span.end)))
                .then(choice((
                    just(TokenKind::PlusEq).to(BinaryOp::Add),
                    just(TokenKind::MinusEq).to(BinaryOp::Sub),
                    just(TokenKind::StarEq).to(BinaryOp::Mul),
                    just(TokenKind::SlashEq).to(BinaryOp::Div),
                )))
                .then(expr.clone())
                .map_with_span(|(((name, name_span), op), rhs), span| {
                    let span: Span = span.into();
                    let expr = Node::new(
                        Expr::Binary {
                            op,
                            left: Box::new(Node::new(Expr::Identifier(name.clone()), name_span)),
                            right: Box::new(rhs),
                        },
                        span,
                    );
                    let target = Node::new(Expr::Identifier(name), name_span);
                    Node::new(Statement::Assignment { target, expr }, span)
                })
                .boxed();

            // Simple assignment (=)
            let simple_assignment = expr
                .clone()
                .then_ignore(just(TokenKind::Equals))
                .then(expr.clone())
                .map_with_span(|(target, expr), span| {
                    Node::new(Statement::Assignment { target, expr }, span)
                })
                .boxed();

            let pass_stmt = just(TokenKind::Pass)
                .map_with_span(|_, span| Node::new(Statement::Pass, span))
                .boxed();

            let break_stmt = just(TokenKind::Break)
                .map_with_span(|_, span| Node::new(Statement::Break, span))
                .boxed();

            let continue_stmt = just(TokenKind::Continue)
                .map_with_span(|_, span| Node::new(Statement::Continue, span))
                .boxed();

            choice((
                print_stmt,
                return_stmt,
                let_stmt,
                assignment_stmt,
                simple_assignment,
                pass_stmt,
                break_stmt,
                continue_stmt,
                expr.clone()
                    .map_with_span(|expr, span| Node::new(Statement::Expr(expr), span)),
            ))
            .then_ignore(newline.clone().or_not())
            .boxed()
        });

        let match_case = just(TokenKind::Case)
            .ignore_then(pattern_parser())
            .then_ignore(just(TokenKind::Colon))
            .then_ignore(newline.clone())
            .then(
                match_stmt
                    .clone()
                    .repeated()
                    .at_least(1)
                    .delimited_by(just(TokenKind::Indent), just(TokenKind::Dedent))
                    .map_with_span(|block, span| Node::new(Block::new(block), span)),
            )
            .map_with_span(|(pattern, body), span| {
                Node::new(
                    MatchArm {
                        pattern,
                        guard: None,
                        body,
                    },
                    span,
                )
            })
            .then_ignore(newline.clone().or_not())
            .boxed();

        just(TokenKind::Match)
            .ignore_then(logical.clone())
            .then(
                just(TokenKind::Colon)
                    .ignore_then(newline.clone())
                    .ignore_then(just(TokenKind::Indent))
                    .ignore_then(match_case.repeated().at_least(1))
                    .then_ignore(just(TokenKind::Dedent)),
            )
            .map_with_span(|(value, arms), span| {
                Node::new(
                    Expr::Match {
                        value: Box::new(value),
                        arms,
                    },
                    span,
                )
            })
            .or(logical)
    })
}

/// Pattern parser for match expressions
fn pattern_parser() -> impl Parser<TokenKind, Node<Pattern>, Error = Simple<TokenKind>> {
    recursive(|pattern| {
        let wildcard = just(TokenKind::Identifier("_".to_string()))
            .map_with_span(|_, span| Node::new(Pattern::Wildcard, span))
            .boxed();

        let literal_pattern = literal_expr_parser()
            .map_with_span(|expr, span| {
                Node::new(
                    match expr.into_inner() {
                        Expr::Literal(lit) => Pattern::Literal(lit),
                        _ => Pattern::Wildcard, // Fallback
                    },
                    span,
                )
            })
            .boxed();

        let identifier_pattern = identifier_parser()
            .map_with_span(|ident, span| Node::new(Pattern::Identifier(ident), span))
            .boxed();

        let variant_name = choice((
            identifier_parser(),
            just(TokenKind::None).to("None".to_string()),
        ))
        .boxed();

        let enum_variant_pattern = identifier_parser()
            .then_ignore(just(TokenKind::Dot))
            .then(variant_name)
            .then(
                just(TokenKind::LParen)
                    .ignore_then(
                        pattern
                            .clone()
                            .separated_by(just(TokenKind::Comma))
                            .allow_trailing(),
                    )
                    .then_ignore(just(TokenKind::RParen))
                    .or_not(),
            )
            .map_with_span(|((enum_name, variant), fields), span| {
                Node::new(
                    Pattern::EnumVariant {
                        enum_name,
                        variant,
                        fields: fields.unwrap_or_default(),
                    },
                    span,
                )
            })
            .boxed();

        let struct_pattern = identifier_parser()
            .then(
                just(TokenKind::LBrace)
                    .ignore_then(
                        identifier_parser()
                            .then(just(TokenKind::Colon).ignore_then(pattern.clone()).or_not())
                            .separated_by(just(TokenKind::Comma))
                            .allow_trailing(),
                    )
                    .then_ignore(just(TokenKind::RBrace)),
            )
            .map_with_span(|(name, fields), span| {
                Node::new(
                    Pattern::Struct {
                        name,
                        fields: fields.into_iter().collect(),
                    },
                    span,
                )
            })
            .boxed();

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
            .map_with_span(|(patterns, rest), span| {
                Node::new(Pattern::Array { patterns, rest }, span)
            })
            .boxed();

        choice((
            wildcard,
            literal_pattern,
            enum_variant_pattern,
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
        .map_with_span(|arg, span| {
            let span: Span = span.into();
            Node::new(
                Statement::Expr(Node::new(
                    Expr::Call {
                        func: Box::new(Node::new(Expr::Identifier("print".to_string()), span)),
                        args: vec![arg],
                    },
                    span,
                )),
                span,
            )
        })
        .boxed();

    let return_stmt = just(TokenKind::Return)
        .ignore_then(expr.clone().or_not())
        .map_with_span(|expr, span| Node::new(Statement::Return(expr), span))
        .boxed();

    let pub_keyword = just(TokenKind::Pub).or_not();

    let let_stmt = pub_keyword
        .clone()
        .then(just(TokenKind::Let))
        .then(
            identifier_parser()
                .map_with_span(Node::new)
                .then(just(TokenKind::Colon).ignore_then(type_parser()).or_not()),
        )
        .then_ignore(just(TokenKind::Equals))
        .then(expr.clone())
        .map_with_span(|(((pub_kw, _let), (name, ty)), expr), span| {
            Node::new(
                Statement::Let {
                    name,
                    ty,
                    expr,
                    public: pub_kw.is_some(),
                },
                span,
            )
        });

    let simple_assignment_stmt = expr
        .clone()
        .then_ignore(just(TokenKind::Equals))
        .then(expr.clone())
        .map_with_span(|(target, expr), span| {
            Node::new(Statement::Assignment { target, expr }, span)
        });

    let compound_assignment_stmt = identifier_parser()
        .map_with_span(|name, span| (name, Span::new(span.start, span.end)))
        .then(choice((
            just(TokenKind::PlusEq).to(BinaryOp::Add),
            just(TokenKind::MinusEq).to(BinaryOp::Sub),
            just(TokenKind::StarEq).to(BinaryOp::Mul),
            just(TokenKind::SlashEq).to(BinaryOp::Div),
        )))
        .then(expr.clone())
        .map_with_span(|(((name, name_span), op), rhs), span| {
            let span: Span = span.into();
            // Desugar: x += y becomes x = x + y
            let expr = Node::new(
                Expr::Binary {
                    op,
                    left: Box::new(Node::new(Expr::Identifier(name.clone()), name_span)),
                    right: Box::new(rhs),
                },
                span,
            );
            let target = Node::new(Expr::Identifier(name), name_span);
            Node::new(Statement::Assignment { target, expr }, span)
        })
        .boxed();

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

    fn normalize_module_name(module: String) -> String {
        if let Some(stripped) = module.strip_prefix("otterc_") {
            stripped.to_string()
        } else {
            module
        }
    }

    let module_path = path_segment
        .clone()
        .then(path_separator.then(path_segment.clone()).repeated())
        .map(|(first, rest)| {
            let mut module = first;
            for (sep, segment) in rest {
                module.push_str(&sep);
                module.push_str(&segment);
            }
            normalize_module_name(module)
        });

    let use_import = module_path
        .clone()
        .then(
            just(TokenKind::As)
                .ignore_then(identifier_parser())
                .or_not(),
        )
        .map_with_span(|(module, alias), span| Node::new(UseImport::new(module, alias), span))
        .boxed();

    let use_stmt = just(TokenKind::Use)
        .ignore_then(
            use_import
                .separated_by(just(TokenKind::Comma))
                .allow_trailing()
                .at_least(1),
        )
        .map_with_span(|imports, span| Node::new(Statement::Use { imports }, span))
        .boxed();

    // pub use statement for re-exports
    // Syntax: pub use otterc_module.item [as alias]
    //         pub use otterc_module (re-export all)
    let pub_use_stmt = just(TokenKind::Pub)
        .ignore_then(just(TokenKind::Use))
        .ignore_then(
            module_path
                .clone()
                .then(
                    just(TokenKind::Dot)
                        .ignore_then(identifier_parser())
                        .or_not(),
                )
                .then(
                    just(TokenKind::As)
                        .ignore_then(identifier_parser())
                        .or_not(),
                )
                .map_with_span(|((module, item), alias), span| {
                    Node::new(
                        Statement::PubUse {
                            module,
                            item,
                            alias,
                        },
                        span,
                    )
                }),
        )
        .boxed();

    let break_stmt = just(TokenKind::Break)
        .map_with_span(|_, span| Node::new(Statement::Break, span))
        .boxed();
    let continue_stmt = just(TokenKind::Continue)
        .map_with_span(|_, span| Node::new(Statement::Continue, span))
        .boxed();
    let pass_stmt = just(TokenKind::Pass)
        .map_with_span(|_, span| Node::new(Statement::Pass, span))
        .boxed();

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
                    .map_with_span(|block, span| Node::new(Block::new(block), span)),
            )
            .map(|(cond, block)| (cond, block))
            .boxed();

        let if_stmt = just(TokenKind::If)
            .ignore_then(expr.clone())
            .then_ignore(just(TokenKind::Colon))
            .then_ignore(newline.clone())
            .then(
                stmt.clone()
                    .repeated()
                    .at_least(1)
                    .delimited_by(just(TokenKind::Indent), just(TokenKind::Dedent))
                    .map_with_span(|block, span| Node::new(Block::new(block), span)),
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
                            .map_with_span(|block, span| Node::new(Block::new(block), span)),
                    )
                    .or_not(),
            )
            .map_with_span(|(((cond, then_block), elif_blocks), else_block), span| {
                Node::new(
                    Statement::If {
                        cond,
                        then_block,
                        elif_blocks,
                        else_block: else_block.map(|(_, block)| block),
                    },
                    span,
                )
            })
            .boxed();

        let for_stmt = just(TokenKind::For)
            .ignore_then(identifier_parser().map_with_span(Node::new))
            .then_ignore(just(TokenKind::In))
            .then(expr.clone())
            .then_ignore(just(TokenKind::Colon))
            .then_ignore(newline.clone())
            .then(
                stmt.clone()
                    .repeated()
                    .at_least(1)
                    .delimited_by(just(TokenKind::Indent), just(TokenKind::Dedent))
                    .map_with_span(|block, span| Node::new(Block::new(block), span)),
            )
            .map_with_span(|((var, iterable), body), span| {
                Node::new(
                    Statement::For {
                        var,
                        iterable,
                        body,
                    },
                    span,
                )
            })
            .boxed();

        let while_stmt = just(TokenKind::While)
            .ignore_then(expr.clone())
            .then_ignore(just(TokenKind::Colon))
            .then_ignore(newline.clone())
            .then(
                stmt.clone()
                    .repeated()
                    .at_least(1)
                    .delimited_by(just(TokenKind::Indent), just(TokenKind::Dedent))
                    .map_with_span(|block, span| Node::new(Block::new(block), span)),
            )
            .map_with_span(|(cond, body), span| Node::new(Statement::While { cond, body }, span))
            .boxed();

        // Exception handling (try/except/finally/raise) removed - use Result<T, E> pattern matching instead

        choice((
            print_stmt,
            return_stmt,
            let_stmt,
            compound_assignment_stmt,
            simple_assignment_stmt,
            use_stmt,
            pub_use_stmt,
            if_stmt,
            for_stmt,
            while_stmt,
            break_stmt,
            continue_stmt,
            pass_stmt,
            expr.clone()
                .map_with_span(|expr, span| Node::new(Statement::Expr(expr), span)),
        ))
        .then_ignore(newline.clone().or_not())
        .boxed()
    });

    let block = statement
        .clone()
        .repeated()
        .at_least(1)
        .delimited_by(just(TokenKind::Indent), just(TokenKind::Dedent))
        .map_with_span(|block, span| Node::new(Block::new(block), span))
        .boxed();

    let function_param = identifier_parser()
        .map_with_span(Node::new)
        .then(choice((
            just(TokenKind::Colon).ignore_then(type_parser()).map(Some),
            empty().to(None),
        )))
        .then(choice((
            just(TokenKind::Equals).ignore_then(expr.clone()).map(Some),
            empty().to(None),
        )))
        .map_with_span(|((name, ty), default), span| Node::new(Param::new(name, ty, default), span))
        .boxed();

    let function_params = function_param
        .separated_by(just(TokenKind::Comma))
        .allow_trailing()
        .delimited_by(just(TokenKind::LParen), just(TokenKind::RParen))
        .or_not()
        .map(|params| params.unwrap_or_default());

    let function_ret_type = just(TokenKind::Arrow).ignore_then(type_parser()).or_not();

    let function_keyword = just(TokenKind::Fn);

    let function = pub_keyword
        .clone()
        .then(function_keyword.clone())
        .then(identifier_parser())
        .then(function_params)
        .then(function_ret_type)
        .then_ignore(just(TokenKind::Colon))
        .then_ignore(newline.clone())
        .then(block.clone())
        .map_with_span(|(((((pub_kw, _fn), name), params), ret_ty), body), span| {
            Node::new(
                if pub_kw.is_some() {
                    Function::new_public(name, params, ret_ty, body)
                } else {
                    Function::new(name, params, ret_ty, body)
                },
                span,
            )
        })
        .map_with_span(|func, span| Node::new(Statement::Function(func), span))
        .then_ignore(newline.clone().or_not())
        .boxed();

    //     field: Type
    //     fn method(self, ...) -> ReturnType:
    //         ...
    let struct_generics = || {
        identifier_parser()
            .separated_by(just(TokenKind::Comma))
            .allow_trailing()
            .delimited_by(just(TokenKind::Lt), just(TokenKind::Gt))
            .or_not()
            .map(|params| params.unwrap_or_default())
    };

    let enum_variant_name = choice((
        identifier_parser(),
        just(TokenKind::None).to("None".to_string()),
    ));

    let enum_variant = enum_variant_name
        .then(
            just(TokenKind::Colon)
                .ignore_then(
                    type_parser()
                        .separated_by(just(TokenKind::Comma))
                        .allow_trailing()
                        .delimited_by(just(TokenKind::LParen), just(TokenKind::RParen)),
                )
                .or_not(),
        )
        .then_ignore(newline.clone().or_not())
        .map_with_span(|(name, fields), span| {
            Node::new(EnumVariant::new(name, fields.unwrap_or_default()), span)
        })
        .boxed();

    let enum_body = enum_variant
        .repeated()
        .at_least(1)
        .then_ignore(newline.clone().or_not())
        .boxed();

    let struct_field = identifier_parser()
        .then_ignore(just(TokenKind::Colon))
        .then(type_parser())
        .map(|(name, ty)| (name, ty));

    // Parse struct body: fields and methods (indented)
    //     x: float
    //     y: float
    //     fn distance(self) -> float:
    //         return math.sqrt(self.x * self.x + self.y * self.y)

    // Field definition
    let struct_field_def = struct_field
        .then_ignore(newline.clone().or_not())
        .map(|field| (Some(field), None::<Node<Function>>))
        .boxed();

    // Method definition (fn method(self, ...) -> ReturnType: ...)
    // Recreate parsers for method definition
    let method_function_param = identifier_parser()
        .map_with_span(Node::new)
        .then(choice((
            just(TokenKind::Colon).ignore_then(type_parser()).map(Some),
            empty().to(None),
        )))
        .then(choice((
            just(TokenKind::Equals).ignore_then(expr.clone()).map(Some),
            empty().to(None),
        )))
        .map_with_span(|((name, ty), default), span| Node::new(Param::new(name, ty, default), span))
        .boxed();

    let method_function_params = method_function_param
        .separated_by(just(TokenKind::Comma))
        .allow_trailing()
        .delimited_by(just(TokenKind::LParen), just(TokenKind::RParen))
        .or_not()
        .map(|params| params.unwrap_or_default())
        .boxed();

    let method_function_ret_type = just(TokenKind::Arrow).ignore_then(type_parser()).or_not();

    let struct_method_def = function_keyword
        .clone()
        .then(identifier_parser())
        .then(method_function_params)
        .then(method_function_ret_type)
        .then_ignore(just(TokenKind::Colon))
        .then_ignore(newline.clone())
        .then(block.clone())
        .map_with_span(|((((_kw, name), params), ret_ty), body), span| {
            // Methods automatically get 'self' as first parameter if not present
            let mut method_params = params;
            if method_params.is_empty() || method_params[0].as_ref().name.as_ref() != "self" {
                // Add self parameter at the beginning
                let self_type = Type::Simple("Self".to_string());
                let self_span = Span::new(span.start + name.len() + 1, span.start + name.len() + 5);
                let self_type_span = Span::new(self_span.start(), self_span.start());
                let self_param = Node::new(
                    Param::new(
                        Node::new("self".to_string(), self_span),
                        Some(Node::new(self_type, self_type_span)),
                        None,
                    ),
                    self_span,
                );
                method_params.insert(0, self_param);
            }
            Node::new(Function::new(name, method_params, ret_ty, body), span)
        })
        .map(|method| (None::<(String, Node<Type>)>, Some(method)))
        .then_ignore(newline.clone().or_not())
        .boxed();

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
        .then(struct_generics())
        .then_ignore(just(TokenKind::Colon))
        .then_ignore(newline.clone())
        .then(struct_body.delimited_by(just(TokenKind::Indent), just(TokenKind::Dedent)))
        .then_ignore(newline.clone().or_not())
        .map_with_span(
            |((((pub_kw, _), name), generics), (fields, methods)), span| {
                Node::new(
                    Statement::Struct {
                        name,
                        fields,
                        methods,
                        public: pub_kw.is_some(),
                        generics,
                    },
                    span,
                )
            },
        )
        .boxed();

    let enum_def = pub_keyword
        .clone()
        .then(just(TokenKind::Enum))
        .then(identifier_parser())
        .then(struct_generics())
        .then_ignore(just(TokenKind::Colon))
        .then_ignore(newline.clone())
        .then(enum_body.delimited_by(just(TokenKind::Indent), just(TokenKind::Dedent)))
        .then_ignore(newline.clone().or_not())
        .map_with_span(|((((pub_kw, _), name), generics), variants), span| {
            Node::new(
                Statement::Enum {
                    name,
                    variants,
                    public: pub_kw.is_some(),
                    generics,
                },
                span,
            )
        })
        .boxed();

    // Type alias: type Name<T> = Type
    let type_alias_generics = identifier_parser()
        .separated_by(just(TokenKind::Comma))
        .allow_trailing()
        .delimited_by(just(TokenKind::Lt), just(TokenKind::Gt))
        .or_not()
        .map_with_span(|params, span| Node::new(params.unwrap_or_default(), span))
        .boxed();

    let type_alias_def = pub_keyword
        .clone()
        .then(just(TokenKind::Identifier("type".to_string()))) // Using identifier since "type" isn't a keyword yet
        .then(identifier_parser())
        .then(type_alias_generics)
        .then_ignore(just(TokenKind::Equals))
        .then(type_parser())
        .then_ignore(newline.clone().or_not())
        .map_with_span(|((((pub_kw, _), name), generics), target), span| {
            Node::new(
                Statement::TypeAlias {
                    name,
                    target,
                    public: pub_kw.is_some(),
                    generics: generics.into_inner(),
                },
                span,
            )
        })
        .boxed();

    newline
        .clone()
        .or_not()
        .ignore_then(choice((struct_def, enum_def, type_alias_def, function, statement)).repeated())
        .then_ignore(newline.repeated().or_not())
        .then_ignore(just(TokenKind::Eof))
        .map(Program::new)
}

#[cfg(test)]
mod tests {
    #![expect(clippy::panic, reason = "Panicking on test failures is acceptable")]

    use super::*;

    #[test]
    fn parses_multiple_use_modules() {
        let source = "use otterc_fmt, math as m\n";
        let tokens = otterc_lexer::tokenize(source).expect("tokenize use statement");
        let program = parse(&tokens).expect("parse use statement");

        assert_eq!(program.statements.len(), 1);
        match &program.statements[0].as_ref() {
            Statement::Use { imports } => {
                assert_eq!(imports.len(), 2);
                assert_eq!(imports[0].as_ref().module, "fmt");
                assert!(imports[0].as_ref().alias.is_none());
                assert_eq!(imports[1].as_ref().module, "math");
                assert_eq!(imports[1].as_ref().alias.as_deref(), Some("m"));
            }
            other => panic!("expected use statement, got {:?}", other),
        }
    }

    #[test]
    fn parses_otter_namespace_use() {
        let source = "use otter:core\n";
        let tokens = otterc_lexer::tokenize(source).expect("tokenize namespace use");
        let program = parse(&tokens).expect("parse namespace use");

        assert_eq!(program.statements.len(), 1);
        match &program.statements[0].as_ref() {
            Statement::Use { imports } => {
                assert_eq!(imports.len(), 1);
                assert_eq!(imports[0].as_ref().module, "otter:core");
            }
            other => panic!("expected use statement, got {:?}", other),
        }
    }

    #[test]
    fn parses_core_stdlib_module() {
        let source = include_str!("../../../stdlib/otter/core.ot");
        let tokens = otterc_lexer::tokenize(source).expect("tokenize core module");
        parse(&tokens).expect("parse core module");
    }

    #[test]
    fn parses_enum_demo_example() {
        let source = include_str!("../../../examples/basic/enum_demo.ot");
        let tokens = otterc_lexer::tokenize(source).expect("tokenize enum demo");
        parse(&tokens).expect("parse enum demo");
    }

    #[test]
    fn parses_chained_member_after_call() {
        let source = "foo().bar().baz";
        let tokens = otterc_lexer::tokenize(source).expect("tokenize chained expression");
        let end = tokens.last().map(|t| t.span().end()).unwrap_or(0);
        let stream = Stream::from_iter(
            end..end + 1,
            tokens
                .iter()
                .map(|token| (token.kind().clone(), token.span().into())),
        );
        let expr = expr_parser()
            .parse(stream)
            .expect("parse chained expression");

        match expr.as_ref() {
            Expr::Member { field, object } => {
                assert_eq!(field, "baz");
                match object.as_ref().as_ref() {
                    Expr::Call { func, args } => {
                        assert!(args.is_empty(), "final call should have no args");
                        match func.as_ref().as_ref() {
                            Expr::Member { field, object } => {
                                assert_eq!(field, "bar");
                                match object.as_ref().as_ref() {
                                    Expr::Call { func, args } => {
                                        assert!(args.is_empty(), "inner call should have no args");
                                        match func.as_ref().as_ref() {
                                            Expr::Identifier(name) => assert_eq!(name, "foo"),
                                            other => panic!(
                                                "expected identifier for inner call, got {:?}",
                                                other
                                            ),
                                        }
                                    }
                                    other => panic!(
                                        "expected call before bar member, got {:?}",
                                        other
                                    ),
                                }
                            }
                            other =>
                                panic!("expected member call for bar, got {:?}", other),
                        }
                    }
                    other => panic!("expected call feeding baz, got {:?}", other),
                }
            }
            other => panic!("expected member expression, got {:?}", other),
        }
    }

    #[test]
    fn parses_member_assignment_statement() {
        let source = "foo.bar = 42\n";
        let tokens = otterc_lexer::tokenize(source).expect("tokenize assignment");
        let program = parse(&tokens).expect("parse assignment");
        assert_eq!(program.statements.len(), 1);
        match program.statements[0].as_ref() {
            Statement::Assignment { target, expr } => {
                match target.as_ref() {
                    Expr::Member { object, field } => {
                        assert_eq!(field, "bar");
                        match object.as_ref().as_ref() {
                            Expr::Identifier(name) => assert_eq!(name, "foo"),
                            other => panic!("expected identifier base, got {:?}", other),
                        }
                    }
                    other => panic!("expected member target, got {:?}", other),
                }

                match expr.as_ref() {
                    Expr::Literal(lit) => match lit.as_ref() {
                        Literal::Number(num) => assert_eq!(num.value as i64, 42),
                        other => panic!("expected numeric literal, got {:?}", other),
                    },
                    other => panic!("expected literal rhs, got {:?}", other),
                }
            }
            other => panic!("expected assignment, got {:?}", other),
        }
    }
}
