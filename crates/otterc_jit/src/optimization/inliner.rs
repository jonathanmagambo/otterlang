use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::call_graph::CallGraph;
use otterc_ast::nodes::{
    Block, Expr, FStringPart, Function, Literal, MatchArm, Node, Pattern, Program, Statement,
};

/// Configuration for the inliner.
#[derive(Debug, Clone)]
pub struct InlineConfig {
    /// Maximum number of statements (recursive) a callee may contain to be eligible for inlining.
    pub max_inline_size: usize,
    /// Maximum recursive inlining depth to avoid exponential growth.
    pub max_depth: usize,
    /// Only inline calls that involve a hot caller/callee pair.
    pub inline_hot_only: bool,
}

impl Default for InlineConfig {
    fn default() -> Self {
        Self {
            max_inline_size: 64,
            max_depth: 3,
            inline_hot_only: true,
        }
    }
}

/// Summary of inline activity.
#[derive(Debug, Default, Clone)]
pub struct InlineStats {
    pub attempted: usize,
    pub applied: usize,
    pub skipped_missing: usize,
    pub skipped_size: usize,
    pub skipped_cold: usize,
    pub skipped_recursive: usize,
    pub skipped_complex: usize,
}

/// Inlines function calls for optimization
pub struct Inliner {
    config: InlineConfig,
    inline_id: AtomicUsize,
}

impl Inliner {
    pub fn new() -> Self {
        Self::with_config(InlineConfig::default())
    }

    pub fn with_config(config: InlineConfig) -> Self {
        Self {
            config,
            inline_id: AtomicUsize::new(0),
        }
    }

    pub fn config(&self) -> &InlineConfig {
        &self.config
    }

    /// Produce an optimized clone of `program` with eligible calls inlined.
    pub fn inline_program(
        &self,
        program: &Program,
        hot_functions: &HashSet<String>,
        call_graph: &CallGraph,
    ) -> (Program, InlineStats) {
        let mut stats = InlineStats::default();
        let mut optimized = program.clone();
        let function_map = Self::index_functions(program);

        let ctx = InlineContext {
            function_map: &function_map,
            hot_functions,
            call_graph,
        };

        for stmt in &mut optimized.statements {
            if let Statement::Function(func) = stmt.as_mut() {
                let mut stack = vec![func.as_ref().name.clone()];
                self.inline_function(func, &ctx, &mut stack, &mut stats, 0);
            }
        }

        (optimized, stats)
    }

    fn inline_function(
        &self,
        function: &mut Node<Function>,
        ctx: &InlineContext<'_>,
        stack: &mut Vec<String>,
        stats: &mut InlineStats,
        depth: usize,
    ) {
        if depth >= self.config.max_depth {
            return;
        }
        let name = function.as_ref().name.clone();
        let current_hot = ctx.hot_functions.contains(&name);
        self.inline_block(
            &mut function.as_mut().body,
            ctx,
            stack,
            stats,
            depth,
            current_hot,
            &name,
        );
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "TODO: Combine these arguments into a struct"
    )]
    fn inline_block(
        &self,
        block: &mut Node<Block>,
        ctx: &InlineContext<'_>,
        stack: &mut Vec<String>,
        stats: &mut InlineStats,
        depth: usize,
        current_hot: bool,
        current_name: &str,
    ) {
        let mut transformed = Vec::with_capacity(block.as_ref().statements.len());

        for stmt in block.as_mut().statements.drain(..) {
            self.inline_statement(
                stmt,
                ctx,
                stack,
                stats,
                depth,
                current_hot,
                current_name,
                &mut transformed,
            );
        }

        block.as_mut().statements = transformed;
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "TODO: Combine these arguments into a struct"
    )]
    fn inline_statement(
        &self,
        stmt: Node<Statement>,
        ctx: &InlineContext<'_>,
        stack: &mut Vec<String>,
        stats: &mut InlineStats,
        depth: usize,
        current_hot: bool,
        current_name: &str,
        out: &mut Vec<Node<Statement>>,
    ) {
        let (stmt, span) = stmt.into_parts();
        match stmt {
            Statement::Let {
                name,
                ty,
                expr,
                public,
            } => {
                let annotation = ty.clone();
                let mut expr_clone = expr.clone();
                if let Some(mut snippet) = self.try_inline_expr(
                    &mut expr_clone,
                    true,
                    ctx,
                    stack,
                    stats,
                    depth,
                    current_hot,
                    current_name,
                ) {
                    self.emit_snippet(&mut snippet, ctx, stack, stats, depth, out);
                    let value = snippet.result_expr.unwrap_or(Node::new(
                        Expr::Literal(Node::new(Literal::Unit, span)),
                        span,
                    ));
                    out.push(Node::new(
                        Statement::Let {
                            name,
                            ty: annotation.clone(),
                            expr: value,
                            public,
                        },
                        span,
                    ));
                } else {
                    let mut expr = expr;
                    self.inline_expr(
                        &mut expr,
                        ctx,
                        stack,
                        stats,
                        depth,
                        current_hot,
                        current_name,
                    );
                    out.push(Node::new(
                        Statement::Let {
                            name,
                            ty: annotation,
                            expr,
                            public,
                        },
                        span,
                    ));
                }
            }
            Statement::Assignment { target, expr } => {
                let mut expr_clone = expr.clone();
                if let Some(mut snippet) = self.try_inline_expr(
                    &mut expr_clone,
                    true,
                    ctx,
                    stack,
                    stats,
                    depth,
                    current_hot,
                    current_name,
                ) {
                    self.emit_snippet(&mut snippet, ctx, stack, stats, depth, out);
                    let value = snippet
                        .result_expr
                        .unwrap_or(Node::new(
                            Expr::Literal(Node::new(Literal::Unit, span)),
                            span,
                        ));
                    out.push(Node::new(
                        Statement::Assignment {
                            target: target.clone(),
                            expr: value,
                        },
                        span,
                    ));
                } else {
                    let mut expr = expr;
                    self.inline_expr(
                        &mut expr,
                        ctx,
                        stack,
                        stats,
                        depth,
                        current_hot,
                        current_name,
                    );
                    out.push(Node::new(
                        Statement::Assignment {
                            target,
                            expr,
                        },
                        span,
                    ));
                }
            }
            Statement::Expr(mut expr) => {
                if let Some(mut snippet) = self.try_inline_expr(
                    &mut expr,
                    false,
                    ctx,
                    stack,
                    stats,
                    depth,
                    current_hot,
                    current_name,
                ) {
                    self.emit_snippet(&mut snippet, ctx, stack, stats, depth, out);
                } else {
                    self.inline_expr(
                        &mut expr,
                        ctx,
                        stack,
                        stats,
                        depth,
                        current_hot,
                        current_name,
                    );
                    out.push(Node::new(Statement::Expr(expr), span));
                }
            }
            Statement::If {
                mut cond,
                mut then_block,
                mut elif_blocks,
                mut else_block,
            } => {
                self.inline_expr(
                    &mut cond,
                    ctx,
                    stack,
                    stats,
                    depth,
                    current_hot,
                    current_name,
                );
                self.inline_block(
                    &mut then_block,
                    ctx,
                    stack,
                    stats,
                    depth,
                    current_hot,
                    current_name,
                );
                for (_, block) in &mut elif_blocks {
                    self.inline_block(block, ctx, stack, stats, depth, current_hot, current_name);
                }
                if let Some(ref mut blk) = else_block {
                    self.inline_block(blk, ctx, stack, stats, depth, current_hot, current_name);
                }
                out.push(Node::new(
                    Statement::If {
                        cond,
                        then_block,
                        elif_blocks,
                        else_block,
                    },
                    span,
                ));
            }
            Statement::While { mut cond, mut body } => {
                self.inline_expr(
                    &mut cond,
                    ctx,
                    stack,
                    stats,
                    depth,
                    current_hot,
                    current_name,
                );
                self.inline_block(
                    &mut body,
                    ctx,
                    stack,
                    stats,
                    depth,
                    current_hot,
                    current_name,
                );
                out.push(Node::new(Statement::While { cond, body }, span));
            }
            Statement::For {
                var,
                mut iterable,
                mut body,
            } => {
                self.inline_expr(
                    &mut iterable,
                    ctx,
                    stack,
                    stats,
                    depth,
                    current_hot,
                    current_name,
                );
                self.inline_block(
                    &mut body,
                    ctx,
                    stack,
                    stats,
                    depth,
                    current_hot,
                    current_name,
                );
                out.push(Node::new(
                    Statement::For {
                        var,
                        iterable,
                        body,
                    },
                    span,
                ));
            }
            Statement::Block(mut inner) => {
                self.inline_block(
                    &mut inner,
                    ctx,
                    stack,
                    stats,
                    depth,
                    current_hot,
                    current_name,
                );
                out.push(Node::new(Statement::Block(inner), span));
            }
            // Exception handling (try/except/finally/raise) removed - use Result<T, E> pattern matching instead
            other => out.push(Node::new(other, span)),
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "TODO: Combine these arguments into a struct"
    )]
    fn inline_expr(
        &self,
        expr: &mut Node<Expr>,
        ctx: &InlineContext<'_>,
        stack: &mut Vec<String>,
        stats: &mut InlineStats,
        depth: usize,
        current_hot: bool,
        current_name: &str,
    ) {
        match expr.as_mut() {
            Expr::Call { func, args } => {
                self.inline_expr(func, ctx, stack, stats, depth, current_hot, current_name);
                for arg in args {
                    self.inline_expr(arg, ctx, stack, stats, depth, current_hot, current_name);
                }
            }
            Expr::Binary { left, right, .. } => {
                self.inline_expr(left, ctx, stack, stats, depth, current_hot, current_name);
                self.inline_expr(right, ctx, stack, stats, depth, current_hot, current_name);
            }
            Expr::Unary { expr: inner, .. } => {
                self.inline_expr(inner, ctx, stack, stats, depth, current_hot, current_name);
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.inline_expr(cond, ctx, stack, stats, depth, current_hot, current_name);
                self.inline_expr(
                    then_branch,
                    ctx,
                    stack,
                    stats,
                    depth,
                    current_hot,
                    current_name,
                );
                if let Some(branch) = else_branch.as_mut() {
                    self.inline_expr(branch, ctx, stack, stats, depth, current_hot, current_name);
                }
            }
            Expr::Match { value, arms } => {
                self.inline_expr(value, ctx, stack, stats, depth, current_hot, current_name);
                for arm in arms {
                    if let Some(guard) = &mut arm.as_mut().guard {
                        self.inline_expr(
                            guard,
                            ctx,
                            stack,
                            stats,
                            depth,
                            current_hot,
                            current_name,
                        );
                    }
                    self.inline_block(
                        &mut arm.as_mut().body,
                        ctx,
                        stack,
                        stats,
                        depth,
                        current_hot,
                        current_name,
                    );
                }
            }
            Expr::Array(values) => {
                for value in values {
                    self.inline_expr(value, ctx, stack, stats, depth, current_hot, current_name);
                }
            }
            Expr::Dict(pairs) => {
                for (key, value) in pairs {
                    self.inline_expr(key, ctx, stack, stats, depth, current_hot, current_name);
                    self.inline_expr(value, ctx, stack, stats, depth, current_hot, current_name);
                }
            }
            Expr::ListComprehension {
                element,
                iterable,
                condition,
                ..
            } => {
                self.inline_expr(element, ctx, stack, stats, depth, current_hot, current_name);
                self.inline_expr(
                    iterable,
                    ctx,
                    stack,
                    stats,
                    depth,
                    current_hot,
                    current_name,
                );
                if let Some(cond) = condition {
                    self.inline_expr(cond, ctx, stack, stats, depth, current_hot, current_name);
                }
            }
            Expr::DictComprehension {
                key,
                value,
                iterable,
                condition,
                ..
            } => {
                self.inline_expr(key, ctx, stack, stats, depth, current_hot, current_name);
                self.inline_expr(value, ctx, stack, stats, depth, current_hot, current_name);
                self.inline_expr(
                    iterable,
                    ctx,
                    stack,
                    stats,
                    depth,
                    current_hot,
                    current_name,
                );
                if let Some(cond) = condition {
                    self.inline_expr(cond, ctx, stack, stats, depth, current_hot, current_name);
                }
            }
            Expr::FString { parts } => {
                for part in parts {
                    if let FStringPart::Expr(expr) = part.as_mut() {
                        self.inline_expr(expr, ctx, stack, stats, depth, current_hot, current_name);
                    }
                }
            }
            // Lambda expressions removed - use anonymous fn syntax instead
            Expr::Spawn(expr) | Expr::Await(expr) => {
                self.inline_expr(expr, ctx, stack, stats, depth, current_hot, current_name);
            }
            Expr::Struct { fields, .. } => {
                for (_, value) in fields {
                    self.inline_expr(value, ctx, stack, stats, depth, current_hot, current_name);
                }
            }
            _ => {}
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "TODO: Combine these arguments into a struct"
    )]
    fn try_inline_expr(
        &self,
        expr: &mut Node<Expr>,
        needs_result: bool,
        ctx: &InlineContext<'_>,
        stack: &mut [String],
        stats: &mut InlineStats,
        depth: usize,
        current_hot: bool,
        current_name: &str,
    ) -> Option<InlineSnippet> {
        if let Expr::Call { func, args } = expr.as_mut()
            && let Expr::Identifier(name) = func.as_ref().as_ref()
        {
            return self.try_inline_call(
                name,
                args,
                ctx,
                stack,
                stats,
                depth,
                current_hot,
                current_name,
                needs_result,
            );
        }
        None
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "TODO: Combine these arguments into a struct"
    )]
    fn try_inline_call(
        &self,
        callee_name: &str,
        args: &[Node<Expr>],
        ctx: &InlineContext<'_>,
        stack: &mut [String],
        stats: &mut InlineStats,
        _depth: usize,
        current_hot: bool,
        _current_name: &str,
        needs_result: bool,
    ) -> Option<InlineSnippet> {
        stats.attempted += 1;

        let Some(callee) = ctx.function_map.get(callee_name) else {
            stats.skipped_missing += 1;
            return None;
        };

        if args.len() != callee.as_ref().params.len() {
            stats.skipped_complex += 1;
            return None;
        }

        if stack.contains(&callee_name.to_string()) {
            stats.skipped_recursive += 1;
            return None;
        }

        if self.config.inline_hot_only && !current_hot && !ctx.hot_functions.contains(callee_name) {
            stats.skipped_cold += 1;
            return None;
        }

        let size = callee.as_ref().body.as_ref().recursive_count();
        if size > self.config.max_inline_size {
            stats.skipped_size += 1;
            return None;
        }

        if Self::has_internal_return(&callee.as_ref().body) {
            stats.skipped_complex += 1;
            return None;
        }

        let inline_id = self.next_inline_id();
        let mut builder = InlineBuilder::new(inline_id);
        let snippet = builder.build_snippet(callee, args);

        if needs_result && snippet.result_expr.is_none() {
            stats.skipped_complex += 1;
            return None;
        }

        stats.applied += 1;
        Some(InlineSnippet {
            callee: callee_name.to_string(),
            block: snippet.block,
            result_expr: snippet.result_expr,
        })
    }

    fn emit_snippet(
        &self,
        snippet: &mut InlineSnippet,
        ctx: &InlineContext<'_>,
        stack: &mut Vec<String>,
        stats: &mut InlineStats,
        depth: usize,
        out: &mut Vec<Node<Statement>>,
    ) {
        stack.push(snippet.callee.clone());
        let callee_hot = ctx.hot_functions.contains(&snippet.callee);
        self.inline_block(
            &mut snippet.block,
            ctx,
            stack,
            stats,
            depth + 1,
            callee_hot,
            &snippet.callee,
        );
        stack.pop();

        out.append(&mut snippet.block.as_mut().statements);
    }

    fn has_internal_return(block: &Node<Block>) -> bool {
        if block.as_ref().statements.is_empty() {
            return false;
        }

        for (idx, stmt) in block.as_ref().statements.iter().enumerate() {
            let is_last = idx == block.as_ref().statements.len() - 1;
            match stmt.as_ref() {
                Statement::Return(_) => {
                    if !is_last {
                        return true;
                    }
                }
                Statement::If {
                    then_block,
                    elif_blocks,
                    else_block,
                    ..
                } => {
                    if Self::has_internal_return(then_block) {
                        return true;
                    }
                    for (_, block) in elif_blocks {
                        if Self::has_internal_return(block) {
                            return true;
                        }
                    }
                    if let Some(block) = else_block
                        && Self::has_internal_return(block)
                    {
                        return true;
                    }
                }
                Statement::While { body, .. }
                | Statement::For { body, .. }
                | Statement::Block(body) => {
                    if Self::has_internal_return(body) {
                        return true;
                    }
                }
                // Exception handling (try/except/finally/raise) removed
                _ => {}
            }
        }

        false
    }

    fn next_inline_id(&self) -> usize {
        self.inline_id.fetch_add(1, Ordering::Relaxed)
    }

    fn index_functions(program: &Program) -> HashMap<String, Node<Function>> {
        let mut map = HashMap::new();
        for stmt in &program.statements {
            if let Statement::Function(func) = stmt.as_ref() {
                map.insert(func.as_ref().name.clone(), func.clone());
            }
        }
        map
    }
}

impl Default for Inliner {
    fn default() -> Self {
        Self::new()
    }
}

struct InlineSnippet {
    callee: String,
    block: Node<Block>,
    result_expr: Option<Node<Expr>>,
}

struct InlineContext<'a> {
    function_map: &'a HashMap<String, Node<Function>>,
    hot_functions: &'a HashSet<String>,
    #[expect(dead_code, reason = "Work in progress")]
    call_graph: &'a CallGraph,
}

struct BuiltSnippet {
    block: Node<Block>,
    result_expr: Option<Node<Expr>>,
}

struct InlineBuilder {
    names: InlineNameGenerator,
}

impl InlineBuilder {
    fn new(inline_id: usize) -> Self {
        Self {
            names: InlineNameGenerator::new(inline_id),
        }
    }

    fn build_snippet(&mut self, callee: &Node<Function>, args: &[Node<Expr>]) -> BuiltSnippet {
        let mut statements = Vec::new();
        let inline_id = self.names.id();
        for (idx, param) in callee.as_ref().params.iter().enumerate() {
            let arg = args[idx].clone();
            let param_name = self.names.register_param(
                param.as_ref().name.as_ref(),
                format!("__inl{}_arg{}", inline_id, idx),
            );
            statements.push(Node::new(
                Statement::Let {
                    name: Node::new(param_name, *param.as_ref().name.span()),
                    ty: param.as_ref().ty.clone(),
                    expr: arg,
                    public: false,
                },
                *param.span(),
            ));
        }

        let body = self.rewrite_body(&callee.as_ref().body);
        statements.extend(body.statements);

        BuiltSnippet {
            block: Node::new(Block { statements }, *callee.span()),
            result_expr: body.return_expr,
        }
    }

    fn rewrite_body(&mut self, block: &Node<Block>) -> InlineBody {
        let mut statements = Vec::new();
        let tail_return = block
            .as_ref()
            .statements
            .last()
            .and_then(|stmt| match stmt.as_ref() {
                Statement::Return(expr) => expr.clone(),
                _ => None,
            });

        let tail_index = block.as_ref().statements.len().saturating_sub(1);

        for (idx, stmt) in block.as_ref().statements.iter().enumerate() {
            if matches!(stmt.as_ref(), Statement::Return(_)) && idx == tail_index {
                break;
            }
            statements.push(self.rewrite_statement(stmt));
        }

        InlineBody {
            statements,
            return_expr: tail_return.map(|expr| self.rewrite_expr(&expr)),
        }
    }

    fn rewrite_statement(&mut self, stmt: &Node<Statement>) -> Node<Statement> {
        stmt.clone().map(|stmt| match stmt {
            Statement::Let {
                name,
                ty,
                expr,
                public,
            } => Statement::Let {
                name: name.map(|name| self.names.rename_local(&name)),
                ty: ty.clone(),
                expr: self.rewrite_expr(&expr),
                public,
            },
            Statement::Assignment { target, expr } => Statement::Assignment {
                target: self.rewrite_expr(&target),
                expr: self.rewrite_expr(&expr),
            },
            Statement::Expr(expr) => Statement::Expr(self.rewrite_expr(&expr)),
            Statement::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
            } => Statement::If {
                cond: self.rewrite_expr(&cond),
                then_block: self.rewrite_nested_block(&then_block),
                elif_blocks: elif_blocks
                    .iter()
                    .map(|(cond, block)| {
                        (self.rewrite_expr(cond), self.rewrite_nested_block(block))
                    })
                    .collect(),
                else_block: else_block
                    .as_ref()
                    .map(|block| self.rewrite_nested_block(block)),
            },
            Statement::For {
                var,
                iterable,
                body,
            } => Statement::For {
                var: var.map(|var| self.names.rename_local(&var)),
                iterable: self.rewrite_expr(&iterable),
                body: self.rewrite_nested_block(&body),
            },
            Statement::While { cond, body } => Statement::While {
                cond: self.rewrite_expr(&cond),
                body: self.rewrite_nested_block(&body),
            },
            Statement::Block(block) => Statement::Block(self.rewrite_nested_block(&block)),
            // Exception handling (try/except/finally/raise) removed
            other => other.clone(),
        })
    }

    fn rewrite_nested_block(&mut self, block: &Node<Block>) -> Node<Block> {
        let mut statements = Vec::with_capacity(block.as_ref().statements.len());
        for stmt in &block.as_ref().statements {
            statements.push(self.rewrite_statement(stmt));
        }
        Node::new(Block { statements }, *block.span())
    }

    fn rewrite_expr(&mut self, expr: &Node<Expr>) -> Node<Expr> {
        expr.clone().map(|expr| match expr {
            Expr::Identifier(name) => Expr::Identifier(self.names.resolve_or_clone(&name)),
            Expr::Binary { op, left, right } => Expr::Binary {
                op,
                left: Box::new(self.rewrite_expr(&left)),
                right: Box::new(self.rewrite_expr(&right)),
            },
            Expr::Unary { op, expr } => Expr::Unary {
                op,
                expr: Box::new(self.rewrite_expr(&expr)),
            },
            Expr::Call { func, args } => Expr::Call {
                func: Box::new(self.rewrite_expr(&func)),
                args: args.iter().map(|arg| self.rewrite_expr(arg)).collect(),
            },
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => Expr::If {
                cond: Box::new(self.rewrite_expr(&cond)),
                then_branch: Box::new(self.rewrite_expr(&then_branch)),
                else_branch: else_branch
                    .as_ref()
                    .map(|branch| Box::new(self.rewrite_expr(branch))),
            },
            Expr::Match { value, arms } => Expr::Match {
                value: Box::new(self.rewrite_expr(&value)),
                arms: arms
                    .into_iter()
                    .map(|arm| {
                        arm.map(|arm| MatchArm {
                            pattern: self.rewrite_pattern(&arm.pattern),
                            guard: arm.guard.as_ref().map(|g| self.rewrite_expr(g)),
                            body: self.rewrite_nested_block(&arm.body),
                        })
                    })
                    .collect(),
            },
            Expr::Array(values) => Expr::Array(
                values
                    .iter()
                    .map(|value| self.rewrite_expr(value))
                    .collect(),
            ),
            Expr::Dict(pairs) => Expr::Dict(
                pairs
                    .iter()
                    .map(|(k, v)| (self.rewrite_expr(k), self.rewrite_expr(v)))
                    .collect(),
            ),
            Expr::ListComprehension {
                element,
                var,
                iterable,
                condition,
            } => Expr::ListComprehension {
                element: Box::new(self.rewrite_expr(&element)),
                var: self.names.rename_local(&var),
                iterable: Box::new(self.rewrite_expr(&iterable)),
                condition: condition
                    .as_ref()
                    .map(|cond| Box::new(self.rewrite_expr(cond))),
            },
            Expr::DictComprehension {
                key,
                value,
                var,
                iterable,
                condition,
            } => Expr::DictComprehension {
                key: Box::new(self.rewrite_expr(&key)),
                value: Box::new(self.rewrite_expr(&value)),
                var: self.names.rename_local(&var),
                iterable: Box::new(self.rewrite_expr(&iterable)),
                condition: condition
                    .as_ref()
                    .map(|cond| Box::new(self.rewrite_expr(cond))),
            },
            Expr::FString { parts } => Expr::FString {
                parts: parts
                    .into_iter()
                    .map(|part| {
                        part.map(|part| match part {
                            FStringPart::Expr(expr) => FStringPart::Expr(self.rewrite_expr(&expr)),
                            FStringPart::Text(text) => FStringPart::Text(text.clone()),
                        })
                    })
                    .collect(),
            },
            // Lambda expressions removed - use anonymous fn syntax instead
            Expr::Spawn(expr) => Expr::Spawn(Box::new(self.rewrite_expr(&expr))),
            Expr::Await(expr) => Expr::Await(Box::new(self.rewrite_expr(&expr))),
            Expr::Struct { name, fields } => Expr::Struct {
                name: name.clone(),
                fields: fields
                    .iter()
                    .map(|(field, value)| (field.clone(), self.rewrite_expr(value)))
                    .collect(),
            },
            _ => expr.clone(),
        })
    }

    fn rewrite_pattern(&mut self, pattern: &Node<Pattern>) -> Node<Pattern> {
        pattern.clone().map(|pattern| match pattern {
            Pattern::Identifier(name) => Pattern::Identifier(self.names.rename_local(&name)),
            Pattern::Struct { name, fields } => Pattern::Struct {
                name: name.clone(),
                fields: fields
                    .into_iter()
                    .map(|(field, pat)| {
                        (field.clone(), pat.map(|inner| self.rewrite_pattern(&inner)))
                    })
                    .collect(),
            },
            Pattern::Array { patterns, rest } => Pattern::Array {
                patterns: patterns
                    .into_iter()
                    .map(|pat| self.rewrite_pattern(&pat))
                    .collect(),
                rest: rest.as_ref().map(|name| self.names.rename_local(name)),
            },
            Pattern::EnumVariant {
                enum_name,
                variant,
                fields,
            } => Pattern::EnumVariant {
                enum_name: enum_name.clone(),
                variant: variant.clone(),
                fields: fields
                    .into_iter()
                    .map(|pat| self.rewrite_pattern(&pat))
                    .collect(),
            },
            _ => pattern.clone(),
        })
    }
}

struct InlineBody {
    statements: Vec<Node<Statement>>,
    return_expr: Option<Node<Expr>>,
}

struct InlineNameGenerator {
    inline_id: usize,
    counter: usize,
    map: HashMap<String, String>,
}

impl InlineNameGenerator {
    fn new(inline_id: usize) -> Self {
        Self {
            inline_id,
            counter: 0,
            map: HashMap::new(),
        }
    }

    fn id(&self) -> usize {
        self.inline_id
    }

    fn register_param(&mut self, original: &str, replacement: String) -> String {
        self.map.insert(original.to_string(), replacement.clone());
        replacement
    }

    fn rename_local(&mut self, original: &str) -> String {
        let name = format!("__inl{}_{}_{}", self.inline_id, original, self.counter);
        self.counter += 1;
        self.map.insert(original.to_string(), name.clone());
        name
    }

    fn resolve_or_clone(&self, original: &str) -> String {
        self.map
            .get(original)
            .cloned()
            .unwrap_or_else(|| original.to_string())
    }
}
