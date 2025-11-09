use std::hash::{Hash, Hasher};

#[derive(Debug, Clone)]
pub struct Program {
    pub statements: Vec<Statement>,
}

impl Program {
    pub fn new(statements: Vec<Statement>) -> Self {
        Self { statements }
    }

    /// Get all function definitions in the program
    pub fn functions(&self) -> impl Iterator<Item = &Function> {
        self.statements.iter().filter_map(|stmt| {
            if let Statement::Function(func) = stmt {
                Some(func)
            } else {
                None
            }
        })
    }

    /// Count the total number of statements recursively
    pub fn statement_count(&self) -> usize {
        self.statements.iter().map(|s| s.recursive_count()).sum()
    }
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub params: Vec<Param>,
    pub ret_ty: Option<Type>,
    pub body: Block,
    pub public: bool,
}

impl Function {
    pub fn new(
        name: impl Into<String>,
        params: Vec<Param>,
        ret_ty: Option<Type>,
        body: Block,
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
        params: Vec<Param>,
        ret_ty: Option<Type>,
        body: Block,
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
    Generic { base: String, args: Vec<Type> },
    Option(Box<Type>),
    Result { ok: Box<Type>, err: Box<Type> },
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: Option<Type>,
    pub default: Option<Expr>,
}

impl Param {
    pub fn new(name: impl Into<String>, ty: Option<Type>, default: Option<Expr>) -> Self {
        Self {
            name: name.into(),
            ty,
            default,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Block {
    pub statements: Vec<Statement>,
}

impl Block {
    pub fn new(statements: Vec<Statement>) -> Self {
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
pub enum Statement {
    // Variable declarations and assignments
    Let {
        name: String,
        expr: Expr,
        public: bool,
    },
    Assignment {
        name: String,
        expr: Expr,
    },

    // Control flow
    If {
        cond: Box<Expr>,
        then_block: Block,
        elif_blocks: Vec<(Expr, Block)>,
        else_block: Option<Block>,
    },
    For {
        var: String,
        iterable: Expr,
        body: Block,
    },
    While {
        cond: Expr,
        body: Block,
    },
    Break,
    Continue,
    Pass,
    Return(Option<Expr>),

    // Function definitions
    Function(Function),

    // Type definitions
    Struct {
        name: String,
        fields: Vec<(String, Type)>,
        methods: Vec<Function>, // Methods (functions with self parameter)
        public: bool,
        generics: Vec<String>, // Generic type parameters
    },
    TypeAlias {
        name: String,
        target: Type,
        public: bool,
        generics: Vec<String>, // Generic type parameters
    },

    // Expressions as statements
    Expr(Expr),

    // Module imports
    Use {
        imports: Vec<UseImport>,
    },

    // Blocks (for grouping)
    Block(Block),

    // Exception handling
    Try {
        body: Block,
        handlers: Vec<ExceptHandler>,
        else_block: Option<Block>,
        finally_block: Option<Block>,
    },
    Raise(Option<Expr>),
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
            | Statement::Struct { .. }
            | Statement::TypeAlias { .. }
            | Statement::Raise(_) => 1,

            Statement::If {
                then_block,
                elif_blocks,
                else_block,
                ..
            } => {
                let mut count = 1;
                count += then_block.recursive_count();
                for (_, block) in elif_blocks {
                    count += block.recursive_count();
                }
                if let Some(block) = else_block {
                    count += block.recursive_count();
                }
                count
            }
            Statement::For { body, .. } | Statement::While { body, .. } => {
                1 + body.recursive_count()
            }
            Statement::Function(func) => 1 + func.body.recursive_count(),
            Statement::Block(block) => block.recursive_count(),
            Statement::Try {
                body,
                handlers,
                else_block,
                finally_block,
            } => {
                let mut count = 1 + body.recursive_count();
                for handler in handlers {
                    count += handler.body.recursive_count();
                }
                if let Some(block) = else_block {
                    count += block.recursive_count();
                }
                if let Some(block) = finally_block {
                    count += block.recursive_count();
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
        self.statements.iter().map(|s| s.recursive_count()).sum()
    }

    /// Check if block is empty
    pub fn is_empty(&self) -> bool {
        self.statements.is_empty()
    }
}

#[derive(Debug, Clone)]
pub enum Expr {
    // Literals
    Literal(Literal),

    // Variables and access
    Identifier(String),
    Member {
        object: Box<Expr>,
        field: String,
    },

    // Function calls
    Call {
        func: Box<Expr>,
        args: Vec<Expr>,
    },

    // Binary operations
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },

    // Unary operations
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },

    // Control flow expressions
    If {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Option<Box<Expr>>,
    },

    // Match expressions (pattern matching)
    Match {
        value: Box<Expr>,
        arms: Vec<MatchArm>,
    },

    // Range expressions
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
    },

    // Collection literals
    Array(Vec<Expr>),
    Dict(Vec<(Expr, Expr)>), // Key-value pairs
    ListComprehension {
        element: Box<Expr>,
        var: String,
        iterable: Box<Expr>,
        condition: Option<Box<Expr>>,
    },
    DictComprehension {
        key: Box<Expr>,
        value: Box<Expr>,
        var: String,
        iterable: Box<Expr>,
        condition: Option<Box<Expr>>,
    },

    // String interpolation
    FString {
        parts: Vec<FStringPart>,
    },

    // Lambda expressions
    Lambda {
        params: Vec<Param>,
        ret_ty: Option<Type>,
        body: Block,
    },

    // Async operations
    Await(Box<Expr>),
    Spawn(Box<Expr>),

    // Struct instantiation
    Struct {
        name: String,
        fields: Vec<(String, Expr)>, // field name -> value
    },
}

/// Match arm for pattern matching
#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
}

/// Exception handler for try/except blocks
#[derive(Debug, Clone)]
pub struct ExceptHandler {
    pub exception: Option<Type>,
    pub alias: Option<String>,
    pub body: Block,
}

impl ExceptHandler {
    pub fn new(exception: Option<Type>, alias: Option<String>, body: Block) -> Self {
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
    Literal(Literal),
    /// Identifier pattern (binds to variable)
    Identifier(String),
    /// Tuple/struct pattern (Point { x, y })
    Struct {
        name: String,
        fields: Vec<(String, Option<Pattern>)>, // field name and optional nested pattern
    },
    /// Array/list pattern ([a, b, ..rest])
    Array {
        patterns: Vec<Pattern>,
        rest: Option<String>, // Variable name for rest pattern
    },
}

#[derive(Debug, Clone)]
pub enum FStringPart {
    Text(String),
    Expr(Box<Expr>),
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
