use std::collections::HashSet;

use super::call_graph::CallGraph;
use super::inliner::{InlineConfig, Inliner};
use crate::codegen::CodegenOptLevel;
use otterc_ast::nodes::{
    BinaryOp, Block, Expr, FStringPart, Function, Literal, Node, NumberLiteral, Program, Statement,
    UnaryOp,
};

/// Re-optimizes hot functions
pub struct Reoptimizer {
    #[expect(dead_code, reason = "Work in progress")]
    opt_level: CodegenOptLevel,
    hot_functions: HashSet<String>,
    inliner: Inliner,
}

impl Reoptimizer {
    pub fn new() -> Self {
        Self::with_opt_level(CodegenOptLevel::Aggressive)
    }

    pub fn with_opt_level(opt_level: CodegenOptLevel) -> Self {
        let inline_config = match opt_level {
            CodegenOptLevel::None => InlineConfig {
                max_inline_size: 24,
                max_depth: 1,
                inline_hot_only: true,
            },
            CodegenOptLevel::Default => InlineConfig {
                max_inline_size: 48,
                max_depth: 2,
                inline_hot_only: true,
            },
            CodegenOptLevel::Aggressive => InlineConfig {
                max_inline_size: 80,
                max_depth: 3,
                inline_hot_only: false,
            },
        };

        Self {
            opt_level,
            hot_functions: HashSet::new(),
            inliner: Inliner::with_config(inline_config),
        }
    }

    /// Provide an explicit set of hot functions discovered by the profiler.
    pub fn set_hot_functions<I, S>(&mut self, hot: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.hot_functions = hot.into_iter().map(|s| s.into()).collect();
    }

    pub fn inliner(&self) -> &Inliner {
        &self.inliner
    }

    /// Re-optimize a function by applying aggressive but semantics-preserving cleanups.
    pub fn reoptimize_function(&self, function: &Function) -> Function {
        let mut optimized = function.clone();
        self.clean_block(optimized.body.as_mut());
        optimized
    }

    /// Optimize hot call paths by inlining and running post-inline cleanups.
    pub fn optimize_hot_paths(&self, program: &Program, call_graph: &CallGraph) -> Program {
        let hot_candidates = if self.hot_functions.is_empty() {
            call_graph.hot_candidates(8)
        } else {
            self.hot_functions.iter().cloned().collect()
        };
        let hot_set: HashSet<String> = hot_candidates.into_iter().collect();

        let (mut optimized, _) = self.inliner.inline_program(program, &hot_set, call_graph);

        for stmt in &mut optimized.statements {
            if let Statement::Function(func) = stmt.as_mut() {
                if hot_set.contains(&func.as_ref().name) {
                    *func = func.clone().map(|func| self.post_inline_optimize(&func));
                } else {
                    *func = func.clone().map(|func| self.reoptimize_function(&func));
                }
            }
        }

        optimized
    }

    /// Apply post-inline optimizations such as dead-code elimination and block flattening.
    pub fn post_inline_optimize(&self, function: &Function) -> Function {
        let mut optimized = function.clone();
        self.clean_block(optimized.body.as_mut());
        self.prune_empty_blocks(optimized.body.as_mut());
        optimized
    }

    fn clean_block(&self, block: &mut Block) {
        self.fold_constants_in_block(block);
        self.remove_dead_statements(block);
    }

    fn fold_constants_in_block(&self, block: &mut Block) {
        let mut rewritten = Vec::with_capacity(block.statements.len());
        for mut stmt in block.statements.drain(..) {
            self.fold_constants_in_statement(stmt.as_mut());
            let (stmt, span) = stmt.into_parts();
            match self.simplify_statement(stmt) {
                StatementTransform::Single(stmt) => rewritten.push(Node::new(*stmt, span)),
                StatementTransform::Many(stmts) => {
                    rewritten.extend(stmts.into_iter().map(|s| Node::new(s, span)));
                }
                StatementTransform::None => {}
            }
        }
        block.statements = rewritten;
    }

    fn fold_constants_in_statement(&self, stmt: &mut Statement) {
        match stmt {
            Statement::Let { expr, .. }
            | Statement::Assignment { expr, .. }
            | Statement::Expr(expr)
            | Statement::Return(Some(expr)) => {
                self.fold_constants_in_expr(expr.as_mut());
            }
            Statement::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
            } => {
                self.fold_constants_in_expr(cond.as_mut());
                self.fold_constants_in_block(then_block.as_mut());
                for (_, block) in elif_blocks {
                    self.fold_constants_in_block(block.as_mut());
                }
                if let Some(block) = else_block {
                    self.fold_constants_in_block(block.as_mut());
                }
            }
            Statement::While { cond, body } => {
                self.fold_constants_in_expr(cond.as_mut());
                self.fold_constants_in_block(body.as_mut());
            }
            Statement::For { iterable, body, .. } => {
                self.fold_constants_in_expr(iterable.as_mut());
                self.fold_constants_in_block(body.as_mut());
            }
            Statement::Block(inner) => self.fold_constants_in_block(inner.as_mut()),
            // Exception handling (try/except/finally/raise) removed
            _ => {}
        }
    }

    fn fold_constants_in_expr(&self, expr: &mut Expr) -> Option<Node<Literal>> {
        match expr {
            Expr::Literal(lit) => Some(lit.clone()),
            Expr::Unary { op, expr: inner } => {
                let literal = self.fold_constants_in_expr(inner.as_mut().as_mut());
                if let Some(lit) = literal
                    && let Some(new_lit) = Self::eval_unary(*op, &lit)
                {
                    *expr = Expr::Literal(new_lit.clone());
                    return Some(new_lit);
                }
                None
            }
            Expr::Binary { op, left, right } => {
                let left_lit = self.fold_constants_in_expr(left.as_mut().as_mut());
                let right_lit = self.fold_constants_in_expr(right.as_mut().as_mut());
                if let (Some(l), Some(r)) = (left_lit, right_lit)
                    && let Some(new_lit) = Self::eval_binary(*op, &l, &r)
                {
                    *expr = Expr::Literal(new_lit.clone());
                    return Some(new_lit);
                }
                None
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let cond_lit = self.fold_constants_in_expr(cond.as_mut().as_mut());
                self.fold_constants_in_expr(then_branch.as_mut().as_mut());
                if let Some(branch) = else_branch.as_mut() {
                    self.fold_constants_in_expr(branch.as_mut().as_mut());
                }
                if let Some(cond_lit) = cond_lit
                    && let Literal::Bool(value) = cond_lit.as_ref()
                {
                    let replacement = if *value {
                        then_branch.clone().into_inner()
                    } else if let Some(branch) = else_branch {
                        branch.clone().into_inner()
                    } else {
                        Expr::Literal(Node::new(Literal::Unit, *cond_lit.span()))
                    };
                    *expr = replacement;
                }
                None
            }
            Expr::Call { func, args } => {
                self.fold_constants_in_expr(func.as_mut().as_mut());
                for arg in args {
                    self.fold_constants_in_expr(arg.as_mut());
                }
                None
            }
            Expr::Array(values) => {
                for value in values {
                    self.fold_constants_in_expr(value.as_mut());
                }
                None
            }
            Expr::Dict(pairs) => {
                for (key, value) in pairs {
                    self.fold_constants_in_expr(key.as_mut());
                    self.fold_constants_in_expr(value.as_mut());
                }
                None
            }
            Expr::ListComprehension {
                element,
                iterable,
                condition,
                ..
            } => {
                self.fold_constants_in_expr(element.as_mut().as_mut());
                self.fold_constants_in_expr(iterable.as_mut().as_mut());
                if let Some(cond) = condition {
                    self.fold_constants_in_expr(cond.as_mut().as_mut());
                }
                None
            }
            Expr::DictComprehension {
                key,
                value,
                iterable,
                condition,
                ..
            } => {
                self.fold_constants_in_expr(key.as_mut().as_mut());
                self.fold_constants_in_expr(value.as_mut().as_mut());
                self.fold_constants_in_expr(iterable.as_mut().as_mut());
                if let Some(cond) = condition {
                    self.fold_constants_in_expr(cond.as_mut().as_mut());
                }
                None
            }
            Expr::Match { value, arms } => {
                self.fold_constants_in_expr(value.as_mut().as_mut());
                for arm in arms {
                    if let Some(guard) = &mut arm.as_mut().guard {
                        self.fold_constants_in_expr(guard.as_mut());
                    }
                    self.fold_constants_in_block(arm.as_mut().body.as_mut());
                }
                None
            }
            Expr::FString { parts } => {
                for part in parts {
                    if let FStringPart::Expr(expr) = part.as_mut() {
                        self.fold_constants_in_expr(expr.as_mut());
                    }
                }
                None
            }
            // Lambda expressions removed - use anonymous fn syntax instead
            Expr::Spawn(expr) | Expr::Await(expr) => {
                self.fold_constants_in_expr(expr.as_mut().as_mut());
                None
            }
            Expr::Struct { fields, .. } => {
                for (_, value) in fields {
                    self.fold_constants_in_expr(value.as_mut());
                }
                None
            }
            _ => None,
        }
    }

    fn eval_unary(op: UnaryOp, literal: &Node<Literal>) -> Option<Node<Literal>> {
        let (literal, span) = literal.clone().into_parts();
        match (op, literal) {
            (UnaryOp::Not, Literal::Bool(value)) => Some(Node::new(Literal::Bool(!value), span)),
            (UnaryOp::Neg, Literal::Number(num)) => Some(Node::new(
                Literal::Number(NumberLiteral::new(-num.value, num.is_float_literal)),
                span,
            )),
            _ => None,
        }
    }

    fn eval_binary(
        op: BinaryOp,
        left: &Node<Literal>,
        right: &Node<Literal>,
    ) -> Option<Node<Literal>> {
        let span = left.span().merge(right.span());
        match op {
            BinaryOp::Add => Self::eval_arithmetic(left.as_ref(), right.as_ref(), |a, b| a + b)
                .map(|lit| Node::new(lit, span)),
            BinaryOp::Sub => Self::eval_arithmetic(left.as_ref(), right.as_ref(), |a, b| a - b)
                .map(|lit| Node::new(lit, span)),
            BinaryOp::Mul => Self::eval_arithmetic(left.as_ref(), right.as_ref(), |a, b| a * b)
                .map(|lit| Node::new(lit, span)),
            BinaryOp::Div => {
                if matches!(right.as_ref(), Literal::Number(n) if n.value == 0.0) {
                    None
                } else {
                    Self::eval_arithmetic(left.as_ref(), right.as_ref(), |a, b| a / b)
                        .map(|lit| Node::new(lit, span))
                }
            }
            BinaryOp::Mod => Self::eval_arithmetic(left.as_ref(), right.as_ref(), |a, b| a % b)
                .map(|lit| Node::new(lit, span)),
            BinaryOp::And => match (left.as_ref(), right.as_ref()) {
                (Literal::Bool(a), Literal::Bool(b)) => {
                    Some(Node::new(Literal::Bool(*a && *b), span))
                }
                _ => None,
            },
            BinaryOp::Or => match (left.as_ref(), right.as_ref()) {
                (Literal::Bool(a), Literal::Bool(b)) => {
                    Some(Node::new(Literal::Bool(*a || *b), span))
                }
                _ => None,
            },
            BinaryOp::Eq => Some(Node::new(Literal::Bool(left == right), span)),
            BinaryOp::Ne => Some(Node::new(Literal::Bool(left != right), span)),
            BinaryOp::Lt | BinaryOp::Gt | BinaryOp::LtEq | BinaryOp::GtEq => {
                if let (Literal::Number(a), Literal::Number(b)) = (left.as_ref(), right.as_ref()) {
                    let result = match op {
                        BinaryOp::Lt => a.value < b.value,
                        BinaryOp::Gt => a.value > b.value,
                        BinaryOp::LtEq => a.value <= b.value,
                        BinaryOp::GtEq => a.value >= b.value,
                        _ => unreachable!(),
                    };
                    Some(Node::new(Literal::Bool(result), span))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn eval_arithmetic<F>(left: &Literal, right: &Literal, op: F) -> Option<Literal>
    where
        F: Fn(f64, f64) -> f64,
    {
        if let (Literal::Number(a), Literal::Number(b)) = (left, right) {
            let value = op(a.value, b.value);
            Some(Literal::Number(NumberLiteral::new(
                value,
                a.is_float_literal || b.is_float_literal,
            )))
        } else {
            None
        }
    }

    fn simplify_statement(&self, stmt: Statement) -> StatementTransform {
        match stmt {
            Statement::Pass => StatementTransform::None,
            Statement::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
            } => {
                if let Expr::Literal(lit) = cond.as_ref()
                    && let Literal::Bool(value) = lit.as_ref()
                {
                    if *value {
                        StatementTransform::Many(
                            then_block
                                .into_inner()
                                .statements
                                .into_iter()
                                .map(|stmt| stmt.into_inner())
                                .collect(),
                        )
                    } else if let Some(block) = else_block {
                        StatementTransform::Many(
                            block
                                .into_inner()
                                .statements
                                .into_iter()
                                .map(|stmt| stmt.into_inner())
                                .collect(),
                        )
                    } else {
                        StatementTransform::None
                    }
                } else if elif_blocks.is_empty()
                    && else_block
                        .as_ref()
                        .map(|block| block.as_ref().statements.is_empty())
                        .unwrap_or(true)
                    && then_block.as_ref().statements.is_empty()
                {
                    StatementTransform::None
                } else {
                    StatementTransform::Single(Box::new(Statement::If {
                        cond,
                        then_block,
                        elif_blocks,
                        else_block,
                    }))
                }
            }
            Statement::Block(block) if block.as_ref().statements.is_empty() => {
                StatementTransform::None
            }
            other => StatementTransform::Single(Box::new(other)),
        }
    }

    fn remove_dead_statements(&self, block: &mut Block) {
        let mut pruned = Vec::with_capacity(block.statements.len());
        let mut terminated = false;

        for stmt in block.statements.drain(..) {
            if terminated {
                break;
            }
            terminated = matches!(
                stmt.as_ref(),
                Statement::Return(_) | Statement::Break | Statement::Continue
            );
            pruned.push(stmt);
        }

        block.statements = pruned;

        for stmt in &mut block.statements {
            match stmt.as_mut() {
                Statement::If {
                    then_block,
                    elif_blocks,
                    else_block,
                    ..
                } => {
                    self.remove_dead_statements(then_block.as_mut());
                    for (_, block) in elif_blocks {
                        self.remove_dead_statements(block.as_mut());
                    }
                    if let Some(block) = else_block {
                        self.remove_dead_statements(block.as_mut());
                    }
                }
                Statement::While { body, .. }
                | Statement::For { body, .. }
                | Statement::Block(body) => self.remove_dead_statements(body.as_mut()),
                // Exception handling (try/except/finally/raise) removed
                _ => {}
            }
        }
    }

    fn prune_empty_blocks(&self, block: &mut Block) {
        let mut flattened = Vec::with_capacity(block.statements.len());
        for mut stmt in block.statements.drain(..) {
            let span = *stmt.span();
            match stmt.as_mut() {
                Statement::Block(inner) => {
                    self.prune_empty_blocks(inner.as_mut());
                    if inner.as_mut().statements.is_empty() {
                        continue;
                    }
                    flattened.push(Node::new(Statement::Block(inner.clone()), span));
                }
                Statement::If {
                    then_block,
                    elif_blocks,
                    else_block,
                    ..
                } => {
                    self.prune_empty_blocks(then_block.as_mut());
                    for (_, block) in elif_blocks {
                        self.prune_empty_blocks(block.as_mut());
                    }
                    if let Some(block) = else_block {
                        self.prune_empty_blocks(block.as_mut());
                    }
                    flattened.push(stmt);
                }
                Statement::While { body, .. } | Statement::For { body, .. } => {
                    self.prune_empty_blocks(body.as_mut());
                    flattened.push(stmt);
                }
                // Exception handling (try/except/finally/raise) removed
                _ => flattened.push(stmt),
            }
        }
        block.statements = flattened;
    }
}

impl Default for Reoptimizer {
    fn default() -> Self {
        Self::new()
    }
}

enum StatementTransform {
    Single(Box<Statement>),
    Many(Vec<Statement>),
    None,
}
