use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::call_graph::CallGraph;
use ast::nodes::{
    Block, Expr, FStringPart, Function, Literal, MatchArm, Pattern, Program, Statement,
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
            if let Statement::Function(func) = stmt {
                let mut stack = vec![func.name.clone()];
                self.inline_function(func, &ctx, &mut stack, &mut stats, 0);
            }
        }

        (optimized, stats)
    }

    fn inline_function(
        &self,
        function: &mut Function,
        ctx: &InlineContext<'_>,
        stack: &mut Vec<String>,
        stats: &mut InlineStats,
        depth: usize,
    ) {
        if depth >= self.config.max_depth {
            return;
        }
        let current_hot = ctx.hot_functions.contains(&function.name);
        self.inline_block(
            &mut function.body,
            ctx,
            stack,
            stats,
            depth,
            current_hot,
            &function.name,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn inline_block(
        &self,
        block: &mut Block,
        ctx: &InlineContext<'_>,
        stack: &mut Vec<String>,
        stats: &mut InlineStats,
        depth: usize,
        current_hot: bool,
        current_name: &str,
    ) {
        let mut transformed = Vec::with_capacity(block.statements.len());

        for stmt in block.statements.drain(..) {
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

        block.statements = transformed;
    }

    #[allow(clippy::too_many_arguments)]
    fn inline_statement(
        &self,
        stmt: Statement,
        ctx: &InlineContext<'_>,
        stack: &mut Vec<String>,
        stats: &mut InlineStats,
        depth: usize,
        current_hot: bool,
        current_name: &str,
        out: &mut Vec<Statement>,
    ) {
        match stmt {
            Statement::Let {
                name,
                ty,
                expr,
                public,
                span,
            } => {
                let annotation = ty.clone();
                let expr_clone = expr.clone();
                if let Some(mut snippet) = self.try_inline_expr(
                    expr_clone,
                    true,
                    ctx,
                    stack,
                    stats,
                    depth,
                    current_hot,
                    current_name,
                ) {
                    self.emit_snippet(&mut snippet, ctx, stack, stats, depth, out);
                    let value = snippet.result_expr.unwrap_or(Expr::Literal(Literal::Unit));
                    out.push(Statement::Let {
                        name,
                        ty: annotation.clone(),
                        expr: value,
                        public,
                        span,
                    });
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
                    out.push(Statement::Let {
                        name,
                        ty: annotation,
                        expr,
                        public,
                        span,
                    });
                }
            }
            Statement::Assignment { name, expr, span } => {
                let expr_clone = expr.clone();
                if let Some(mut snippet) = self.try_inline_expr(
                    expr_clone,
                    true,
                    ctx,
                    stack,
                    stats,
                    depth,
                    current_hot,
                    current_name,
                ) {
                    self.emit_snippet(&mut snippet, ctx, stack, stats, depth, out);
                    let value = snippet.result_expr.unwrap_or(Expr::Literal(Literal::Unit));
                    out.push(Statement::Assignment {
                        name,
                        expr: value,
                        span,
                    });
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
                    out.push(Statement::Assignment { name, expr, span });
                }
            }
            Statement::Expr(mut expr) => {
                if let Some(mut snippet) = self.try_inline_expr(
                    expr.clone(),
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
                    out.push(Statement::Expr(expr));
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
                out.push(Statement::If {
                    cond,
                    then_block,
                    elif_blocks,
                    else_block,
                });
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
                out.push(Statement::While { cond, body });
            }
            Statement::For {
                var,
                mut iterable,
                mut body,
                var_span,
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
                out.push(Statement::For {
                    var,
                    iterable,
                    body,
                    var_span,
                });
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
                out.push(Statement::Block(inner));
            }
            Statement::Try {
                mut body,
                mut handlers,
                mut else_block,
                mut finally_block,
            } => {
                self.inline_block(
                    &mut body,
                    ctx,
                    stack,
                    stats,
                    depth,
                    current_hot,
                    current_name,
                );
                for handler in &mut handlers {
                    self.inline_block(
                        &mut handler.body,
                        ctx,
                        stack,
                        stats,
                        depth,
                        current_hot,
                        current_name,
                    );
                }
                if let Some(ref mut blk) = else_block {
                    self.inline_block(blk, ctx, stack, stats, depth, current_hot, current_name);
                }
                if let Some(ref mut blk) = finally_block {
                    self.inline_block(blk, ctx, stack, stats, depth, current_hot, current_name);
                }
                out.push(Statement::Try {
                    body,
                    handlers,
                    else_block,
                    finally_block,
                });
            }
            other => out.push(other),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn inline_expr(
        &self,
        expr: &mut Expr,
        ctx: &InlineContext<'_>,
        stack: &mut Vec<String>,
        stats: &mut InlineStats,
        depth: usize,
        current_hot: bool,
        current_name: &str,
    ) {
        match expr {
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
                    if let Some(guard) = &mut arm.guard {
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
                    self.inline_expr(
                        &mut arm.body,
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
                    if let FStringPart::Expr(expr) = part {
                        self.inline_expr(expr, ctx, stack, stats, depth, current_hot, current_name);
                    }
                }
            }
            Expr::Lambda { body, .. } => {
                self.inline_block(body, ctx, stack, stats, depth, current_hot, current_name);
            }
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

    #[allow(clippy::too_many_arguments)]
    fn try_inline_expr(
        &self,
        expr: Expr,
        needs_result: bool,
        ctx: &InlineContext<'_>,
        stack: &mut [String],
        stats: &mut InlineStats,
        depth: usize,
        current_hot: bool,
        current_name: &str,
    ) -> Option<InlineSnippet> {
        if let Expr::Call { func, args } = expr
            && let Expr::Identifier { name, .. } = *func
        {
            return self.try_inline_call(
                &name,
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

    #[allow(clippy::too_many_arguments)]
    fn try_inline_call(
        &self,
        callee_name: &str,
        args: Vec<Expr>,
        ctx: &InlineContext<'_>,
        stack: &mut [String],
        stats: &mut InlineStats,
        _depth: usize,
        current_hot: bool,
        _current_name: &str,
        needs_result: bool,
    ) -> Option<InlineSnippet> {
        stats.attempted += 1;

        let callee = match ctx.function_map.get(callee_name) {
            Some(func) => func,
            None => {
                stats.skipped_missing += 1;
                return None;
            }
        };

        if args.len() != callee.params.len() {
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

        let size = callee.body.recursive_count();
        if size > self.config.max_inline_size {
            stats.skipped_size += 1;
            return None;
        }

        if Self::has_internal_return(&callee.body) {
            stats.skipped_complex += 1;
            return None;
        }

        let inline_id = self.next_inline_id();
        let mut builder = InlineBuilder::new(inline_id);
        let snippet = builder.build_snippet(callee, &args);

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
        out: &mut Vec<Statement>,
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

        out.append(&mut snippet.block.statements);
    }

    fn has_internal_return(block: &Block) -> bool {
        if block.statements.is_empty() {
            return false;
        }

        for (idx, stmt) in block.statements.iter().enumerate() {
            let is_last = idx == block.statements.len() - 1;
            match stmt {
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
                Statement::Try {
                    body,
                    handlers,
                    else_block,
                    finally_block,
                } => {
                    if Self::has_internal_return(body) {
                        return true;
                    }
                    for handler in handlers {
                        if Self::has_internal_return(&handler.body) {
                            return true;
                        }
                    }
                    if let Some(block) = else_block
                        && Self::has_internal_return(block)
                    {
                        return true;
                    }
                    if let Some(block) = finally_block
                        && Self::has_internal_return(block)
                    {
                        return true;
                    }
                }
                _ => {}
            }
        }

        false
    }

    fn next_inline_id(&self) -> usize {
        self.inline_id.fetch_add(1, Ordering::Relaxed)
    }

    fn index_functions(program: &Program) -> HashMap<String, Function> {
        let mut map = HashMap::new();
        for stmt in &program.statements {
            if let Statement::Function(func) = stmt {
                map.insert(func.name.clone(), func.clone());
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
    block: Block,
    result_expr: Option<Expr>,
}

struct InlineContext<'a> {
    function_map: &'a HashMap<String, Function>,
    hot_functions: &'a HashSet<String>,
    #[allow(dead_code)]
    call_graph: &'a CallGraph,
}

struct BuiltSnippet {
    block: Block,
    result_expr: Option<Expr>,
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

    fn build_snippet(&mut self, callee: &Function, args: &[Expr]) -> BuiltSnippet {
        let mut statements = Vec::new();
        let inline_id = self.names.id();
        for (idx, param) in callee.params.iter().enumerate() {
            let arg = args[idx].clone();
            let param_name = self
                .names
                .register_param(&param.name, format!("__inl{}_arg{}", inline_id, idx));
            statements.push(Statement::Let {
                name: param_name,
                ty: param.ty.clone(),
                expr: arg,
                public: false,
                span: param.span,
            });
        }

        let body = self.rewrite_body(&callee.body);
        statements.extend(body.statements);

        BuiltSnippet {
            block: Block { statements },
            result_expr: body.return_expr,
        }
    }

    fn rewrite_body(&mut self, block: &Block) -> InlineBody {
        let mut statements = Vec::new();
        let tail_return = block.statements.last().and_then(|stmt| match stmt {
            Statement::Return(expr) => expr.clone(),
            _ => None,
        });

        let tail_index = block.statements.len().saturating_sub(1);

        for (idx, stmt) in block.statements.iter().enumerate() {
            if matches!(stmt, Statement::Return(_)) && idx == tail_index {
                break;
            }
            statements.push(self.rewrite_statement(stmt));
        }

        InlineBody {
            statements,
            return_expr: tail_return.map(|expr| self.rewrite_expr(&expr)),
        }
    }

    fn rewrite_statement(&mut self, stmt: &Statement) -> Statement {
        match stmt {
            Statement::Let {
                name,
                ty,
                expr,
                public,
                span,
            } => Statement::Let {
                name: self.names.rename_local(name),
                ty: ty.clone(),
                expr: self.rewrite_expr(expr),
                public: *public,
                span: *span,
            },
            Statement::Assignment { name, expr, span } => Statement::Assignment {
                name: self.names.resolve_or_clone(name),
                expr: self.rewrite_expr(expr),
                span: *span,
            },
            Statement::Expr(expr) => Statement::Expr(self.rewrite_expr(expr)),
            Statement::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
            } => Statement::If {
                cond: Box::new(self.rewrite_expr(cond)),
                then_block: self.rewrite_nested_block(then_block),
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
                var_span,
            } => Statement::For {
                var: self.names.rename_local(var),
                iterable: self.rewrite_expr(iterable),
                body: self.rewrite_nested_block(body),
                var_span: *var_span,
            },
            Statement::While { cond, body } => Statement::While {
                cond: self.rewrite_expr(cond),
                body: self.rewrite_nested_block(body),
            },
            Statement::Block(block) => Statement::Block(self.rewrite_nested_block(block)),
            Statement::Try {
                body,
                handlers,
                else_block,
                finally_block,
            } => Statement::Try {
                body: self.rewrite_nested_block(body),
                handlers: handlers
                    .iter()
                    .map(|handler| {
                        let mut new_handler = handler.clone();
                        new_handler.body = self.rewrite_nested_block(&handler.body);
                        new_handler
                    })
                    .collect(),
                else_block: else_block
                    .as_ref()
                    .map(|block| self.rewrite_nested_block(block)),
                finally_block: finally_block
                    .as_ref()
                    .map(|block| self.rewrite_nested_block(block)),
            },
            other => other.clone(),
        }
    }

    fn rewrite_nested_block(&mut self, block: &Block) -> Block {
        let mut statements = Vec::with_capacity(block.statements.len());
        for stmt in &block.statements {
            statements.push(self.rewrite_statement(stmt));
        }
        Block { statements }
    }

    fn rewrite_expr(&mut self, expr: &Expr) -> Expr {
        match expr {
            Expr::Identifier { name, span } => Expr::Identifier {
                name: self.names.resolve_or_clone(name),
                span: *span,
            },
            Expr::Binary { op, left, right } => Expr::Binary {
                op: *op,
                left: Box::new(self.rewrite_expr(left)),
                right: Box::new(self.rewrite_expr(right)),
            },
            Expr::Unary { op, expr } => Expr::Unary {
                op: *op,
                expr: Box::new(self.rewrite_expr(expr)),
            },
            Expr::Call { func, args } => Expr::Call {
                func: Box::new(self.rewrite_expr(func)),
                args: args.iter().map(|arg| self.rewrite_expr(arg)).collect(),
            },
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => Expr::If {
                cond: Box::new(self.rewrite_expr(cond)),
                then_branch: Box::new(self.rewrite_expr(then_branch)),
                else_branch: else_branch
                    .as_ref()
                    .map(|branch| Box::new(self.rewrite_expr(branch))),
            },
            Expr::Match { value, arms } => Expr::Match {
                value: Box::new(self.rewrite_expr(value)),
                arms: arms
                    .iter()
                    .map(|arm| MatchArm {
                        pattern: self.rewrite_pattern(&arm.pattern),
                        guard: arm.guard.as_ref().map(|g| self.rewrite_expr(g)),
                        body: self.rewrite_expr(&arm.body),
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
                element: Box::new(self.rewrite_expr(element)),
                var: self.names.rename_local(var),
                iterable: Box::new(self.rewrite_expr(iterable)),
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
                key: Box::new(self.rewrite_expr(key)),
                value: Box::new(self.rewrite_expr(value)),
                var: self.names.rename_local(var),
                iterable: Box::new(self.rewrite_expr(iterable)),
                condition: condition
                    .as_ref()
                    .map(|cond| Box::new(self.rewrite_expr(cond))),
            },
            Expr::FString { parts } => Expr::FString {
                parts: parts
                    .iter()
                    .map(|part| match part {
                        FStringPart::Expr(expr) => {
                            FStringPart::Expr(Box::new(self.rewrite_expr(expr)))
                        }
                        FStringPart::Text(text) => FStringPart::Text(text.clone()),
                    })
                    .collect(),
            },
            Expr::Lambda {
                params,
                ret_ty,
                body,
            } => Expr::Lambda {
                params: params.clone(),
                ret_ty: ret_ty.clone(),
                body: self.rewrite_nested_block(body),
            },
            Expr::Spawn(expr) => Expr::Spawn(Box::new(self.rewrite_expr(expr))),
            Expr::Await(expr) => Expr::Await(Box::new(self.rewrite_expr(expr))),
            Expr::Struct { name, fields } => Expr::Struct {
                name: name.clone(),
                fields: fields
                    .iter()
                    .map(|(field, value)| (field.clone(), self.rewrite_expr(value)))
                    .collect(),
            },
            _ => expr.clone(),
        }
    }

    fn rewrite_pattern(&mut self, pattern: &Pattern) -> Pattern {
        match pattern {
            Pattern::Identifier(name) => Pattern::Identifier(self.names.rename_local(name)),
            Pattern::Struct { name, fields } => Pattern::Struct {
                name: name.clone(),
                fields: fields
                    .iter()
                    .map(|(field, pat)| {
                        (
                            field.clone(),
                            pat.as_ref().map(|inner| self.rewrite_pattern(inner)),
                        )
                    })
                    .collect(),
            },
            Pattern::Array { patterns, rest } => Pattern::Array {
                patterns: patterns
                    .iter()
                    .map(|pat| self.rewrite_pattern(pat))
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
                fields: fields.iter().map(|pat| self.rewrite_pattern(pat)).collect(),
            },
            _ => pattern.clone(),
        }
    }
}

struct InlineBody {
    statements: Vec<Statement>,
    return_expr: Option<Expr>,
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
