use anyhow::{Result, bail};
use std::collections::HashMap;

use crate::runtime::symbol_registry::{FfiType, SymbolRegistry};
use crate::typecheck::types::{
    EnumDefinition, EnumLayout, StructDefinition, TypeContext, TypeError, TypeInfo,
};
use otterc_ast::nodes::{
    BinaryOp, Block, Expr, FStringPart, Function, Literal, Node, Pattern, Program, Statement, Type,
    UnaryOp, UseImport,
};
use otterc_language::LanguageFeatureFlags;
use otterc_span::Span;

/// Type checker that validates and infers types in OtterLang programs
pub struct TypeChecker {
    errors: Vec<TypeError>,
    context: TypeContext,
    registry: Option<&'static SymbolRegistry>,
    expr_types: HashMap<usize, TypeInfo>,
    expr_types_by_span: HashMap<Span, TypeInfo>,
    expr_spans: HashMap<usize, Span>,
    comprehension_var_types: HashMap<Span, TypeInfo>,
    method_comprehension_spans: HashMap<String, Vec<Span>>,
    method_expr_ids: HashMap<String, Vec<usize>>,
    features: LanguageFeatureFlags,
    /// Current function's return type (if inside a function)
    current_function_return_type: Option<TypeInfo>,
}

#[derive(Debug, Clone, Default)]
pub struct ModuleExports {
    pub module: String,
    pub functions: HashMap<String, TypeInfo>,
    pub variables: HashMap<String, TypeInfo>,
    pub structs: HashMap<String, StructDefinition>,
    pub enums: HashMap<String, EnumDefinition>,
    pub type_aliases: HashMap<String, TypeInfo>,
}

impl ModuleExports {
    pub fn new(module: impl Into<String>) -> Self {
        Self {
            module: module.into(),
            ..Default::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.functions.is_empty()
            && self.variables.is_empty()
            && self.structs.is_empty()
            && self.enums.is_empty()
            && self.type_aliases.is_empty()
    }
}

impl TypeChecker {
    fn collect_generic_usages(&self, ty: &TypeInfo, used: &mut std::collections::HashSet<String>) {
        match ty {
            TypeInfo::Generic { base, args } => {
                if args.is_empty() {
                    // If it has no args, it might be a raw generic parameter
                    used.insert(base.clone());
                } else {
                    // If it has args, the base is likely a type (List, Dict, Struct), so we check args
                    for arg in args {
                        self.collect_generic_usages(arg, used);
                    }
                }
            }
            TypeInfo::List(inner) => self.collect_generic_usages(inner, used),
            TypeInfo::Dict { key, value } => {
                self.collect_generic_usages(key, used);
                self.collect_generic_usages(value, used);
            }
            TypeInfo::Function {
                params,
                return_type,
                ..
            } => {
                for param in params {
                    self.collect_generic_usages(param, used);
                }
                self.collect_generic_usages(return_type, used);
            }
            // Structs and Enums in TypeInfo are usually fully resolved or have their generics in args (if we added args to them)
            // Current TypeInfo::Struct doesn't store args, which is a limitation if we want to track usage inside it.
            // But usually `TypeInfo::Generic` covers the usage *of* a struct with generics.
            // TypeInfo::Struct { .. } | TypeInfo::Enum { .. } => ..
            _ => {}
        }
    }
    fn is_unknown_like(ty: &TypeInfo) -> bool {
        matches!(ty, TypeInfo::Unknown)
            || matches!(ty, TypeInfo::Generic { args, .. } if args.is_empty())
    }

    fn merge_unknown_like_types(left: &TypeInfo, right: &TypeInfo) -> TypeInfo {
        match (Self::is_unknown_like(left), Self::is_unknown_like(right)) {
            (false, false) | (false, true) => left.clone(),
            (true, false) => right.clone(),
            (true, true) => match (left, right) {
                (
                    TypeInfo::Generic { base: b1, args: a1 },
                    TypeInfo::Generic { base: b2, args: a2 },
                ) if a1.is_empty() && a2.is_empty() && b1 == b2 => left.clone(),
                (TypeInfo::Generic { args, .. }, _) if args.is_empty() => left.clone(),
                (_, TypeInfo::Generic { args, .. }) if args.is_empty() => right.clone(),
                _ => TypeInfo::Unknown,
            },
        }
    }

    fn record_expr_type(&mut self, expr: &Node<Expr>, ty: &TypeInfo) {
        let id = expr.as_ref() as *const Expr as usize;
        self.expr_types.insert(id, ty.clone());
        self.expr_types_by_span.insert(*expr.span(), ty.clone());
        self.expr_spans.insert(id, *expr.span());
    }

    pub fn new() -> Self {
        Self::with_language_features(LanguageFeatureFlags::default())
    }

    pub fn with_language_features(features: LanguageFeatureFlags) -> Self {
        let mut context = TypeContext::with_features(features.clone());

        // Register built-in functions
        Self::register_builtins(&mut context);

        // Common type aliases (language-level names to internal types)
        context.define_type_alias("float".to_string(), TypeInfo::F64, true);
        context.define_type_alias("int".to_string(), TypeInfo::I32, true);
        context.define_type_alias("number".to_string(), TypeInfo::F64, true);
        context.define_type_alias("None".to_string(), TypeInfo::Unit, true);
        context.define_type_alias("unit".to_string(), TypeInfo::Unit, true);
        context.define_type_alias(
            "list".to_string(),
            TypeInfo::List(Box::new(TypeInfo::Unknown)),
            true,
        );
        context.define_type_alias(
            "dict".to_string(),
            TypeInfo::Dict {
                key: Box::new(TypeInfo::Unknown),
                value: Box::new(TypeInfo::Unknown),
            },
            true,
        );
        context.define_type_alias("Error".to_string(), TypeInfo::Error, true);
        context.define_type_alias("string".to_string(), TypeInfo::Str, true);

        Self {
            errors: Vec::new(),
            context,
            registry: None,
            expr_types: HashMap::new(),
            expr_types_by_span: HashMap::new(),
            expr_spans: HashMap::new(),
            comprehension_var_types: HashMap::new(),
            method_comprehension_spans: HashMap::new(),
            method_expr_ids: HashMap::new(),
            features,
            current_function_return_type: None,
        }
    }

    pub fn with_registry(mut self, registry: &'static SymbolRegistry) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Register all built-in functions in the type context
    fn register_builtins(context: &mut TypeContext) {
        // print function
        context.functions.insert(
            "print".to_string(),
            TypeInfo::Function {
                params: vec![TypeInfo::Str],
                param_defaults: vec![false],
                return_type: Box::new(TypeInfo::Unit),
            },
        );

        // println function
        context.functions.insert(
            "println".to_string(),
            TypeInfo::Function {
                params: vec![TypeInfo::Str],
                param_defaults: vec![false],
                return_type: Box::new(TypeInfo::Unit),
            },
        );

        context.functions.insert(
            "str".to_string(),
            TypeInfo::Function {
                params: vec![TypeInfo::Unknown],
                param_defaults: vec![false],
                return_type: Box::new(TypeInfo::Str),
            },
        );

        // len functions (accepts string, list, map, etc.)
        context.functions.insert(
            "len".to_string(),
            TypeInfo::Function {
                params: vec![TypeInfo::Unknown],
                param_defaults: vec![false],
                return_type: Box::new(TypeInfo::I64),
            },
        );

        // cap function
        context.functions.insert(
            "cap".to_string(),
            TypeInfo::Function {
                params: vec![TypeInfo::Str],
                param_defaults: vec![false],
                return_type: Box::new(TypeInfo::I64),
            },
        );

        // panic function
        context.functions.insert(
            "panic".to_string(),
            TypeInfo::Function {
                params: vec![TypeInfo::Str],
                param_defaults: vec![false],
                return_type: Box::new(TypeInfo::Unit),
            },
        );
    }

    /// Type check a program
    pub fn check_program(&mut self, program: &Program) -> Result<()> {
        self.register_module_imports(&program.statements);
        // First pass: collect struct definitions, enums, and type aliases
        self.register_type_definitions(&program.statements);

        // Second pass: collect function signatures
        for statement in &program.statements {
            if let Statement::Function(function) = statement.as_ref() {
                let sig = self.infer_function_signature(function);
                self.context
                    .functions
                    .insert(function.as_ref().name.clone(), sig);
            }
        }

        // Third pass: type check function bodies and top-level statements
        for statement in &program.statements {
            let span = statement.span();
            match statement.as_ref() {
                Statement::Function(function) => {
                    self.check_function(function)?;
                }
                Statement::Struct { name, methods, .. } => {
                    self.check_struct_methods(name, methods)?;
                }
                Statement::Let { .. } | Statement::Expr(_) => {
                    // Top-level let and expressions are allowed
                    self.check_statement(statement)?;
                }
                Statement::Enum { .. }
                | Statement::TypeAlias { .. }
                | Statement::Use { .. }
                | Statement::PubUse { .. } => {}
                _ => {
                    self.errors.push(
                        TypeError::new(format!(
                            "unexpected statement at top level: {:?}",
                            statement
                        ))
                        .with_hint("Only function definitions, let statements, and expressions are allowed at the top level".to_string())
                        .with_span(*span),
                    );
                }
            }
        }

        if !self.errors.is_empty() {
            let error_messages: Vec<String> = self.errors.iter().map(|e| e.to_string()).collect();
            bail!("type checking failed:\n{}", error_messages.join("\n\n"));
        }

        Ok(())
    }

    fn check_struct_methods(
        &mut self,
        struct_name: &str,
        methods: &[Node<Function>],
    ) -> Result<()> {
        for method in methods {
            let mut method_clone = method.as_ref().clone();
            method_clone.name = format!("{}.{}", struct_name, method_clone.name);
            self.rewrite_method_self_param(&mut method_clone, struct_name);
            let node = Node::new(method_clone, *method.span());
            self.record_method_metadata(&node.as_ref().name, node.as_ref().body.as_ref());
            self.check_function(&node)?;
        }
        Ok(())
    }

    fn record_method_metadata(&mut self, method_name: &str, body: &Block) {
        let mut spans = Vec::new();
        let mut expr_ids = Vec::new();
        self.collect_metadata_in_block(body, &mut spans, &mut expr_ids);
        if !spans.is_empty() {
            self.method_comprehension_spans
                .entry(method_name.to_string())
                .or_default()
                .extend(spans);
        }
        if !expr_ids.is_empty() {
            self.method_expr_ids
                .entry(method_name.to_string())
                .or_default()
                .extend(expr_ids);
        }
    }

    fn collect_metadata_in_block(
        &self,
        block: &Block,
        spans: &mut Vec<Span>,
        expr_ids: &mut Vec<usize>,
    ) {
        for stmt in &block.statements {
            self.collect_metadata_in_statement(stmt.as_ref(), spans, expr_ids);
        }
    }

    fn collect_metadata_in_statement(
        &self,
        stmt: &Statement,
        spans: &mut Vec<Span>,
        expr_ids: &mut Vec<usize>,
    ) {
        match stmt {
            Statement::Expr(expr)
            | Statement::Let { expr, .. }
            | Statement::Assignment { expr, .. } => {
                self.collect_metadata_in_expr(expr, spans, expr_ids);
            }
            Statement::Return(Some(expr)) => self.collect_metadata_in_expr(expr, spans, expr_ids),
            Statement::Return(None)
            | Statement::Break
            | Statement::Continue
            | Statement::Pass
            | Statement::Use { .. }
            | Statement::PubUse { .. }
            | Statement::Struct { .. }
            | Statement::Enum { .. }
            | Statement::TypeAlias { .. }
            | Statement::Function(_) => {}
            Statement::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
            } => {
                self.collect_metadata_in_expr(cond, spans, expr_ids);
                self.collect_metadata_in_block(then_block.as_ref(), spans, expr_ids);
                for (elif_cond, block) in elif_blocks {
                    self.collect_metadata_in_expr(elif_cond, spans, expr_ids);
                    self.collect_metadata_in_block(block.as_ref(), spans, expr_ids);
                }
                if let Some(block) = else_block {
                    self.collect_metadata_in_block(block.as_ref(), spans, expr_ids);
                }
            }
            Statement::For { iterable, body, .. } => {
                self.collect_metadata_in_expr(iterable, spans, expr_ids);
                self.collect_metadata_in_block(body.as_ref(), spans, expr_ids);
            }
            Statement::While { cond, body } => {
                self.collect_metadata_in_expr(cond, spans, expr_ids);
                self.collect_metadata_in_block(body.as_ref(), spans, expr_ids);
            }
            Statement::Block(block) => {
                self.collect_metadata_in_block(block.as_ref(), spans, expr_ids);
            }
        }
    }

    fn collect_metadata_in_expr(
        &self,
        expr: &Node<Expr>,
        spans: &mut Vec<Span>,
        expr_ids: &mut Vec<usize>,
    ) {
        let id = expr.as_ref() as *const Expr as usize;
        expr_ids.push(id);
        match expr.as_ref() {
            Expr::ListComprehension {
                element,
                iterable,
                condition,
                ..
            } => {
                spans.push(*expr.span());
                self.collect_metadata_in_expr(element, spans, expr_ids);
                self.collect_metadata_in_expr(iterable, spans, expr_ids);
                if let Some(cond) = condition {
                    self.collect_metadata_in_expr(cond, spans, expr_ids);
                }
            }
            Expr::DictComprehension {
                key,
                value,
                iterable,
                condition,
                ..
            } => {
                spans.push(*expr.span());
                self.collect_metadata_in_expr(key, spans, expr_ids);
                self.collect_metadata_in_expr(value, spans, expr_ids);
                self.collect_metadata_in_expr(iterable, spans, expr_ids);
                if let Some(cond) = condition {
                    self.collect_metadata_in_expr(cond, spans, expr_ids);
                }
            }
            Expr::Binary { left, right, .. } => {
                self.collect_metadata_in_expr(left, spans, expr_ids);
                self.collect_metadata_in_expr(right, spans, expr_ids);
            }
            Expr::Unary { expr, .. }
            | Expr::Await(expr)
            | Expr::Spawn(expr)
            | Expr::Member { object: expr, .. } => {
                self.collect_metadata_in_expr(expr, spans, expr_ids);
            }
            Expr::Call { func, args } => {
                self.collect_metadata_in_expr(func, spans, expr_ids);
                for arg in args {
                    self.collect_metadata_in_expr(arg, spans, expr_ids);
                }
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.collect_metadata_in_expr(cond, spans, expr_ids);
                self.collect_metadata_in_expr(then_branch, spans, expr_ids);
                if let Some(expr) = else_branch {
                    self.collect_metadata_in_expr(expr, spans, expr_ids);
                }
            }
            Expr::Match { value, arms } => {
                self.collect_metadata_in_expr(value, spans, expr_ids);
                for arm in arms {
                    if let Some(guard) = &arm.as_ref().guard {
                        self.collect_metadata_in_expr(guard, spans, expr_ids);
                    }
                    self.collect_metadata_in_block(arm.as_ref().body.as_ref(), spans, expr_ids);
                }
            }
            Expr::Range { start, end } => {
                self.collect_metadata_in_expr(start, spans, expr_ids);
                self.collect_metadata_in_expr(end, spans, expr_ids);
            }
            Expr::Array(elements) => {
                for elem in elements {
                    self.collect_metadata_in_expr(elem, spans, expr_ids);
                }
            }
            Expr::Dict(pairs) => {
                for (key, value) in pairs {
                    self.collect_metadata_in_expr(key, spans, expr_ids);
                    self.collect_metadata_in_expr(value, spans, expr_ids);
                }
            }
            Expr::FString { parts } => {
                for part in parts {
                    if let FStringPart::Expr(expr) = part.as_ref() {
                        self.collect_metadata_in_expr(expr, spans, expr_ids);
                    }
                }
            }
            Expr::Struct { fields, .. } => {
                for (_, value) in fields {
                    self.collect_metadata_in_expr(value, spans, expr_ids);
                }
            }
            Expr::Literal(_) | Expr::Identifier(_) => {}
        }
    }

    fn infer_struct_generics_from_instance(
        &mut self,
        struct_name: &str,
        concrete_fields: &HashMap<String, TypeInfo>,
    ) -> HashMap<String, TypeInfo> {
        let mut inferred = HashMap::new();
        if let Some(struct_def) = self.context.get_struct(struct_name).cloned() {
            for (field_name, field_ty) in &struct_def.fields {
                if let Some(concrete_ty) = concrete_fields.get(field_name) {
                    self.infer_generics_from_type_info(
                        field_ty,
                        concrete_ty,
                        &struct_def.generics,
                        &mut inferred,
                    );
                }
            }
        }
        inferred
    }

    fn apply_method_specialization(
        &mut self,
        method_name: &str,
        inferred: &HashMap<String, TypeInfo>,
    ) {
        if inferred.is_empty() {
            return;
        }
        if let Some(spans) = self.method_comprehension_spans.get(method_name) {
            for span in spans {
                if let Some(ty) = self.comprehension_var_types.get_mut(span) {
                    *ty = ty.substitute(inferred);
                }
            }
        }
        if let Some(expr_ids) = self.method_expr_ids.get(method_name) {
            for id in expr_ids {
                if let Some(ty) = self.expr_types.get_mut(id) {
                    *ty = ty.substitute(inferred);
                    if let Some(span) = self.expr_spans.get(id) {
                        self.expr_types_by_span.insert(*span, ty.clone());
                    }
                }
            }
        }
    }

    fn rewrite_method_self_param(&self, method_func: &mut Function, struct_name: &str) {
        let Some(first_param) = method_func.params.first_mut() else {
            return;
        };

        if let Some(ty) = first_param.as_ref().ty.as_ref() {
            if matches!(ty.as_ref(), Type::Simple(name) if name == "Self") {
                let span = *ty.span();
                let replacement = Type::Simple(struct_name.to_string());
                first_param.as_mut().ty = Some(Node::new(replacement, span));
            }
            return;
        }

        if first_param.as_ref().name.as_ref() == "self" {
            let span = *first_param.as_ref().name.span();
            let replacement = Type::Simple(struct_name.to_string());
            first_param.as_mut().ty = Some(Node::new(replacement, span));
        }
    }

    /// Infer function signature from declaration
    fn infer_function_signature(&mut self, function: &Node<Function>) -> TypeInfo {
        let mut param_types = Vec::new();
        let mut param_defaults = Vec::new();
        let mut seen_default = false;

        for param in &function.as_ref().params {
            let explicit_type = param
                .as_ref()
                .ty
                .as_ref()
                .map(|ty| self.context.type_from_annotation(ty));
            let resolved_type = if let Some(ty) = &explicit_type {
                if let TypeInfo::Generic { base, args } = ty {
                    if args.is_empty() {
                        if let Some(aliased_type) = self.context.resolve_type_alias(base) {
                            aliased_type.clone()
                        } else {
                            ty.clone()
                        }
                    } else {
                        ty.clone()
                    }
                } else {
                    ty.clone()
                }
            } else {
                TypeInfo::Unknown
            };

            param_types.push(resolved_type.clone());

            if let Some(default_expr) = &param.as_ref().default {
                seen_default = true;
                param_defaults.push(true);
                if let Ok(default_type) = self.infer_expr_type(default_expr)
                    && let Some(expected) = &explicit_type
                    && !default_type.is_compatible_with(expected)
                {
                    self.errors.push(
                        TypeError::new(format!(
                            "default value for parameter `{}` has type {}, expected {}",
                            param.as_ref().name,
                            default_type.display_name(),
                            expected.display_name()
                        ))
                        .with_hint(
                            "Ensure the default expression matches the declared parameter type"
                                .to_string(),
                        )
                        .with_span(*param.span()),
                    );
                }
            } else {
                if seen_default {
                    self.errors.push(
                        TypeError::new(format!(
                            "parameter `{}` without default cannot follow parameters with defaults",
                            param.as_ref().name
                        ))
                        .with_hint("Move parameters without defaults before parameters that specify defaults".to_string())
                        .with_span(*param.span()),
                    );
                }
                param_defaults.push(false);
            }
        }

        let return_type = if let Some(ty) = &function.as_ref().ret_ty {
            self.context.type_from_annotation(ty)
        } else {
            TypeInfo::Unknown
        };

        TypeInfo::Function {
            params: param_types,
            param_defaults,
            return_type: Box::new(return_type),
        }
    }

    pub fn register_module_definitions(&mut self, program: &Program) {
        self.register_type_definitions(&program.statements);
    }

    fn register_module_imports(&mut self, statements: &[Node<Statement>]) {
        for statement in statements {
            if let Statement::Use { imports } = statement.as_ref() {
                for import in imports {
                    self.try_register_module(import);
                }
            }
        }
    }

    fn try_register_module(&mut self, import: &Node<UseImport>) {
        let Some(registry) = self.registry else {
            return;
        };

        let Some(module_name) = Self::canonical_module_name(&import.as_ref().module) else {
            return;
        };

        if !registry.has_module(&module_name) {
            return;
        }

        registry.activate_module(&module_name);
        let alias = import
            .as_ref()
            .alias
            .clone()
            .unwrap_or_else(|| module_name.clone());
        self.context
            .insert_variable(alias, TypeInfo::Module(module_name));
    }

    fn canonical_module_name(module: &str) -> Option<String> {
        if module.starts_with("rust:") {
            return None;
        }

        let candidate = module.rsplit(':').next().unwrap_or(module);
        if candidate.starts_with('.') || candidate.starts_with('/') {
            return None;
        }

        if candidate.contains('/') || candidate.contains('\\') {
            return None;
        }

        let trimmed = candidate.trim();
        if trimmed.is_empty() {
            return None;
        }

        let canonical = trimmed.rsplit('.').next().unwrap_or(trimmed);
        if canonical.is_empty() {
            None
        } else {
            Some(canonical.to_string())
        }
    }

    fn register_type_definitions(&mut self, statements: &[Node<Statement>]) {
        for statement in statements {
            match statement.as_ref() {
                Statement::Struct {
                    name,
                    fields,
                    methods,
                    generics,
                    public,
                } => {
                    let mut field_types = HashMap::new();
                    for (field_name, field_ty) in fields {
                        let ty = self.context.type_from_annotation(field_ty);
                        field_types.insert(field_name.clone(), ty);
                    }

                    // Validate generic parameters
                    let mut used_generics = std::collections::HashSet::new();
                    for ty in field_types.values() {
                        self.collect_generic_usages(ty, &mut used_generics);
                    }

                    for generic in generics {
                        if !used_generics.contains(generic) {
                            self.errors.push(
                                TypeError::new(format!(
                                    "generic parameter '{}' declared on struct '{}' is never used",
                                    generic, name
                                ))
                                .with_hint(
                                    "remove the unused generic or use it in a field type"
                                        .to_string(),
                                )
                                .with_span(*statement.span()),
                            );
                        }
                    }

                    for used in &used_generics {
                        if !generics.contains(used) {
                            self.errors.push(
                                TypeError::new(format!(
                                    "generic parameter '{}' used in struct '{}' but not declared",
                                    used, name
                                ))
                                .with_hint(format!(
                                    "add '{}' to the struct's generic parameter list",
                                    used
                                ))
                                .with_span(*statement.span()),
                            );
                        }
                    }
                    let definition = StructDefinition {
                        name: name.clone(),
                        generics: generics.clone(),
                        fields: field_types,
                        public: *public,
                    };
                    self.context.define_struct(definition);

                    // Push generic parameters to context for method type checking
                    for generic in generics {
                        self.context.push_generic(generic.clone());
                    }

                    for method in methods {
                        let mut method_clone = method.as_ref().clone();
                        self.rewrite_method_self_param(&mut method_clone, name);
                        let method_name = format!("{}.{}", name, method_clone.name);
                        let method_node = Node::new(method_clone, *method.span());
                        self.record_method_metadata(
                            &method_name,
                            method_node.as_ref().body.as_ref(),
                        );
                        let sig = self.infer_function_signature(&method_node);
                        self.context.insert_function(method_name.clone(), sig);
                    }

                    // Pop generic parameters
                    for _ in generics {
                        self.context.pop_generic();
                    }
                }
                Statement::TypeAlias {
                    name,
                    target,
                    public,
                    ..
                } => {
                    let ty = self.context.type_from_annotation(target);
                    self.context.define_type_alias(name.clone(), ty, *public);
                }
                Statement::Enum {
                    name,
                    variants,
                    generics,
                    public,
                } => {
                    let definition = EnumDefinition {
                        name: name.clone(),
                        generics: generics.clone(),
                        variants: variants.iter().map(|v| v.as_ref()).cloned().collect(),
                        public: *public,
                    };
                    self.context.define_enum(definition);
                }
                _ => {}
            }
        }
    }

    /// Type check a function
    fn check_function(&mut self, function: &Node<Function>) -> Result<()> {
        // Determine function return type
        let return_type = if let Some(ret_ty) = &function.as_ref().ret_ty {
            self.context.type_from_annotation(ret_ty)
        } else {
            TypeInfo::Unit
        };

        let mut fn_context = TypeContext::with_features(self.features.clone());
        fn_context.variables = self.context.variables.clone();

        // Add function parameters to context, overriding any globals/imports
        for param in &function.as_ref().params {
            let param_type = if let Some(ty) = &param.as_ref().ty {
                self.context.type_from_annotation(ty)
            } else {
                TypeInfo::Unknown
            };
            fn_context.insert_variable(param.as_ref().name.as_ref().clone(), param_type);
        }

        // Copy function signatures to inner context
        for (name, sig) in &self.context.functions {
            fn_context.functions.insert(name.clone(), sig.clone());
        }

        fn_context.structs = self.context.structs.clone();
        fn_context.type_aliases = self.context.type_aliases.clone();
        fn_context.enums = self.context.enums.clone();

        // Type check function body with return type tracking
        let old_context = std::mem::replace(&mut self.context, fn_context);
        let old_return_type = self.current_function_return_type.replace(return_type);
        let _ = self.check_block(&function.as_ref().body)?;
        self.context = old_context;
        self.current_function_return_type = old_return_type;

        Ok(())
    }

    /// Check function with generic type parameters
    /// This handles functions that have generic type parameters in their signature
    #[expect(dead_code, reason = "Work in progress")]
    fn check_function_with_generics(&mut self, function: &Node<Function>) -> Result<()> {
        // Extract generic type parameters from function signature
        let mut generic_params = Vec::new();

        // Check parameter types for generic parameters
        for param in &function.as_ref().params {
            if let Some(ty) = &param.as_ref().ty {
                self.extract_generic_params(ty, &mut generic_params);
            }
        }

        if let Some(ret_ty) = &function.as_ref().ret_ty {
            self.extract_generic_params(ret_ty, &mut generic_params);
        }

        // Push generic parameters to context
        for param in &generic_params {
            self.context.push_generic(param.clone());
        }

        // Type check function body
        let result = self.check_function(function);

        // Pop generic parameters
        for _ in &generic_params {
            self.context.pop_generic();
        }

        result
    }

    /// Extract generic type parameter names from a type
    fn extract_generic_params(&self, ty: &Node<Type>, params: &mut Vec<String>) {
        match ty.as_ref() {
            Type::Simple(name) => {
                // Check if this looks like a generic parameter (single uppercase letter or common generic names)
                // In OtterLang, generic parameters are typically single uppercase letters (T, U, etc.)
                if name.len() == 1
                    && name.chars().next().unwrap().is_uppercase()
                    && !params.contains(name)
                {
                    params.push(name.clone());
                }
            }
            Type::Generic { base, args } => {
                // Check if base is a generic parameter
                if base.len() == 1
                    && base.chars().next().unwrap().is_uppercase()
                    && !params.contains(base)
                {
                    params.push(base.clone());
                }
                // Recursively extract from type arguments
                for arg in args {
                    self.extract_generic_params(arg, params);
                }
            }
        }
    }

    fn try_eval_enum_constructor(
        &mut self,
        func: &Node<Expr>,
        args: &[Node<Expr>],
    ) -> Result<Option<TypeInfo>> {
        if let Expr::Member { object, field } = func.as_ref()
            && let Expr::Identifier(enum_name) = object.as_ref().as_ref()
            && let Some(definition) = self.context.get_enum(enum_name).cloned()
        {
            let Some(variant) = definition
                .variants
                .iter()
                .find(|variant| variant.name == *field)
            else {
                self.errors.push(
                    TypeError::new(format!("enum '{}' has no variant '{}'", enum_name, field))
                        .with_span(*object.span()),
                );
                return Ok(Some(TypeInfo::Error));
            };

            let expected_len = variant.fields.len();
            if expected_len != args.len() {
                self.errors.push(
                    TypeError::new(format!(
                        "enum variant '{}.{}' expects {} argument(s), got {}",
                        enum_name,
                        field,
                        expected_len,
                        args.len()
                    ))
                    .with_span(*object.span()),
                );
            }

            let mut arg_types = Vec::new();
            for arg in args {
                arg_types.push(self.infer_expr_type(arg)?);
            }

            for (field_ty, actual_ty) in variant.fields.iter().zip(arg_types.iter()) {
                let expected_type = self.context.type_from_annotation(field_ty);
                if !self.type_contains_enum_generic(field_ty.as_ref(), &definition.generics)
                    && !actual_ty.is_compatible_with(&expected_type)
                {
                    self.errors.push(
                        TypeError::new(format!(
                            "argument for '{}.{}' expects type {}, got {}",
                            enum_name,
                            field,
                            expected_type.display_name(),
                            actual_ty.display_name()
                        ))
                        .with_span(*field_ty.span()),
                    );
                }
            }

            let mut inferred = HashMap::new();
            for (field_ty, actual_ty) in variant.fields.iter().zip(arg_types.iter()) {
                self.infer_enum_generics_from_type(field_ty, actual_ty, &definition, &mut inferred);
            }

            let resolved_args = definition
                .generics
                .iter()
                .map(|name| inferred.get(name).cloned().unwrap_or(TypeInfo::Unknown))
                .collect::<Vec<_>>();

            if let Some(enum_type) = self.context.build_enum_type(enum_name, resolved_args) {
                return Ok(Some(enum_type));
            }

            return Ok(Some(TypeInfo::Error));
        }
        Ok(None)
    }

    fn type_contains_enum_generic(&self, ty: &Type, generics: &[String]) -> bool {
        match ty {
            Type::Simple(name) => generics.contains(name),
            Type::Generic { base, args } => {
                generics.contains(base)
                    || args
                        .iter()
                        .any(|arg| self.type_contains_enum_generic(arg.as_ref(), generics))
            }
        }
    }

    fn infer_generics_from_type(
        &mut self,
        expected: &Node<Type>,
        actual: &TypeInfo,
        generics: &[String],
        inferred: &mut HashMap<String, TypeInfo>,
    ) {
        match expected.as_ref() {
            Type::Simple(name) => {
                if generics.contains(name) {
                    inferred
                        .entry(name.clone())
                        .or_insert_with(|| actual.clone());
                }
            }
            Type::Generic { base, args } => {
                if generics.contains(base) && args.is_empty() {
                    inferred
                        .entry(base.clone())
                        .or_insert_with(|| actual.clone());
                } else if base.eq_ignore_ascii_case("List") {
                    if let TypeInfo::List(inner) = actual
                        && let Some(sub_ty) = args.first()
                    {
                        self.infer_generics_from_type(sub_ty, inner, generics, inferred);
                    }
                } else if base.eq_ignore_ascii_case("Dict") {
                    if let TypeInfo::Dict { key, value } = actual {
                        if let Some(key_ty) = args.first() {
                            self.infer_generics_from_type(key_ty, key, generics, inferred);
                        }
                        if let Some(val_ty) = args.get(1) {
                            self.infer_generics_from_type(val_ty, value, generics, inferred);
                        }
                    }
                } else {
                    match actual {
                        TypeInfo::Generic {
                            base: actual_base,
                            args: actual_args,
                        } if actual_base == base => {
                            for (expected_arg, actual_arg) in args.iter().zip(actual_args.iter()) {
                                self.infer_generics_from_type(
                                    expected_arg,
                                    actual_arg,
                                    generics,
                                    inferred,
                                );
                            }
                        }
                        TypeInfo::Enum {
                            name,
                            args: actual_args,
                            ..
                        } if name == base => {
                            for (expected_arg, actual_arg) in args.iter().zip(actual_args.iter()) {
                                self.infer_generics_from_type(
                                    expected_arg,
                                    actual_arg,
                                    generics,
                                    inferred,
                                );
                            }
                        }
                        TypeInfo::Struct { name, .. } if name == base => {
                            // Structs don't store args in TypeInfo yet, so we can't infer from them easily unless we change TypeInfo::Struct
                            // But for now, we assume if names match, we might be good?
                            // Actually, TypeInfo::Struct doesn't have args, so we can't infer from it.
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn infer_generics_from_type_info(
        &mut self,
        expected: &TypeInfo,
        actual: &TypeInfo,
        generics: &[String],
        inferred: &mut HashMap<String, TypeInfo>,
    ) {
        match expected {
            TypeInfo::Generic { base, args } => {
                if generics.contains(base) && args.is_empty() {
                    inferred
                        .entry(base.clone())
                        .or_insert_with(|| actual.clone());
                } else if !args.is_empty() {
                    match actual {
                        TypeInfo::List(inner) if base.eq_ignore_ascii_case("List") => {
                            if let Some(sub_ty) = args.first() {
                                self.infer_generics_from_type_info(
                                    sub_ty, inner, generics, inferred,
                                );
                            }
                        }
                        TypeInfo::Dict { key, value } if base.eq_ignore_ascii_case("Dict") => {
                            if let Some(key_ty) = args.first() {
                                self.infer_generics_from_type_info(key_ty, key, generics, inferred);
                            }
                            if let Some(val_ty) = args.get(1) {
                                self.infer_generics_from_type_info(
                                    val_ty, value, generics, inferred,
                                );
                            }
                        }
                        TypeInfo::Generic {
                            base: actual_base,
                            args: actual_args,
                        } if actual_base == base => {
                            for (expected_arg, actual_arg) in args.iter().zip(actual_args.iter()) {
                                self.infer_generics_from_type_info(
                                    expected_arg,
                                    actual_arg,
                                    generics,
                                    inferred,
                                );
                            }
                        }
                        TypeInfo::Enum {
                            name,
                            args: actual_args,
                            ..
                        } if name == base => {
                            for (expected_arg, actual_arg) in args.iter().zip(actual_args.iter()) {
                                self.infer_generics_from_type_info(
                                    expected_arg,
                                    actual_arg,
                                    generics,
                                    inferred,
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            TypeInfo::List(inner) => {
                if let TypeInfo::List(actual_inner) = actual {
                    self.infer_generics_from_type_info(inner, actual_inner, generics, inferred);
                }
            }
            TypeInfo::Dict { key, value } => {
                if let TypeInfo::Dict {
                    key: actual_key,
                    value: actual_value,
                } = actual
                {
                    self.infer_generics_from_type_info(key, actual_key, generics, inferred);
                    self.infer_generics_from_type_info(value, actual_value, generics, inferred);
                }
            }
            _ => {}
        }
    }

    fn infer_enum_generics_from_type(
        &mut self,
        expected: &Node<Type>,
        actual: &TypeInfo,
        definition: &EnumDefinition,
        inferred: &mut HashMap<String, TypeInfo>,
    ) {
        self.infer_generics_from_type(expected, actual, &definition.generics, inferred);
    }

    /// Bind variables from a pattern into the type checking context
    fn bind_pattern_variables(&mut self, pattern: &Node<Pattern>, ty: &TypeInfo) {
        match pattern.as_ref() {
            Pattern::Identifier(name) => {
                // Simple identifier pattern binds the whole value
                self.context.insert_variable(name.clone(), ty.clone());
            }
            Pattern::EnumVariant {
                enum_name,
                variant,
                fields,
            } => {
                // Get variant info from the concrete enum type (with generics already substituted)
                // Strategy: Try to build the enum type directly from the pattern's enum name
                // This works even if the type is still Generic after normalization
                let mut bound = false;

                // First, check if the type is already an Enum
                if let TypeInfo::Enum { variants, .. } = &ty
                    && let Some(variant_info) = variants.get(variant)
                    && fields.len() == variant_info.fields.len()
                {
                    for (field_pattern, field_type) in fields.iter().zip(variant_info.fields.iter())
                    {
                        self.bind_pattern_variables(field_pattern, field_type);
                    }
                    bound = true;
                }

                // If not bound yet, try to extract Generic type args and build enum
                if !bound {
                    let args = if let TypeInfo::Generic { base, args } = &ty {
                        if base == enum_name {
                            Some(args.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let Some(args) = args
                        && let Some(built_enum) = self.context.build_enum_type(enum_name, args)
                        && let TypeInfo::Enum { variants, .. } = &built_enum
                        && let Some(variant_info) = variants.get(variant)
                        && fields.len() == variant_info.fields.len()
                    {
                        for (field_pattern, field_type) in
                            fields.iter().zip(variant_info.fields.iter())
                        {
                            self.bind_pattern_variables(field_pattern, field_type);
                        }
                        bound = true;
                    }
                }

                // If still not bound, try normalization as last resort
                if !bound {
                    let normalized = self.context.normalize_type(ty.clone());
                    if let TypeInfo::Enum { variants, .. } = &normalized
                        && let Some(variant_info) = variants.get(variant)
                        && fields.len() == variant_info.fields.len()
                    {
                        for (field_pattern, field_type) in
                            fields.iter().zip(variant_info.fields.iter())
                        {
                            self.bind_pattern_variables(field_pattern, field_type);
                        }
                    }
                }
            }
            Pattern::Struct { name, fields } => {
                // Clone struct fields to avoid borrow conflicts
                let struct_def = self.context.get_struct(name).cloned();
                if let Some(struct_def) = struct_def {
                    for (field_name, nested_pattern) in fields {
                        if let Some(field_type) = struct_def.fields.get(field_name) {
                            if let Some(pattern) = nested_pattern {
                                self.bind_pattern_variables(pattern, field_type);
                            } else {
                                // No nested pattern, bind the field name directly
                                self.context
                                    .insert_variable(field_name.clone(), field_type.clone());
                            }
                        }
                    }
                }
            }
            Pattern::Array { patterns, rest } => {
                if let TypeInfo::List(elem_type) = ty {
                    for pattern in patterns {
                        self.bind_pattern_variables(pattern, elem_type);
                    }
                    if let Some(rest_var) = rest {
                        // Rest pattern gets the list type
                        self.context.insert_variable(rest_var.clone(), ty.clone());
                    }
                }
            }
            Pattern::Wildcard | Pattern::Literal(_) => {
                // No variables to bind
            }
        }
    }

    fn validate_pattern_against_type(&mut self, pattern: &Node<Pattern>, ty: &TypeInfo) {
        match pattern.as_ref() {
            Pattern::Wildcard | Pattern::Identifier(_) => {
                // Wildcard and Identifier binds any type
            }
            Pattern::Literal(lit) => {
                // Check literal type matches expected type
                let lit_type = match lit.as_ref() {
                    Literal::String(_) => TypeInfo::Str,
                    Literal::Number(n) => {
                        if n.value.fract() == 0.0
                            && n.value >= i32::MIN as f64
                            && n.value <= i32::MAX as f64
                        {
                            TypeInfo::I32
                        } else {
                            TypeInfo::F64
                        }
                    }
                    Literal::Bool(_) => TypeInfo::Bool,
                    Literal::None | Literal::Unit => TypeInfo::Unit,
                };

                if !lit_type.is_compatible_with(ty) {
                    self.errors.push(
                        TypeError::new(format!(
                            "literal pattern type {} does not match expected type {}",
                            lit_type.display_name(),
                            ty.display_name()
                        ))
                        .with_span(*pattern.span()),
                    );
                }
            }
            Pattern::EnumVariant {
                enum_name,
                variant,
                fields,
            } => {
                // Check that the value type is an enum and matches the pattern
                match ty {
                    TypeInfo::Enum {
                        name,
                        args: _,
                        variants,
                    } => {
                        if name != enum_name {
                            self.errors.push(
                                TypeError::new(format!(
                                    "enum pattern '{}' does not match value type {}",
                                    enum_name, name
                                ))
                                .with_span(*pattern.span()),
                            );
                            return;
                        }

                        // Find the variant in the enum definition
                        if let Some(variant_def) = variants.get(variant) {
                            if variant_def.fields.len() != fields.len() {
                                self.errors.push(TypeError::new(format!(
                                    "enum variant '{}.{}' has {} field(s), but pattern destructures {}",
                                    enum_name, variant, variant_def.fields.len(), fields.len()
                                )).with_span(*pattern.span()));
                            } else {
                                // Check that nested patterns match field types
                                for (field_pattern, field_type) in
                                    fields.iter().zip(variant_def.fields.iter())
                                {
                                    self.validate_pattern_against_type(field_pattern, field_type);
                                }
                            }
                        } else {
                            self.errors.push(
                                TypeError::new(format!(
                                    "enum '{}' has no variant '{}'",
                                    enum_name, variant
                                ))
                                .with_span(*pattern.span()),
                            );
                        }
                    }
                    TypeInfo::Generic { base, args } if base == enum_name => {
                        // Try to build the enum type from generic
                        if let Some(built_enum) =
                            self.context.build_enum_type(enum_name, args.clone())
                        {
                            self.validate_pattern_against_type(pattern, &built_enum);
                        } else {
                            self.errors.push(
                                TypeError::new(format!(
                                    "cannot resolve generic enum '{}' with args {:?}",
                                    enum_name, args
                                ))
                                .with_span(*pattern.span()),
                            );
                        }
                    }
                    _ => {
                        self.errors.push(
                            TypeError::new(format!(
                                "cannot match enum pattern '{}' against non-enum type {}",
                                enum_name,
                                ty.display_name()
                            ))
                            .with_span(*pattern.span()),
                        );
                    }
                }
            }
            Pattern::Struct { name, fields } => {
                // Check that value type is compatible with struct pattern
                match ty {
                    TypeInfo::Struct {
                        name: struct_name,
                        fields: struct_fields,
                    } => {
                        if name != struct_name {
                            self.errors.push(
                                TypeError::new(format!(
                                    "struct pattern '{}' does not match value type {}",
                                    name, struct_name
                                ))
                                .with_span(*pattern.span()),
                            );
                            return;
                        }

                        // Check each field pattern
                        for (field_name, field_pattern) in fields {
                            if let Some(field_type) = struct_fields.get(field_name) {
                                if let Some(pattern) = field_pattern {
                                    self.validate_pattern_against_type(pattern, field_type);
                                }
                                // If no pattern, just bind the field - no validation needed
                            } else {
                                self.errors.push(
                                    TypeError::new(format!(
                                        "struct '{}' has no field '{}'",
                                        name, field_name
                                    ))
                                    .with_span(*pattern.span()),
                                );
                            }
                        }
                    }
                    _ => {
                        self.errors.push(
                            TypeError::new(format!(
                                "cannot match struct pattern '{}' against non-struct type {}",
                                name,
                                ty.display_name()
                            ))
                            .with_span(*pattern.span()),
                        );
                    }
                }
            }
            Pattern::Array { patterns, rest } => {
                // Check that value type is a list/array
                match ty {
                    TypeInfo::List(elem_type) => {
                        // Check each pattern element
                        for pattern in patterns {
                            self.validate_pattern_against_type(pattern, elem_type);
                        }
                        // Rest pattern is allowed and binds remaining elements
                    }
                    _ => {
                        self.errors.push(
                            TypeError::new(format!(
                                "cannot match array pattern against non-list type {}",
                                ty.display_name()
                            ))
                            .with_span(*pattern.span()),
                        );
                    }
                }

                // Check rest pattern usage
                if let Some(rest_name) = rest
                    && patterns.is_empty()
                    && rest_name.is_empty()
                {
                    self.errors.push(
                        TypeError::new("invalid array pattern: empty rest pattern".to_string())
                            .with_span(*pattern.span()),
                    );
                }
            }
        }
    }

    /// Type check a block
    fn check_block(&mut self, block: &Node<Block>) -> Result<TypeInfo> {
        let mut last_type = TypeInfo::Unit;
        for statement in &block.as_ref().statements {
            last_type = self.check_statement(statement)?;
        }
        Ok(last_type)
    }

    /// Type check a statement
    fn check_statement(&mut self, statement: &Node<Statement>) -> Result<TypeInfo> {
        let span = statement.span();
        match statement.as_ref() {
            Statement::Let { name, ty, expr, .. } => {
                let expr_type = self.infer_expr_type(expr)?;
                if let Some(annotation) = ty {
                    let annotated_type = self.context.type_from_annotation(annotation);
                    if !expr_type.is_compatible_with(&annotated_type) {
                        self.errors.push(
                            TypeError::new(format!(
                                "type mismatch: expected {}, got {}",
                                annotated_type.display_name(),
                                expr_type.display_name()
                            ))
                            .with_hint(format!(
                                "The variable `{}` is declared as `{}`, but the initializer has type `{}`",
                                name,
                                annotated_type.display_name(),
                                expr_type.display_name()
                            ))
                            .with_help(
                                "Update the annotation or change the initializer to match the declared type"
                                    .to_string(),
                            )
                            .with_span(*span),
                        );
                    }
                    self.context
                        .insert_variable(name.as_ref().clone(), annotated_type);
                } else {
                    self.context
                        .insert_variable(name.as_ref().clone(), expr_type);
                }
                Ok(TypeInfo::Unit)
            }
            Statement::Assignment { name, expr } => {
                let var_type = self
                    .context
                    .get_variable(name.as_ref())
                    .ok_or_else(|| {
                        TypeError::new(format!("undefined variable: {}", name))
                            .with_hint(format!("did you mean to declare it with `let {}`?", name))
                            .with_help(
                                "Variables must be declared with `let` before they can be assigned"
                                    .to_string(),
                            )
                            .with_span(*span)
                    })?
                    .clone();

                let expr_type = self.infer_expr_type(expr)?;
                if !expr_type.is_compatible_with(&var_type) {
                    self.errors.push(TypeError::new(format!(
                        "cannot assign {} to {} (expected {})",
                        expr_type.display_name(),
                        name,
                        var_type.display_name()
                    ))
                    .with_hint(format!("The variable `{}` is declared as `{}`, but you're trying to assign a value of type `{}`", name, var_type.display_name(), expr_type.display_name()))
                    .with_help("Make sure the types match or are compatible (e.g., i32 can be promoted to i64 or f64)".to_string())
                    .with_span(*span));
                }
                Ok(TypeInfo::Unit)
            }
            Statement::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
            } => {
                let cond_type = self.infer_expr_type(cond)?;
                if !cond_type.is_compatible_with(&TypeInfo::Bool) {
                    self.errors.push(
                        TypeError::new(format!(
                            "if condition must be bool, got {}",
                            cond_type.display_name()
                        ))
                        .with_span(*span),
                    );
                }

                self.check_block(then_block)?;
                for (_, block) in elif_blocks {
                    self.check_block(block)?;
                }
                if let Some(block) = else_block {
                    self.check_block(block)?;
                }
                Ok(TypeInfo::Unit)
            }
            Statement::For {
                var,
                iterable,
                body,
                ..
            } => {
                let iter_type = self.infer_expr_type(iterable)?;
                let element_type = match &iter_type {
                    TypeInfo::List(elem) => elem.as_ref().clone(),
                    TypeInfo::Dict { value, .. } => value.as_ref().clone(),
                    TypeInfo::Str => TypeInfo::Str,
                    TypeInfo::Range(start, end) => {
                        if matches!(start.as_ref(), TypeInfo::I64)
                            || matches!(end.as_ref(), TypeInfo::I64)
                        {
                            TypeInfo::I64
                        } else {
                            TypeInfo::I32
                        }
                    }
                    _ => {
                        self.errors.push(
                            TypeError::new(format!(
                                "cannot iterate over type {}",
                                iter_type.display_name()
                            ))
                            .with_span(*span),
                        );
                        TypeInfo::Unknown
                    }
                };

                let previous = self.context.remove_variable(var.as_ref());
                self.context
                    .insert_variable(var.as_ref().clone(), element_type);
                self.check_block(body)?;
                match previous {
                    Some(prev) => {
                        self.context.insert_variable(var.as_ref().clone(), prev);
                    }
                    None => {
                        self.context.remove_variable(var.as_ref());
                    }
                }
                Ok(TypeInfo::Unit)
            }
            Statement::While { cond, body } => {
                let cond_type = self.infer_expr_type(cond)?;
                if !cond_type.is_compatible_with(&TypeInfo::Bool) {
                    self.errors.push(
                        TypeError::new(format!(
                            "while condition must be bool, got {}",
                            cond_type.display_name()
                        ))
                        .with_span(*span),
                    );
                }
                self.check_block(body)?;
                Ok(TypeInfo::Unit)
            }
            Statement::Return(expr) => {
                if let Some(expr) = expr {
                    let expr_type = self.infer_expr_type(expr)?;

                    // Check return type matches function signature
                    if let Some(expected_return_type) = &self.current_function_return_type {
                        if !expr_type.is_compatible_with(expected_return_type) {
                            self.errors.push(
                                TypeError::new(format!(
                                    "return type mismatch: expected {}, got {}",
                                    expected_return_type.display_name(),
                                    expr_type.display_name()
                                ))
                                .with_span(*span),
                            );
                        }
                    } else {
                        // We're not inside a function context - this is an error
                        self.errors.push(
                            TypeError::new("return statement outside of function".to_string())
                                .with_span(*span),
                        );
                    }
                } else {
                    // Bare return - check if function expects unit
                    if let Some(expected_return_type) = &self.current_function_return_type {
                        if !expected_return_type.is_compatible_with(&TypeInfo::Unit) {
                            self.errors.push(
                                TypeError::new(format!(
                                    "bare return in function that expects return type {}",
                                    expected_return_type.display_name()
                                ))
                                .with_span(*span),
                            );
                        }
                    } else {
                        // We're not inside a function context - this is an error
                        self.errors.push(
                            TypeError::new("return statement outside of function".to_string())
                                .with_span(*span),
                        );
                    }
                }
                Ok(TypeInfo::Unit)
            }
            Statement::Function(_) => {
                // Functions are handled separately
                Ok(TypeInfo::Unit)
            }
            Statement::Expr(expr) => {
                let expr_type = self.infer_expr_type(expr)?;
                // Expression statements are allowed (e.g., function calls)
                Ok(expr_type)
            }
            Statement::Break | Statement::Continue => {
                // These are handled by loop context
                Ok(TypeInfo::Unit)
            }
            Statement::Pass => {
                // No-op
                Ok(TypeInfo::Unit)
            }
            Statement::Use { .. } => {
                // Module imports are handled separately
                Ok(TypeInfo::Unit)
            }
            Statement::PubUse { .. } => Ok(TypeInfo::Unit),
            Statement::Struct { .. } => {
                // Struct definitions are handled at the module level
                Ok(TypeInfo::Unit)
            }
            Statement::Enum { .. } => {
                // Enums are handled during the module pass
                Ok(TypeInfo::Unit)
            }
            Statement::TypeAlias { .. } => {
                // Type aliases are handled at the module level
                Ok(TypeInfo::Unit)
            }
            Statement::Block(block) => self.check_block(block),
        }
    }

    /// Infer the type of an expression
    pub fn infer_expr_type(&mut self, expr: &Node<Expr>) -> Result<TypeInfo> {
        let span = expr.span();
        let ty = (|| -> Result<TypeInfo> {
            match expr.as_ref() {
                Expr::Literal(lit) => Ok(match lit.as_ref() {
                    Literal::Number(num) => {
                        if num.is_float_literal {
                            TypeInfo::F64
                        } else {
                            TypeInfo::I64
                        }
                    }
                    Literal::String(_) => TypeInfo::Str,
                    Literal::Bool(_) => TypeInfo::Bool,
                    Literal::None | Literal::Unit => TypeInfo::Unit,
                }),
                Expr::Identifier(name) => {
                    if let Some(var_type) = self.context.get_variable(name) {
                        Ok(var_type.clone())
                    } else {
                        if self.registry.is_some_and(|r| r.has_module(name)) {
                            self.errors.push(
                                TypeError::new(format!("module `{}` is not imported", name))
                                    .with_hint(format!("add `use {}` at the top of the file", name))
                                    .with_span(*span),
                            );
                            return Ok(TypeInfo::Error);
                        }

                        let mut error = TypeError::new(format!("undefined variable: {}", name))
                            .with_help("Variables must be declared before use".to_string())
                            .with_span(*span);

                        // Try to find a suggestion
                        let candidates = self.context.variables.keys().cloned();
                        if let Some(closest) =
                            otterc_utils::suggest::find_best_match(name, candidates)
                        {
                            error = error
                                .with_hint(format!("did you mean `{}`?", closest))
                                .with_suggestion(closest);
                        } else {
                            error = error.with_hint(format!(
                                "did you mean to declare it with `let {}`?",
                                name
                            ));
                        }

                        self.errors.push(error);
                        Ok(TypeInfo::Error)
                    }
                }
                Expr::Binary { op, left, right } => {
                    let left_type = self.infer_expr_type(left)?;
                    let right_type = self.infer_expr_type(right)?;

                    match op {
                        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                            // Numeric operations
                            match (&left_type, &right_type) {
                                // String concatenation (must come before numeric patterns)
                                (TypeInfo::Str, TypeInfo::Str)
                                | (TypeInfo::Str, TypeInfo::I64 | TypeInfo::I32)
                                | (TypeInfo::I64 | TypeInfo::I32, TypeInfo::Str)
                                    if matches!(op, BinaryOp::Add) =>
                                {
                                    Ok(TypeInfo::Str)
                                }
                                // Numeric operations
                                (TypeInfo::F64, _) | (_, TypeInfo::F64) => Ok(TypeInfo::F64),
                                (TypeInfo::I64, _) | (_, TypeInfo::I64) => Ok(TypeInfo::I64),
                                (TypeInfo::I32, TypeInfo::I32) => Ok(TypeInfo::I32),
                                _ => {
                                    if Self::is_unknown_like(&left_type)
                                        || Self::is_unknown_like(&right_type)
                                    {
                                        Ok(Self::merge_unknown_like_types(&left_type, &right_type))
                                    } else {
                                        self.errors.push(
                                            TypeError::new(format!(
                                                "cannot apply {op:?} to {} and {}",
                                                left_type.display_name(),
                                                right_type.display_name()
                                            ))
                                            .with_span(*span),
                                        );
                                        Ok(TypeInfo::Error)
                                    }
                                }
                            }
                        }
                        BinaryOp::Eq
                        | BinaryOp::Ne
                        | BinaryOp::Lt
                        | BinaryOp::LtEq
                        | BinaryOp::Gt
                        | BinaryOp::GtEq => {
                            // Comparison operations return bool
                            if left_type.is_compatible_with(&right_type) {
                                Ok(TypeInfo::Bool)
                            } else {
                                self.errors.push(
                                    TypeError::new(format!(
                                        "cannot compare {} and {}",
                                        left_type.display_name(),
                                        right_type.display_name()
                                    ))
                                    .with_span(*span),
                                );
                                Ok(TypeInfo::Error)
                            }
                        }
                        BinaryOp::Is | BinaryOp::IsNot => {
                            // Identity comparison allows matching types or comparisons with None
                            let allow = left_type.is_compatible_with(&right_type)
                                || right_type.is_compatible_with(&left_type)
                                || matches!(left_type, TypeInfo::Unit)
                                || matches!(right_type, TypeInfo::Unit)
                                || matches!(left_type, TypeInfo::Unknown)
                                || matches!(right_type, TypeInfo::Unknown);

                            if allow {
                                Ok(TypeInfo::Bool)
                            } else {
                                self.errors.push(
                                    TypeError::new(format!(
                                        "cannot use `is` between {} and {}",
                                        left_type.display_name(),
                                        right_type.display_name()
                                    ))
                                    .with_span(*span),
                                );
                                Ok(TypeInfo::Error)
                            }
                        }
                        BinaryOp::And | BinaryOp::Or => {
                            // Logical operations require bool operands
                            if left_type.is_compatible_with(&TypeInfo::Bool)
                                && right_type.is_compatible_with(&TypeInfo::Bool)
                            {
                                Ok(TypeInfo::Bool)
                            } else {
                                self.errors.push(
                                    TypeError::new(format!(
                                        "logical operations require bool operands, got {} and {}",
                                        left_type.display_name(),
                                        right_type.display_name()
                                    ))
                                    .with_span(*span),
                                );
                                Ok(TypeInfo::Error)
                            }
                        }
                        BinaryOp::Mod => {
                            // Modulo requires integer operands
                            match (&left_type, &right_type) {
                                (TypeInfo::I32, TypeInfo::I32) => Ok(TypeInfo::I32),
                                (TypeInfo::I64, TypeInfo::I64) => Ok(TypeInfo::I64),
                                _ => {
                                    if Self::is_unknown_like(&left_type)
                                        || Self::is_unknown_like(&right_type)
                                    {
                                        Ok(TypeInfo::Unknown)
                                    } else {
                                        self.errors.push(
                                            TypeError::new(format!(
                                                "modulo requires integer operands, got {} and {}",
                                                left_type.display_name(),
                                                right_type.display_name()
                                            ))
                                            .with_span(*span),
                                        );
                                        Ok(TypeInfo::Error)
                                    }
                                }
                            }
                        }
                    }
                }
                Expr::Unary { op, expr } => {
                    let expr_type = self.infer_expr_type(expr)?;
                    match op {
                        UnaryOp::Not => {
                            if expr_type.is_compatible_with(&TypeInfo::Bool) {
                                Ok(TypeInfo::Bool)
                            } else {
                                self.errors.push(
                                    TypeError::new(format!(
                                        "not operator requires bool operand, got {}",
                                        expr_type.display_name()
                                    ))
                                    .with_span(*span),
                                );
                                Ok(TypeInfo::Error)
                            }
                        }
                        UnaryOp::Neg => {
                            if expr_type.is_compatible_with(&TypeInfo::I32)
                                || expr_type.is_compatible_with(&TypeInfo::I64)
                                || expr_type.is_compatible_with(&TypeInfo::F64)
                            {
                                Ok(expr_type)
                            } else {
                                self.errors.push(
                                    TypeError::new(format!(
                                        "negation requires numeric operand, got {}",
                                        expr_type.display_name()
                                    ))
                                    .with_span(*span),
                                );
                                Ok(TypeInfo::Error)
                            }
                        }
                    }
                }
                Expr::Call { func, args } => {
                    if let Some(enum_type) = self.try_eval_enum_constructor(func.as_ref(), args)? {
                        return Ok(enum_type);
                    }
                    let span = func.span();
                    let func_type = match func.as_ref().as_ref() {
                        Expr::Identifier(name) => {
                            if let Some(func) = self.context.get_function(name).cloned() {
                                func
                            } else {
                                self.errors.push(
                                    TypeError::new(format!("undefined function: {}", name))
                                        .with_span(*span),
                                );
                                TypeInfo::Error
                            }
                        }
                        Expr::Member { object, field } => {
                            let full_name = self.build_member_path(object, field);

                            // First check registry for exact FFI signatures
                            if let Some(registry) = self.registry {
                                if let Some(symbol) = registry.resolve(&full_name) {
                                    let params: Vec<TypeInfo> = symbol
                                        .signature
                                        .params
                                        .iter()
                                        .map(ffi_type_to_typeinfo)
                                        .collect();
                                    let return_type = if full_name == "sys.getenv" {
                                        if let Some(option_enum) = self
                                            .context
                                            .build_enum_type("Option", vec![TypeInfo::Str])
                                        {
                                            option_enum
                                        } else {
                                            ffi_type_to_typeinfo(&symbol.signature.result)
                                        }
                                    } else {
                                        ffi_type_to_typeinfo(&symbol.signature.result)
                                    };
                                    TypeInfo::Function {
                                        params,
                                        param_defaults: vec![false; symbol.signature.params.len()],
                                        return_type: Box::new(return_type),
                                    }
                                } else {
                                    self.context
                                        .get_function(&full_name)
                                        .cloned()
                                        .unwrap_or_else(|| {
                                            self.resolve_member_function(object, field, span)
                                        })
                                }
                            } else {
                                self.context
                                    .get_function(&full_name)
                                    .cloned()
                                    .unwrap_or_else(|| {
                                        self.resolve_member_function(object, field, span)
                                    })
                            }
                        }
                        _ => {
                            self.errors.push(
                                TypeError::new(
                                    "function calls must use identifier or module.function syntax"
                                        .to_string(),
                                )
                                .with_span(*span),
                            );
                            return Ok(TypeInfo::Error);
                        }
                    };

                    match func_type {
                        TypeInfo::Error => Ok(TypeInfo::Error),
                        TypeInfo::Function {
                            params,
                            param_defaults,
                            return_type,
                        } => {
                            let mut params_slice: &[TypeInfo] = &params;
                            let mut defaults_slice: &[bool] = &param_defaults;
                            let has_signature = !params.is_empty() || !param_defaults.is_empty();

                            if let Expr::Member { object, .. } = func.as_ref().as_ref()
                                && let Ok(object_type) = self.infer_expr_type(object)
                                && matches!(object_type, TypeInfo::Struct { .. })
                                && !params.is_empty()
                            {
                                let self_param = &params[0];
                                let self_matches = matches!(
                                    self_param,
                                    TypeInfo::Generic { base, args }
                                        if base == "Self" && args.is_empty()
                                ) || self_param.is_compatible_with(&object_type);

                                if !self_matches {
                                    self.errors.push(
                                        TypeError::new(format!(
                                            "method expects receiver of type {}, got {}",
                                            self_param.display_name(),
                                            object_type.display_name()
                                        ))
                                        .with_span(*span),
                                    );
                                    return Ok(TypeInfo::Error);
                                }
                                params_slice = &params[1..];
                                defaults_slice = &param_defaults[1..];

                                if let TypeInfo::Struct { name, fields } = object_type {
                                    let inferred =
                                        self.infer_struct_generics_from_instance(&name, &fields);
                                    let method_name = match func.as_ref().as_ref() {
                                        Expr::Member { field, .. } => format!("{}.{}", name, field),
                                        _ => name.clone(),
                                    };
                                    self.apply_method_specialization(&method_name, &inferred);
                                }
                            }

                            if has_signature {
                                let total_params = params_slice.len();
                                let required_params =
                                    defaults_slice.iter().filter(|flag| !**flag).count();

                                if args.len() > total_params {
                                    self.errors.push(
                                        TypeError::new(format!(
                                            "function expects at most {} arguments, got {}",
                                            total_params,
                                            args.len()
                                        ))
                                        .with_hint(
                                            "Remove extra arguments or verify the function signature"
                                                .to_string(),
                                        )
                                        .with_span(*span),
                                    );
                                    return Ok(TypeInfo::Error);
                                }

                                if args.len() < required_params {
                                    self.errors.push(
                                        TypeError::new(format!(
                                            "function expects at least {} arguments, got {}",
                                            required_params,
                                            args.len()
                                        ))
                                        .with_hint(
                                            "Provide values for all parameters without defaults"
                                                .to_string(),
                                        )
                                        .with_span(*span),
                                    );
                                    return Ok(TypeInfo::Error);
                                }

                                for (i, (arg, param_type)) in
                                    args.iter().zip(params_slice.iter()).enumerate()
                                {
                                    let arg_type = self.infer_expr_type(arg)?;
                                    if !matches!(arg_type, TypeInfo::Error)
                                        && !arg_type.is_compatible_with(param_type)
                                    {
                                        self.errors.push(
                                            TypeError::new(format!(
                                                "argument {} type mismatch: expected {}, got {}",
                                                i + 1,
                                                param_type.display_name(),
                                                arg_type.display_name()
                                            ))
                                            .with_span(*span)
                                            .with_hint(format!(
                                                "Argument {} should be of type `{}`",
                                                i + 1,
                                                param_type.display_name()
                                            ))
                                            .with_help("Check the function signature and ensure argument types match".to_string()),
                                        );
                                    }
                                }
                            } else {
                                // For unknown FFI functions, just ensure arguments are type-checked
                                for arg in args {
                                    let _ = self.infer_expr_type(arg)?;
                                }
                            }

                            let result_type =
                                if let Expr::Member { object, field } = func.as_ref().as_ref() {
                                    if let Expr::Identifier(module) = object.as_ref().as_ref() {
                                        let full_name = format!("{}.{}", module, field);
                                        if full_name == "sys.getenv" {
                                            if let Some(option_enum) = self
                                                .context
                                                .build_enum_type("Option", vec![TypeInfo::Str])
                                            {
                                                option_enum
                                            } else {
                                                *return_type
                                            }
                                        } else {
                                            *return_type
                                        }
                                    } else {
                                        *return_type
                                    }
                                } else {
                                    *return_type
                                };
                            Ok(result_type)
                        }
                        _ => {
                            self.errors.push(
                                TypeError::new(format!(
                                    "cannot call non-function type: {}",
                                    func_type.display_name()
                                ))
                                .with_span(*span)
                                .with_hint("Only functions can be called".to_string())
                                .with_help(
                                    "Check that you're using the correct function name".to_string(),
                                ),
                            );
                            Ok(TypeInfo::Error)
                        }
                    }
                }
                Expr::Range { start, end } => {
                    let start_type = self.infer_expr_type(start)?;
                    let end_type = self.infer_expr_type(end)?;

                    if start_type.is_integer() && end_type.is_integer() {
                        Ok(TypeInfo::Range(Box::new(start_type), Box::new(end_type)))
                    } else {
                        self.errors.push(
                            TypeError::new(format!(
                                "range bounds must be integers, got {} and {}",
                                start_type.display_name(),
                                end_type.display_name()
                            ))
                            .with_span(*span),
                        );
                        Ok(TypeInfo::Error)
                    }
                }
                Expr::If {
                    cond,
                    then_branch,
                    else_branch,
                } => {
                    let cond_type = self.infer_expr_type(cond)?;
                    if !cond_type.is_compatible_with(&TypeInfo::Bool) {
                        self.errors.push(
                            TypeError::new(format!(
                                "if condition must be bool, got {}",
                                cond_type.display_name()
                            ))
                            .with_span(*span),
                        );
                    }

                    let then_type = self.infer_expr_type(then_branch)?;
                    if let Some(else_expr) = else_branch {
                        let else_type = self.infer_expr_type(else_expr)?;

                        if then_type.is_compatible_with(&else_type) {
                            Ok(then_type)
                        } else {
                            self.errors.push(
                                TypeError::new(format!(
                                    "if branches must have compatible types, got {} and {}",
                                    then_type.display_name(),
                                    else_type.display_name()
                                ))
                                .with_span(*span),
                            );
                            Ok(TypeInfo::Error)
                        }
                    } else {
                        // If expression without else returns unit
                        Ok(TypeInfo::Unit)
                    }
                }
                Expr::Member { object, field } => {
                    if let Expr::Identifier(enum_name) = object.as_ref().as_ref()
                        && let Some(definition) = self.context.get_enum(enum_name)
                    {
                        if let Some(variant) = definition.variants.iter().find(|v| v.name == *field)
                        {
                            if variant.fields.is_empty()
                                && let Some(enum_type) =
                                    self.context.build_enum_type(enum_name, vec![])
                            {
                                return Ok(enum_type);
                            }

                            return Ok(TypeInfo::Function {
                                params: vec![],
                                param_defaults: vec![],
                                return_type: Box::new(TypeInfo::Unknown),
                            });
                        } else {
                            self.errors.push(
                                TypeError::new(format!(
                                    "enum '{}' has no variant '{}'",
                                    enum_name, field
                                ))
                                .with_span(*span),
                            );
                            return Ok(TypeInfo::Error);
                        }
                    }

                    let full_name = self.build_member_path(object, field);

                    // Check if this is a module path in the registry
                    if let Some(registry) = self.registry {
                        if registry
                            .all()
                            .iter()
                            .any(|f| f.name.starts_with(&format!("{}.", full_name)))
                        {
                            return Ok(TypeInfo::Module(full_name));
                        }
                        if let Some(symbol) = registry.resolve(&full_name) {
                            return Ok(TypeInfo::Function {
                                params: symbol
                                    .signature
                                    .params
                                    .iter()
                                    .map(ffi_type_to_typeinfo)
                                    .collect(),
                                param_defaults: vec![false; symbol.signature.params.len()],
                                return_type: Box::new(ffi_type_to_typeinfo(
                                    &symbol.signature.result,
                                )),
                            });
                        }
                    }

                    let object_type = self.infer_expr_type(object)?;
                    match &object_type {
                        TypeInfo::Module(module_name) => {
                            let full_name = format!("{}.{}", module_name, field);
                            if let Some(registry) = self.registry {
                                if registry
                                    .all()
                                    .iter()
                                    .any(|f| f.name.starts_with(&format!("{}.", full_name)))
                                {
                                    return Ok(TypeInfo::Module(full_name));
                                }
                                if let Some(symbol) = registry.resolve(&full_name) {
                                    return Ok(TypeInfo::Function {
                                        params: symbol
                                            .signature
                                            .params
                                            .iter()
                                            .map(ffi_type_to_typeinfo)
                                            .collect(),
                                        param_defaults: vec![false; symbol.signature.params.len()],
                                        return_type: Box::new(ffi_type_to_typeinfo(
                                            &symbol.signature.result,
                                        )),
                                    });
                                }
                            }
                            Ok(TypeInfo::Module(full_name))
                        }
                        TypeInfo::Struct { fields, .. } => {
                            if let Some(field_type) = fields.get(field) {
                                Ok(field_type.clone())
                            } else {
                                self.errors.push(
                                    TypeError::new(format!("struct has no field '{}'", field))
                                        .with_hint(
                                            "Check the struct definition for available fields"
                                                .to_string(),
                                        )
                                        .with_span(*span),
                                );
                                Ok(TypeInfo::Error)
                            }
                        }
                        TypeInfo::List(_) | TypeInfo::Dict { .. } => Ok(TypeInfo::Unknown),
                        TypeInfo::Error => {
                            // Special handling for Error type fields
                            match field.as_str() {
                                "message" | "data" => Ok(TypeInfo::Str),
                                "code" => Ok(TypeInfo::I32),
                                _ => {
                                    self.errors.push(TypeError::new(format!(
                                        "Error type has no field '{}'. Available fields: message, code, data",
                                        field
                                    )).with_span(*span));
                                    Ok(TypeInfo::Error)
                                }
                            }
                        }
                        _ => {
                            self.errors.push(
                                TypeError::new(format!(
                                    "cannot access member '{}' on type {}",
                                    field,
                                    object_type.display_name()
                                ))
                                .with_span(*span)
                                .with_hint(
                                    "Only struct types and Error support member access".to_string(),
                                ),
                            );
                            Ok(TypeInfo::Error)
                        }
                    }
                }
                Expr::FString { parts } => {
                    // F-strings always evaluate to strings
                    // Type check all embedded expressions
                    for part in parts {
                        if let FStringPart::Expr(expr) = part.as_ref() {
                            let _ = self.infer_expr_type(expr)?;
                            // We don't care about the type, just that it's valid
                        }
                    }
                    Ok(TypeInfo::Str)
                }
                // Lambda expressions removed - use anonymous fn syntax instead
                Expr::Array(elements) => {
                    if elements.is_empty() {
                        // Empty array - can't infer element type
                        Ok(TypeInfo::List(Box::new(TypeInfo::Unknown)))
                    } else {
                        // Infer common element type
                        let mut element_types: Vec<TypeInfo> = elements
                            .iter()
                            .map(|e| self.infer_expr_type(e))
                            .collect::<Result<Vec<_>>>()?;

                        // Find common type (use first element's type for now)
                        let common_type = element_types.remove(0);

                        // Check all elements are compatible
                        for (i, elem_type) in element_types.iter().enumerate() {
                            if !common_type.is_compatible_with(elem_type)
                                && !elem_type.is_compatible_with(&common_type)
                            {
                                self.errors.push(
                                    TypeError::new(format!(
                                        "array element {} has incompatible type: expected {}, got {}",
                                        i + 1,
                                        common_type.display_name(),
                                        elem_type.display_name()
                                    ))
                                    .with_span(*span)
                                    .with_hint(
                                        "All array elements must have compatible types".to_string(),
                                    ),
                                );
                            }
                        }

                        Ok(TypeInfo::List(Box::new(common_type)))
                    }
                }
                Expr::Dict(entries) => {
                    if entries.is_empty() {
                        // Empty dictionary - can't infer key/value types
                        Ok(TypeInfo::Dict {
                            key: Box::new(TypeInfo::Unknown),
                            value: Box::new(TypeInfo::Unknown),
                        })
                    } else {
                        // Infer common key and value types
                        let mut key_types = Vec::new();
                        let mut value_types = Vec::new();

                        for (key_expr, value_expr) in entries {
                            key_types.push(self.infer_expr_type(key_expr)?);
                            value_types.push(self.infer_expr_type(value_expr)?);
                        }

                        // Find common types
                        let common_key_type = key_types.remove(0);
                        let common_value_type = value_types.remove(0);

                        // Check all keys and values are compatible
                        for (i, key_type) in key_types.iter().enumerate() {
                            if !common_key_type.is_compatible_with(key_type) {
                                self.errors.push(
                                    TypeError::new(format!(
                                        "dictionary key {} has incompatible type: expected {}, got {}",
                                        i + 1,
                                        common_key_type.display_name(),
                                        key_type.display_name()
                                    ))
                                    .with_span(*span)
                                    .with_hint(
                                        "All dictionary keys must have compatible types"
                                            .to_string(),
                                    ),
                                );
                            }
                        }

                        for (i, value_type) in value_types.iter().enumerate() {
                            if !common_value_type.is_compatible_with(value_type) {
                                self.errors.push(
                                    TypeError::new(format!(
                                        "dictionary value {} has incompatible type: expected {}, got {}",
                                        i + 1,
                                        common_value_type.display_name(),
                                        value_type.display_name()
                                    ))
                                    .with_span(*span)
                                    .with_hint(
                                        "All dictionary values must have compatible types"
                                            .to_string(),
                                    ),
                                );
                            }
                        }

                        let key_type = if common_key_type.is_compatible_with(&TypeInfo::Str) {
                            TypeInfo::Str
                        } else {
                            self.errors.push(
                                TypeError::new(format!(
                                    "dictionary keys must be str, got {}",
                                    common_key_type.display_name()
                                ))
                                .with_span(*span)
                                .with_hint(
                                    "Only string keys are supported for dictionaries currently"
                                        .to_string(),
                                ),
                            );
                            TypeInfo::Str
                        };

                        Ok(TypeInfo::Dict {
                            key: Box::new(key_type),
                            value: Box::new(common_value_type),
                        })
                    }
                }
                Expr::ListComprehension {
                    element,
                    var,
                    iterable,
                    condition,
                } => {
                    let iterable_type = self.infer_expr_type(iterable)?;
                    let element_iter_type = if let TypeInfo::List(elem) = &iterable_type {
                        elem.as_ref().clone()
                    } else {
                        self.errors.push(
                            TypeError::new(format!(
                                "list comprehension expects list iterable, got {}",
                                iterable_type.display_name()
                            ))
                            .with_span(*span),
                        );
                        TypeInfo::Unknown
                    };

                    let previous = self.context.remove_variable(var);
                    self.context
                        .insert_variable(var.clone(), element_iter_type.clone());

                    if let Some(cond_expr) = condition {
                        let cond_type = self.infer_expr_type(cond_expr)?;
                        if !cond_type.is_compatible_with(&TypeInfo::Bool) {
                            self.errors.push(
                                TypeError::new(format!(
                                    "list comprehension condition must be bool, got {}",
                                    cond_type.display_name()
                                ))
                                .with_span(*span),
                            );
                        }
                    }

                    self.comprehension_var_types
                        .insert(*span, element_iter_type.clone());

                    let element_type = self.infer_expr_type(element)?;

                    match previous {
                        Some(prev) => {
                            self.context.insert_variable(var.clone(), prev);
                        }
                        None => {
                            self.context.remove_variable(var);
                        }
                    }

                    Ok(TypeInfo::List(Box::new(element_type)))
                }
                Expr::DictComprehension {
                    key,
                    value,
                    var,
                    iterable,
                    condition,
                } => {
                    let iterable_type = self.infer_expr_type(iterable)?;
                    let element_iter_type = if let TypeInfo::List(elem) = &iterable_type {
                        elem.as_ref().clone()
                    } else {
                        self.errors.push(
                            TypeError::new(format!(
                                "dict comprehension expects list iterable, got {}",
                                iterable_type.display_name()
                            ))
                            .with_span(*span),
                        );
                        TypeInfo::Unknown
                    };

                    let previous = self.context.remove_variable(var);
                    self.context
                        .insert_variable(var.clone(), element_iter_type.clone());

                    if let Some(cond_expr) = condition {
                        let cond_type = self.infer_expr_type(cond_expr)?;
                        if !cond_type.is_compatible_with(&TypeInfo::Bool) {
                            self.errors.push(
                                TypeError::new(format!(
                                    "dict comprehension condition must be bool, got {}",
                                    cond_type.display_name()
                                ))
                                .with_span(*span),
                            );
                        }
                    }

                    self.comprehension_var_types
                        .insert(*span, element_iter_type.clone());

                    let key_type = self.infer_expr_type(key)?;
                    if !key_type.is_compatible_with(&TypeInfo::Str) {
                        self.errors.push(
                            TypeError::new(format!(
                                "dict comprehension key must be string, got {}",
                                key_type.display_name()
                            ))
                            .with_span(*span),
                        );
                    }

                    let value_type = self.infer_expr_type(value)?;

                    match previous {
                        Some(prev) => {
                            self.context.insert_variable(var.clone(), prev);
                        }
                        None => {
                            self.context.remove_variable(var);
                        }
                    }

                    Ok(TypeInfo::Dict {
                        key: Box::new(TypeInfo::Str),
                        value: Box::new(value_type),
                    })
                }
                Expr::Match { value, arms } => {
                    let mut value_type = self.infer_expr_type(value)?;
                    // Normalize generic types to enum types if applicable
                    value_type = self.context.normalize_type(value_type);

                    if arms.is_empty() {
                        self.errors.push(
                            TypeError::new(
                                "match expression must have at least one arm".to_string(),
                            )
                            .with_span(*span)
                            .with_hint("Add at least one pattern to match against".to_string()),
                        );
                        return Ok(TypeInfo::Error);
                    }

                    // Type check each arm
                    let mut arm_types = Vec::new();
                    for arm in arms {
                        // Bind pattern variables before checking guard and body
                        // Ensure type is normalized before binding (in case it wasn't normalized above)
                        let mut normalized_type = self.context.normalize_type(value_type.clone());

                        // If normalization didn't work and we have an enum variant pattern, try to build the enum type directly
                        if let TypeInfo::Generic { base, args } = &normalized_type
                            && let Pattern::EnumVariant { enum_name, .. } =
                                &arm.as_ref().pattern.as_ref()
                            && base == enum_name
                        {
                            // Try to build the enum type directly using the pattern's enum name
                            if let Some(built_enum) =
                                self.context.build_enum_type(enum_name, args.clone())
                            {
                                normalized_type = built_enum;
                            }
                        }

                        // Check pattern matches value type with sophisticated validation
                        self.validate_pattern_against_type(&arm.as_ref().pattern, &normalized_type);

                        let old_vars = self.context.variables.clone();
                        self.bind_pattern_variables(&arm.as_ref().pattern, &normalized_type);

                        // Check guard if present
                        if let Some(guard) = &arm.as_ref().guard {
                            let guard_type = self.infer_expr_type(guard)?;
                            if !guard_type.is_compatible_with(&TypeInfo::Bool) {
                                self.errors.push(
                                    TypeError::new(format!(
                                        "match guard must be bool, got {}",
                                        guard_type.display_name()
                                    ))
                                    .with_span(*span),
                                );
                            }
                        }

                        // Check body (now a block)
                        let arm_return_type = self.check_block(&arm.as_ref().body)?;
                        arm_types.push(arm_return_type);

                        // Restore original variables (pattern bindings don't leak)
                        self.context.variables = old_vars;
                    }

                    // All arms must return compatible types
                    let common_type = arm_types.remove(0);
                    for (i, arm_type) in arm_types.iter().enumerate() {
                        if !common_type.is_compatible_with(arm_type) {
                            self.errors.push(
                                TypeError::new(format!(
                                    "match arm {} returns incompatible type: expected {}, got {}",
                                    i + 1,
                                    common_type.display_name(),
                                    arm_type.display_name()
                                ))
                                .with_span(*span)
                                .with_hint(
                                    "All match arms must return compatible types".to_string(),
                                ),
                            );
                        }
                    }

                    Ok(common_type)
                }
                Expr::Struct { name, fields } => {
                    // Get struct definition (clone to avoid borrow checker issues)
                    let struct_def = match self.context.get_struct(name) {
                        Some(def) => def.clone(),
                        None => {
                            self.errors.push(
                                TypeError::new(format!("unknown struct type: {}", name))
                                    .with_hint(
                                        "Check that the struct is defined before use".to_string(),
                                    )
                                    .with_span(*span),
                            );
                            return Ok(TypeInfo::Error);
                        }
                    };

                    // Check that all provided fields exist and have correct types
                    let mut provided_fields = std::collections::HashSet::new();
                    let mut inferred_generics = HashMap::new();

                    for (field_name, field_expr) in fields {
                        if provided_fields.contains(field_name) {
                            self.errors.push(
                                TypeError::new(format!(
                                    "duplicate field '{}' in struct initialization",
                                    field_name
                                ))
                                .with_span(*span),
                            );
                        }
                        provided_fields.insert(field_name.clone());

                        let Some(field_type) = struct_def.fields.get(field_name) else {
                            self.errors.push(
                                TypeError::new(format!(
                                    "struct '{}' has no field '{}'",
                                    name, field_name
                                ))
                                .with_span(*span)
                                .with_hint(format!(
                                    "Available fields: {}",
                                    struct_def
                                        .fields
                                        .keys()
                                        .cloned()
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                )),
                            );
                            continue;
                        };

                        let expr_type = self.infer_expr_type(field_expr)?;

                        // Infer generics from field type
                        self.infer_generics_from_type_info(
                            field_type,
                            &expr_type,
                            &struct_def.generics,
                            &mut inferred_generics,
                        );

                        // Substitute inferred generics into field type for checking
                        let concrete_field_type = field_type.substitute(&inferred_generics);

                        if !expr_type.is_compatible_with(&concrete_field_type) {
                            self.errors.push(
                                TypeError::new(format!(
                                    "field '{}' of struct '{}' expects type {}, got {}",
                                    field_name,
                                    name,
                                    concrete_field_type.display_name(),
                                    expr_type.display_name()
                                ))
                                .with_span(*span),
                            );
                        }
                    }

                    // Check that all required fields are provided
                    for field_name in struct_def.fields.keys() {
                        if !provided_fields.contains(field_name) {
                            self.errors.push(
                                TypeError::new(format!(
                                    "missing required field '{}' in struct '{}' initialization",
                                    field_name, name
                                ))
                                .with_span(*span),
                            );
                        }
                    }

                    // Construct the concrete struct type with substituted fields
                    let concrete_fields = struct_def
                        .fields
                        .iter()
                        .map(|(k, v)| (k.clone(), v.substitute(&inferred_generics)))
                        .collect();

                    Ok(TypeInfo::Struct {
                        name: name.clone(),
                        fields: concrete_fields,
                    })
                }
                Expr::Await(expr) => {
                    let inner_type = self.infer_expr_type(expr)?;

                    let (is_task_handle, payload_type) = match &inner_type {
                        TypeInfo::Generic { base, args }
                            if (base == "Task" || base == "Future") && !args.is_empty() =>
                        {
                            (true, args[0].clone())
                        }
                        TypeInfo::Generic { base, .. } if base == "Task" || base == "Future" => {
                            (true, TypeInfo::Unknown)
                        }
                        _ => (false, TypeInfo::Unit),
                    };

                    if !is_task_handle && !matches!(inner_type, TypeInfo::Unknown | TypeInfo::Error)
                    {
                        self.errors.push(
                            TypeError::new(format!(
                                "await expects a Task handle, got {}",
                                inner_type.display_name()
                            ))
                            .with_span(*expr.span()),
                        );
                    }

                    Ok(payload_type)
                }
                Expr::Spawn(expr) => {
                    // Spawn creates a task from an expression
                    // Type check the inner expression
                    let inner_type = self.infer_expr_type(expr)?;

                    Ok(TypeInfo::Generic {
                        base: "Task".to_string(),
                        args: vec![inner_type],
                    })
                }
            }
        })()?;

        self.record_expr_type(expr, &ty);
        Ok(ty)
    }

    /// Get collected errors
    pub fn errors(&self) -> &[TypeError] {
        &self.errors
    }

    pub fn expr_type_map(&self) -> &HashMap<usize, TypeInfo> {
        &self.expr_types
    }

    pub fn into_type_maps(
        self,
    ) -> (
        HashMap<usize, TypeInfo>,
        HashMap<Span, TypeInfo>,
        HashMap<Span, TypeInfo>,
    ) {
        (
            self.expr_types,
            self.expr_types_by_span,
            self.comprehension_var_types,
        )
    }

    pub fn enum_layouts(&self) -> HashMap<String, EnumLayout> {
        self.context.enum_layouts()
    }

    pub fn collect_public_exports(&self, module_name: &str, program: &Program) -> ModuleExports {
        let mut exports = ModuleExports::new(module_name.to_string());
        for statement in &program.statements {
            match statement.as_ref() {
                Statement::Function(function) if function.as_ref().public => {
                    if let Some(sig) = self.context.functions.get(&function.as_ref().name).cloned()
                    {
                        exports
                            .functions
                            .insert(function.as_ref().name.clone(), sig);
                    }
                }
                Statement::Struct { name, public, .. } if *public => {
                    if let Some(def) = self.context.get_struct(name).cloned() {
                        exports.structs.insert(name.clone(), def);
                    }
                }
                Statement::Enum { name, public, .. } if *public => {
                    if let Some(def) = self.context.get_enum(name).cloned() {
                        exports.enums.insert(name.clone(), def);
                    }
                }
                Statement::TypeAlias { name, public, .. } if *public => {
                    if let Some(alias) = self.context.resolve_type_alias(name).cloned() {
                        exports.type_aliases.insert(name.clone(), alias);
                    }
                }
                Statement::Let { name, public, .. } if *public => {
                    if let Some(var_type) = self.context.get_variable(name.as_ref()).cloned() {
                        exports.variables.insert(name.as_ref().clone(), var_type);
                    }
                }
                _ => {}
            }
        }
        exports
    }

    pub fn import_module_exports(&mut self, alias: &str, exports: &ModuleExports) {
        if exports.is_empty() {
            return;
        }

        self.context
            .insert_variable(alias.to_string(), TypeInfo::Module(exports.module.clone()));

        for (name, ty) in &exports.functions {
            let qualified = format!("{}.{}", exports.module, name);
            self.context.insert_function(qualified, ty.clone());
        }

        for (name, ty) in &exports.variables {
            let qualified = format!("{}.{}", exports.module, name);
            self.context.insert_variable(qualified, ty.clone());
        }

        for def in exports.structs.values() {
            self.context.define_struct(def.clone());
        }

        for def in exports.enums.values() {
            self.context.define_enum(def.clone());
        }

        for (name, ty) in &exports.type_aliases {
            self.context.type_aliases.insert(name.clone(), ty.clone());
        }
    }

    fn build_member_path(&self, object: &Node<Expr>, field: &str) -> String {
        match object.as_ref() {
            Expr::Identifier(name) => {
                let canonical = self
                    .context
                    .get_variable(name)
                    .and_then(|ty| match ty {
                        TypeInfo::Module(module_name) => Some(module_name.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| name.clone());
                format!("{}.{}", canonical, field)
            }
            Expr::Member {
                object: inner_obj,
                field: inner_field,
            } => {
                let prefix = self.build_member_path(inner_obj, inner_field);
                format!("{}.{}", prefix, field)
            }
            _ => format!("unknown.{}", field),
        }
    }

    fn resolve_member_function(
        &mut self,
        object: &Node<Expr>,
        field: &str,
        span: &Span,
    ) -> TypeInfo {
        match self.infer_expr_type(object) {
            Ok(TypeInfo::Struct { name, .. }) => {
                let method_name = format!("{}.{}", name, field);
                self.context
                    .get_function(&method_name)
                    .cloned()
                    .unwrap_or_else(|| {
                        self.errors.push(
                            TypeError::new(format!("struct '{}' has no method '{}'", name, field))
                                .with_span(*span),
                        );
                        TypeInfo::Error
                    })
            }
            _ => TypeInfo::Function {
                params: vec![],
                param_defaults: vec![],
                return_type: Box::new(TypeInfo::Unknown),
            },
        }
    }
}

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}

fn ffi_type_to_typeinfo(ft: &FfiType) -> TypeInfo {
    match ft {
        FfiType::Unit => TypeInfo::Unit,
        FfiType::Bool => TypeInfo::Bool,
        FfiType::I32 => TypeInfo::I32,
        // Opaque handles are i64 at type level
        FfiType::I64 | FfiType::Opaque => TypeInfo::I64,
        FfiType::F64 => TypeInfo::F64,
        FfiType::Str => TypeInfo::Str,
        FfiType::List => TypeInfo::List(Box::new(TypeInfo::Unknown)),
        FfiType::Map => TypeInfo::Dict {
            key: Box::new(TypeInfo::Unknown),
            value: Box::new(TypeInfo::Unknown),
        },
        FfiType::Struct { .. } | FfiType::Tuple(_) => TypeInfo::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otterc_ast::nodes::{BinaryOp, Expr, Literal, Node, NumberLiteral};
    use otterc_span::Span;
    use std::f64;

    #[test]
    fn test_type_inference_literal() {
        let mut checker = TypeChecker::new();

        let expr = Node::new(
            Expr::Literal(Node::new(
                Literal::Number(NumberLiteral::new(42.0, false)),
                Span::new(0, 0),
            )),
            Span::new(0, 0),
        );
        let ty = checker.infer_expr_type(&expr).unwrap();
        assert_eq!(ty, TypeInfo::I64);

        let expr = Node::new(
            Expr::Literal(Node::new(
                Literal::Number(NumberLiteral::new(f64::consts::PI, true)),
                Span::new(0, 0),
            )),
            Span::new(0, 0),
        );
        let ty = checker.infer_expr_type(&expr).unwrap();
        assert_eq!(ty, TypeInfo::F64);
    }

    #[test]
    fn test_type_inference_binary() {
        let mut checker = TypeChecker::new();

        let expr = Node::new(
            Expr::Binary {
                op: BinaryOp::Add,
                left: Box::new(Node::new(
                    Expr::Literal(Node::new(
                        Literal::Number(NumberLiteral::new(1.0, true)),
                        Span::new(0, 0),
                    )),
                    Span::new(0, 0),
                )),
                right: Box::new(Node::new(
                    Expr::Literal(Node::new(
                        Literal::Number(NumberLiteral::new(2.0, true)),
                        Span::new(0, 0),
                    )),
                    Span::new(0, 0),
                )),
            },
            Span::new(0, 0),
        );
        let ty = checker.infer_expr_type(&expr).unwrap();
        assert_eq!(ty, TypeInfo::F64);
    }
}
