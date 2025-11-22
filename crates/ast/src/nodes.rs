use std::convert::{AsMut, AsRef};
use std::fmt::{Debug, Display};
use std::hash::{Hash, Hasher};

use common::Span;

/// A node in the AST with an associated span.
#[derive(Debug, Clone)]
pub struct Node<T> {
    value: T,
    span: Span,
}

impl<T> Node<T> {
    pub fn new(value: T, span: impl Into<Span>) -> Self {
        Self {
            value,
            span: span.into(),
        }
    }

    pub fn into_inner(self) -> T {
        self.value
    }

    pub fn into_parts(self) -> (T, Span) {
        (self.value, self.span)
    }

    pub fn span(&self) -> &Span {
        &self.span
    }

    pub fn map<U, F>(self, f: F) -> Node<U>
    where
        F: FnOnce(T) -> U,
    {
        Node {
            value: f(self.value),
            span: self.span,
        }
    }
}

impl<T> AsRef<T> for Node<T> {
    fn as_ref(&self) -> &T {
        &self.value
    }
}

impl<T> AsMut<T> for Node<T> {
    fn as_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

impl<T> PartialEq for Node<T>
where
    T: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T> Eq for Node<T> where T: Eq {}

impl<T> Hash for Node<T>
where
    T: Hash,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl<T> Display for Node<T>
where
    T: Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.value)
    }
}

#[derive(Debug, Clone)]
pub struct Program {
    pub statements: Vec<Node<Statement>>,
}

impl Program {
    pub fn new(statements: Vec<Node<Statement>>) -> Self {
        Self { statements }
    }

    /// Get all function definitions in the program
    pub fn functions(&self) -> impl Iterator<Item = &Node<Function>> {
        self.statements.iter().filter_map(|stmt| {
            if let Statement::Function(func) = stmt.as_ref() {
                Some(func)
            } else {
                None
            }
        })
    }

    /// Count the total number of statements recursively
    pub fn statement_count(&self) -> usize {
        self.statements
            .iter()
            .map(|s| s.as_ref().recursive_count())
            .sum()
    }
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub params: Vec<Node<Param>>,
    pub ret_ty: Option<Node<Type>>,
    pub body: Node<Block>,
    pub public: bool,
}

impl Function {
    pub fn new(
        name: impl Into<String>,
        params: Vec<Node<Param>>,
        ret_ty: Option<Node<Type>>,
        body: Node<Block>,
    ) -> Self {
        Self {
            name: name.into(),
            params,
            ret_ty,
            body,
            public: false,
        }
    }

    pub fn new_public(
        name: impl Into<String>,
        params: Vec<Node<Param>>,
        ret_ty: Option<Node<Type>>,
        body: Node<Block>,
    ) -> Self {
        Self {
            name: name.into(),
            params,
            ret_ty,
            body,
            public: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Simple(String),
    Generic { base: String, args: Vec<Node<Type>> },
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: Node<String>,
    pub ty: Option<Node<Type>>,
    pub default: Option<Node<Expr>>,
}

impl Param {
    pub fn new(name: Node<String>, ty: Option<Node<Type>>, default: Option<Node<Expr>>) -> Self {
        Self { name, ty, default }
    }
}

#[derive(Debug, Clone)]
pub struct Block {
    pub statements: Vec<Node<Statement>>,
}

impl Block {
    pub fn new(statements: Vec<Node<Statement>>) -> Self {
        Self { statements }
    }
}

#[derive(Debug, Clone)]
pub struct UseImport {
    pub module: String,
    pub alias: Option<String>,
}

impl UseImport {
    pub fn new(module: impl Into<String>, alias: Option<String>) -> Self {
        Self {
            module: module.into(),
            alias,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name: String,
    pub fields: Vec<Node<Type>>,
}

impl EnumVariant {
    pub fn new(name: impl Into<String>, fields: Vec<Node<Type>>) -> Self {
        Self {
            name: name.into(),
            fields,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Statement {
    // Variable declarations and assignments
    Let {
        name: Node<String>,
        expr: Node<Expr>,
        ty: Option<Node<Type>>,
        public: bool,
    },
    Assignment {
        name: Node<String>,
        expr: Node<Expr>,
    },

    // Control flow
    If {
        cond: Node<Expr>,
        then_block: Node<Block>,
        elif_blocks: Vec<(Node<Expr>, Node<Block>)>, // Vec<(condition, block)>
        else_block: Option<Node<Block>>,
    },
    For {
        var: Node<String>,
        iterable: Node<Expr>,
        body: Node<Block>,
    },
    While {
        cond: Node<Expr>,
        body: Node<Block>,
    },
    Break,
    Continue,
    Pass,
    Return(Option<Node<Expr>>),

    // Function definitions
    Function(Node<Function>),

    // Type definitions
    Struct {
        name: String,
        fields: Vec<(String, Node<Type>)>,
        methods: Vec<Node<Function>>, // Methods (functions with self parameter)
        public: bool,
        generics: Vec<String>, // Generic type parameters
    },
    Enum {
        name: String,
        variants: Vec<Node<EnumVariant>>,
        public: bool,
        generics: Vec<String>,
    },
    TypeAlias {
        name: String,
        target: Node<Type>,
        public: bool,
        generics: Vec<String>, // Generic type parameters
    },

    // Expressions as statements
    Expr(Node<Expr>),

    // Module imports
    Use {
        imports: Vec<Node<UseImport>>,
    },

    // Re-exports
    PubUse {
        module: String,
        item: Option<String>,  // None means re-export all public items
        alias: Option<String>, // Optional rename
    },

    // Blocks (for grouping)
    Block(Node<Block>),

    // Exception handling
    Try {
        body: Node<Block>,
        handlers: Vec<Node<ExceptHandler>>,
        else_block: Option<Node<Block>>,
        finally_block: Option<Node<Block>>,
    },
    Raise(Option<Node<Expr>>),
}

impl Statement {
    /// Recursively count statements
    pub fn recursive_count(&self) -> usize {
        match self {
            Statement::Let { .. }
            | Statement::Assignment { .. }
            | Statement::Break
            | Statement::Continue
            | Statement::Pass
            | Statement::Return(_)
            | Statement::Expr(_)
            | Statement::Use { .. }
            | Statement::PubUse { .. }
            | Statement::Struct { .. }
            | Statement::Enum { .. }
            | Statement::TypeAlias { .. }
            | Statement::Raise(_) => 1,

            Statement::If {
                then_block,
                elif_blocks,
                else_block,
                ..
            } => {
                let mut count = 1;
                count += then_block.as_ref().recursive_count();
                for (_, block) in elif_blocks {
                    count += block.as_ref().recursive_count();
                }
                if let Some(block) = else_block {
                    count += block.as_ref().recursive_count();
                }
                count
            }
            Statement::For { body, .. } | Statement::While { body, .. } => {
                1 + body.as_ref().recursive_count()
            }
            Statement::Function(func) => 1 + func.as_ref().body.as_ref().recursive_count(),
            Statement::Block(block) => block.as_ref().recursive_count(),
            Statement::Try {
                body,
                handlers,
                else_block,
                finally_block,
            } => {
                let mut count = 1 + body.as_ref().recursive_count();
                for handler in handlers {
                    count += handler.as_ref().body.as_ref().recursive_count();
                }
                if let Some(block) = else_block {
                    count += block.as_ref().recursive_count();
                }
                if let Some(block) = finally_block {
                    count += block.as_ref().recursive_count();
                }
                count
            }
        }
    }

    /// Check if statement is pure (has no side effects)
    pub fn is_pure(&self) -> bool {
        matches!(
            self,
            Statement::Let { .. } | Statement::Break | Statement::Continue | Statement::Pass
        )
    }
}

impl Block {
    /// Recursively count statements
    pub fn recursive_count(&self) -> usize {
        self.statements
            .iter()
            .map(|s| s.as_ref().recursive_count())
            .sum()
    }

    /// Check if block is empty
    pub fn is_empty(&self) -> bool {
        self.statements.is_empty()
    }
}

#[derive(Debug, Clone)]
pub enum Expr {
    // Literals
    Literal(Node<Literal>),

    // Variables and access
    Identifier(String),
    Member {
        object: Box<Node<Expr>>,
        field: String,
    },

    // Function calls
    Call {
        func: Box<Node<Expr>>,
        args: Vec<Node<Expr>>,
    },

    // Binary operations
    Binary {
        op: BinaryOp,
        left: Box<Node<Expr>>,
        right: Box<Node<Expr>>,
    },

    // Unary operations
    Unary {
        op: UnaryOp,
        expr: Box<Node<Expr>>,
    },

    // Control flow expressions
    If {
        cond: Box<Node<Expr>>,
        then_branch: Box<Node<Expr>>,
        else_branch: Option<Box<Node<Expr>>>,
    },

    // Match expressions (pattern matching)
    Match {
        value: Box<Node<Expr>>,
        arms: Vec<Node<MatchArm>>,
    },

    // Range expressions
    Range {
        start: Box<Node<Expr>>,
        end: Box<Node<Expr>>,
    },

    // Collection literals
    Array(Vec<Node<Expr>>),
    Dict(Vec<(Node<Expr>, Node<Expr>)>), // Key-value pairs
    ListComprehension {
        element: Box<Node<Expr>>,
        var: String,
        iterable: Box<Node<Expr>>,
        condition: Option<Box<Node<Expr>>>,
    },
    DictComprehension {
        key: Box<Node<Expr>>,
        value: Box<Node<Expr>>,
        var: String,
        iterable: Box<Node<Expr>>,
        condition: Option<Box<Node<Expr>>>,
    },

    // String interpolation
    FString {
        parts: Vec<Node<FStringPart>>,
    },

    // Lambda expressions
    Lambda {
        params: Vec<Node<Param>>,
        ret_ty: Option<Node<Type>>,
        body: Node<Block>,
    },

    // Async operations
    Await(Box<Node<Expr>>),
    Spawn(Box<Node<Expr>>),

    // Struct instantiation
    Struct {
        name: String,
        fields: Vec<(String, Node<Expr>)>, // field name -> value
    },
}

/// Match arm for pattern matching
#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Node<Pattern>,
    pub guard: Option<Node<Expr>>,
    pub body: Node<Expr>,
}

/// Exception handler for try/except blocks
#[derive(Debug, Clone)]
pub struct ExceptHandler {
    pub exception: Option<Node<Type>>,
    pub alias: Option<String>,
    pub body: Node<Block>,
}

impl ExceptHandler {
    pub fn new(exception: Option<Node<Type>>, alias: Option<String>, body: Node<Block>) -> Self {
        Self {
            exception,
            alias,
            body,
        }
    }
}

/// Pattern for match expressions
#[derive(Debug, Clone)]
pub enum Pattern {
    /// Wildcard pattern (_)
    Wildcard,
    /// Literal pattern (1, true, "hello")
    Literal(Node<Literal>),
    /// Identifier pattern (binds to variable)
    Identifier(String),
    /// Enum variant pattern (Enum.Variant(...))
    EnumVariant {
        enum_name: String,
        variant: String,
        fields: Vec<Node<Pattern>>,
    },
    /// Tuple/struct pattern (Point { x, y })
    Struct {
        name: String,
        fields: Vec<(String, Option<Node<Pattern>>)>, // field name and optional nested pattern
    },
    /// Array/list pattern ([a, b, ..rest])
    Array {
        patterns: Vec<Node<Pattern>>,
        rest: Option<String>, // Variable name for rest pattern
    },
}

#[derive(Debug, Clone)]
pub enum FStringPart {
    Text(String),
    Expr(Node<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,

    // Comparison
    Eq,
    Ne,
    Lt,
    Gt,
    LtEq,
    GtEq,
    Is,
    IsNot,

    // Logical
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy)]
pub struct NumberLiteral {
    pub value: f64,
    pub is_float_literal: bool,
}

impl NumberLiteral {
    pub fn new(value: f64, is_float_literal: bool) -> Self {
        Self {
            value,
            is_float_literal,
        }
    }
}

impl PartialEq for NumberLiteral {
    fn eq(&self, other: &Self) -> bool {
        self.is_float_literal == other.is_float_literal
            && self.value.to_bits() == other.value.to_bits()
    }
}

impl Eq for NumberLiteral {}

impl Hash for NumberLiteral {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.to_bits().hash(state);
        self.is_float_literal.hash(state);
    }
}

#[derive(Debug, Clone)]
pub enum Literal {
    String(String),
    Number(NumberLiteral),
    Bool(bool),
    None,
    Unit, // Unit literal ()
}

impl PartialEq for Literal {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Literal::String(a), Literal::String(b)) => a == b,
            (Literal::Bool(a), Literal::Bool(b)) => a == b,
            (Literal::Number(a), Literal::Number(b)) => a == b,
            (Literal::None, Literal::None) => true,
            (Literal::Unit, Literal::Unit) => true,
            _ => false,
        }
    }
}

impl Eq for Literal {}

impl Hash for Literal {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Literal::String(s) => {
                0u8.hash(state);
                s.hash(state);
            }
            Literal::Number(n) => {
                1u8.hash(state);
                n.hash(state);
            }
            Literal::Bool(b) => {
                2u8.hash(state);
                b.hash(state);
            }
            Literal::None => {
                3u8.hash(state);
            }
            Literal::Unit => {
                4u8.hash(state);
            }
        }
    }
}
