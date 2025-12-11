use otterc_span::Span;
use std::fmt;
use std::hash::{Hash, Hasher};

#[derive(Clone, PartialEq, Eq)]
pub enum TokenKind {
    // Keywords
    Fn,
    Let,
    Return,
    If,
    Else,
    Elif,
    For,
    While,
    Break,
    Continue,
    Pass,
    In,
    Is,
    Not,
    Use,
    As,
    Pub,
    Await,
    Spawn,
    Match,
    Case,
    True,
    False,
    Print,
    None,
    Struct,
    Enum,
    And,
    Or,

    // Identifiers
    Identifier(String),
    UnicodeIdentifier(String),

    // Literals
    Number(String),
    StringLiteral(String),
    FString(String), // Raw f-string content like "π ≈ {result}"
    Bool(bool),

    // Structural
    Colon,
    Newline,
    Indent,
    Dedent,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Dot,

    // Operators
    Arrow,
    Equals,
    EqEq,
    Neq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Pipe,
    Amp,
    Bang,

    // Assignment operators
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,

    // Range operator
    DoubleDot,

    Eof,
}

impl Hash for TokenKind {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            // Keywords - use discriminant for efficiency
            TokenKind::Fn => 0u16.hash(state),
            TokenKind::Let => 1u16.hash(state),
            TokenKind::Return => 2u16.hash(state),
            TokenKind::If => 3u16.hash(state),
            TokenKind::Else => 4u16.hash(state),
            TokenKind::Elif => 5u16.hash(state),
            TokenKind::For => 6u16.hash(state),
            TokenKind::While => 7u16.hash(state),
            TokenKind::Break => 8u16.hash(state),
            TokenKind::Continue => 9u16.hash(state),
            TokenKind::Pass => 10u16.hash(state),
            TokenKind::In => 11u16.hash(state),
            TokenKind::Is => 12u16.hash(state),
            TokenKind::Not => 13u16.hash(state),
            TokenKind::Use => 14u16.hash(state),
            TokenKind::As => 15u16.hash(state),
            TokenKind::Pub => 16u16.hash(state),
            TokenKind::Await => 17u16.hash(state),
            TokenKind::Spawn => 18u16.hash(state),
            TokenKind::Match => 19u16.hash(state),
            TokenKind::Case => 20u16.hash(state),
            TokenKind::True => 21u16.hash(state),
            TokenKind::False => 22u16.hash(state),
            TokenKind::Print => 23u16.hash(state),
            TokenKind::None => 24u16.hash(state),
            TokenKind::Struct => 25u16.hash(state),
            TokenKind::Enum => 26u16.hash(state),
            TokenKind::And => 27u16.hash(state),
            TokenKind::Or => 28u16.hash(state),

            // Identifiers
            TokenKind::Identifier(name) => {
                100u16.hash(state);
                name.hash(state);
            }
            TokenKind::UnicodeIdentifier(name) => {
                101u16.hash(state);
                name.hash(state);
            }

            // Literals
            TokenKind::Number(value) => {
                200u16.hash(state);
                value.hash(state);
            }
            TokenKind::StringLiteral(value) => {
                201u16.hash(state);
                value.hash(state);
            }
            TokenKind::FString(content) => {
                202u16.hash(state);
                content.hash(state);
            }
            TokenKind::Bool(value) => {
                203u16.hash(state);
                value.hash(state);
            }

            // Structural tokens - use their ASCII values for consistency
            TokenKind::Colon => b':'.hash(state),
            TokenKind::Newline => b'\n'.hash(state),
            TokenKind::Indent => 300u16.hash(state),
            TokenKind::Dedent => 301u16.hash(state),
            TokenKind::LParen => b'('.hash(state),
            TokenKind::RParen => b')'.hash(state),
            TokenKind::LBrace => b'{'.hash(state),
            TokenKind::RBrace => b'}'.hash(state),
            TokenKind::LBracket => b'['.hash(state),
            TokenKind::RBracket => b']'.hash(state),
            TokenKind::Comma => b','.hash(state),
            TokenKind::Dot => b'.'.hash(state),

            // Operators
            TokenKind::Arrow => 400u16.hash(state),
            TokenKind::Equals => b'='.hash(state),
            TokenKind::EqEq => 401u16.hash(state),
            TokenKind::Neq => 402u16.hash(state),
            TokenKind::Lt => b'<'.hash(state),
            TokenKind::Gt => b'>'.hash(state),
            TokenKind::LtEq => 403u16.hash(state),
            TokenKind::GtEq => 404u16.hash(state),
            TokenKind::Plus => b'+'.hash(state),
            TokenKind::Minus => b'-'.hash(state),
            TokenKind::Star => b'*'.hash(state),
            TokenKind::Slash => b'/'.hash(state),
            TokenKind::Percent => b'%'.hash(state),
            TokenKind::Pipe => b'|'.hash(state),
            TokenKind::Amp => b'&'.hash(state),
            TokenKind::Bang => b'!'.hash(state),

            // Assignment operators
            TokenKind::PlusEq => 500u16.hash(state),
            TokenKind::MinusEq => 501u16.hash(state),
            TokenKind::StarEq => 502u16.hash(state),
            TokenKind::SlashEq => 503u16.hash(state),

            // Range operator
            TokenKind::DoubleDot => 600u16.hash(state),

            TokenKind::Eof => 999u16.hash(state),
        }
    }
}

// FStringPart is now defined in the AST module

impl TokenKind {
    pub fn name(&self) -> &'static str {
        match self {
            // Keywords
            TokenKind::Fn => "fn",
            TokenKind::Let => "let",
            TokenKind::Return => "return",
            TokenKind::If => "if",
            TokenKind::Else => "else",
            TokenKind::Elif => "elif",
            TokenKind::For => "for",
            TokenKind::While => "while",
            TokenKind::Break => "break",
            TokenKind::Continue => "continue",
            TokenKind::Pass => "pass",
            TokenKind::In => "in",
            TokenKind::Is => "is",
            TokenKind::Not => "not",
            TokenKind::Use => "use",
            TokenKind::As => "as",
            TokenKind::Pub => "pub",
            TokenKind::Await => "await",
            TokenKind::Spawn => "spawn",
            TokenKind::Match => "match",
            TokenKind::Case => "case",
            TokenKind::True => "true",
            TokenKind::False => "false",
            TokenKind::Print => "print",
            TokenKind::None => "None",
            TokenKind::Struct => "struct",
            TokenKind::Enum => "enum",
            TokenKind::And => "and",
            TokenKind::Or => "or",

            // Identifiers
            TokenKind::Identifier(_) => "identifier",
            TokenKind::UnicodeIdentifier(_) => "unicode_identifier",

            // Literals
            TokenKind::Number(_) => "number",
            TokenKind::StringLiteral(_) => "string",
            TokenKind::FString { .. } => "fstring",
            TokenKind::Bool(_) => "bool",

            // Structural
            TokenKind::Colon => ":",
            TokenKind::Newline => "newline",
            TokenKind::Indent => "indent",
            TokenKind::Dedent => "dedent",
            TokenKind::LParen => "(",
            TokenKind::RParen => ")",
            TokenKind::LBrace => "{",
            TokenKind::RBrace => "}",
            TokenKind::LBracket => "[",
            TokenKind::RBracket => "]",
            TokenKind::Comma => ",",
            TokenKind::Dot => ".",

            // Operators
            TokenKind::Arrow => "->",
            TokenKind::Equals => "=",
            TokenKind::EqEq => "==",
            TokenKind::Neq => "!=",
            TokenKind::Lt => "<",
            TokenKind::Gt => ">",
            TokenKind::LtEq => "<=",
            TokenKind::GtEq => ">=",
            TokenKind::Plus => "+",
            TokenKind::Minus => "-",
            TokenKind::Star => "*",
            TokenKind::Slash => "/",
            TokenKind::Percent => "%",
            TokenKind::Pipe => "|",
            TokenKind::Amp => "&",
            TokenKind::Bang => "!",

            // Assignment operators
            TokenKind::PlusEq => "+=",
            TokenKind::MinusEq => "-=",
            TokenKind::StarEq => "*=",
            TokenKind::SlashEq => "/=",

            // Range operator
            TokenKind::DoubleDot => "..",

            TokenKind::Eof => "eof",
        }
    }
}

impl fmt::Debug for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenKind::Identifier(name) => write!(f, "Identifier({name})"),
            TokenKind::UnicodeIdentifier(name) => write!(f, "UnicodeIdentifier({name})"),
            TokenKind::Number(number) => write!(f, "Number({number})"),
            TokenKind::StringLiteral(value) => write!(f, "StringLiteral(\"{value}\")"),
            TokenKind::FString(content) => write!(f, "FString(\"{}\")", content),
            TokenKind::Bool(value) => write!(f, "Bool({value})"),
            kind => f.write_str(kind.name()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    kind: TokenKind,
    span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }

    pub fn kind(&self) -> &TokenKind {
        &self.kind
    }

    pub fn kind_mut(&mut self) -> &mut TokenKind {
        &mut self.kind
    }

    pub fn span(&self) -> Span {
        self.span
    }

    pub fn span_mut(&mut self) -> &mut Span {
        &mut self.span
    }

    pub fn is_keyword(&self) -> bool {
        matches!(
            self.kind,
            TokenKind::Fn
                | TokenKind::Let
                | TokenKind::Return
                | TokenKind::If
                | TokenKind::Else
                | TokenKind::Elif
                | TokenKind::For
                | TokenKind::While
                | TokenKind::Break
                | TokenKind::Continue
                | TokenKind::Pass
                | TokenKind::In
                | TokenKind::Is
                | TokenKind::Not
                | TokenKind::Use
                | TokenKind::As
                | TokenKind::Pub
                | TokenKind::Await
                | TokenKind::Spawn
                | TokenKind::Match
                | TokenKind::Case
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Print
                | TokenKind::None
                | TokenKind::Struct
                | TokenKind::Enum
                | TokenKind::And
                | TokenKind::Or
        )
    }

    pub fn is_literal(&self) -> bool {
        matches!(
            self.kind,
            TokenKind::Number(_)
                | TokenKind::StringLiteral(_)
                | TokenKind::FString(_)
                | TokenKind::Bool(_)
                | TokenKind::None
        )
    }

    pub fn is_identifier(&self) -> bool {
        matches!(
            self.kind,
            TokenKind::Identifier(_) | TokenKind::UnicodeIdentifier(_)
        )
    }

    pub fn is_operator(&self) -> bool {
        matches!(
            self.kind,
            TokenKind::Plus
                | TokenKind::Minus
                | TokenKind::Star
                | TokenKind::Slash
                | TokenKind::Percent
                | TokenKind::Equals
                | TokenKind::EqEq
                | TokenKind::Neq
                | TokenKind::Lt
                | TokenKind::Gt
                | TokenKind::LtEq
                | TokenKind::GtEq
                | TokenKind::Is
                | TokenKind::Not
                | TokenKind::Arrow
                | TokenKind::Pipe
                | TokenKind::Amp
                | TokenKind::Bang
                | TokenKind::PlusEq
                | TokenKind::MinusEq
                | TokenKind::StarEq
                | TokenKind::SlashEq
                | TokenKind::DoubleDot
        )
    }

    pub fn is_structural(&self) -> bool {
        matches!(
            self.kind,
            TokenKind::LParen
                | TokenKind::RParen
                | TokenKind::LBrace
                | TokenKind::RBrace
                | TokenKind::LBracket
                | TokenKind::RBracket
                | TokenKind::Colon
                | TokenKind::Comma
                | TokenKind::Dot
        )
    }
}

impl Hash for Token {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind.hash(state);
        self.span.hash(state);
    }
}
