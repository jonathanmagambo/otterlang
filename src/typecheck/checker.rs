use anyhow::{bail, Result};
use std::collections::HashMap;

use crate::runtime::symbol_registry::{FfiType, SymbolRegistry};
use crate::typecheck::types::{EnumDefinition, TypeContext, TypeError, TypeInfo};
use ast::nodes::{Block, Expr, Function, Literal, Program, Statement};
use language::LanguageFeatureFlags;

/// Type checker that validates and infers types in OtterLang programs
pub struct TypeChecker {
    errors: Vec<TypeError>,
    context: TypeContext,
    registry: Option<&'static SymbolRegistry>,
    expr_types: HashMap<usize, TypeInfo>,
    features: LanguageFeatureFlags,
    /// Current function's return type (if inside a function)
    current_function_return_type: Option<TypeInfo>,
}

impl TypeChecker {
    fn record_expr_type(&mut self, expr: &Expr, ty: &TypeInfo) {
        let id = expr as *const Expr as usize;
        self.expr_types.insert(id, ty.clone());
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

        Self {
            errors: Vec::new(),
            context,
            registry: None,
            expr_types: HashMap::new(),
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

        context.functions.insert(
            "str".to_string(),
            TypeInfo::Function {
                params: vec![TypeInfo::Unknown],
                param_defaults: vec![false],
                return_type: Box::new(TypeInfo::Str),
            },
        );

        // len functions
        context.functions.insert(
            "len".to_string(),
            TypeInfo::Function {
                params: vec![TypeInfo::Str],
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

        // Common math functions (from math module)
        for func_name in &[
            "abs", "sqrt", "pow", "exp", "log", "sin", "cos", "tan", "floor", "ceil", "round",
        ] {
            context.functions.insert(
                func_name.to_string(),
                TypeInfo::Function {
                    params: vec![TypeInfo::F64],
                    param_defaults: vec![false],
                    return_type: Box::new(TypeInfo::F64),
                },
            );
        }

        // min/max with two parameters
        for func_name in &["min", "max"] {
            context.functions.insert(
                func_name.to_string(),
                TypeInfo::Function {
                    params: vec![TypeInfo::F64, TypeInfo::F64],
                    param_defaults: vec![false, false],
                    return_type: Box::new(TypeInfo::F64),
                },
            );
        }
    }

    /// Type check a program
    pub fn check_program(&mut self, program: &Program) -> Result<()> {
        // First pass: collect struct definitions, enums, and type aliases
        self.register_type_definitions(&program.statements);

        // Second pass: collect function signatures
        for statement in &program.statements {
            if let Statement::Function(function) = statement {
                let sig = self.infer_function_signature(function);
                self.context.functions.insert(function.name.clone(), sig);
            }
        }

        // Third pass: type check function bodies and top-level statements
        for statement in &program.statements {
            match statement {
                Statement::Function(function) => {
                    self.check_function(function)?;
                }
                Statement::Let { .. } | Statement::Expr(_) => {
                    // Top-level let and expressions are allowed
                    self.check_statement(statement)?;
                }
                Statement::Struct { .. }
                | Statement::Enum { .. }
                | Statement::TypeAlias { .. }
                | Statement::Use { .. } => {
                    // These are handled in earlier passes
                }
                _ => {
                    self.errors.push(TypeError::new(format!(
                        "unexpected statement at top level: {:?}",
                        statement
                    )).with_hint("Only function definitions, let statements, and expressions are allowed at the top level".to_string()));
                }
            }
        }

        if !self.errors.is_empty() {
            let error_messages: Vec<String> = self.errors.iter().map(|e| e.to_string()).collect();
            bail!("type checking failed:\n{}", error_messages.join("\n\n"));
        }

        Ok(())
    }

    /// Infer function signature from declaration
    fn infer_function_signature(&mut self, function: &Function) -> TypeInfo {
        let mut param_types = Vec::new();
        let mut param_defaults = Vec::new();
        let mut seen_default = false;

        for param in &function.params {
            let explicit_type = param
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

            if let Some(default_expr) = &param.default {
                seen_default = true;
                param_defaults.push(true);
                if let Ok(default_type) = self.infer_expr_type(default_expr) {
                    if let Some(expected) = &explicit_type {
                        if !default_type.is_compatible_with(expected) {
                            self.errors.push(
                                TypeError::new(format!(
                                    "default value for parameter `{}` has type {}, expected {}",
                                    param.name,
                                    default_type.display_name(),
                                    expected.display_name()
                                ))
                                .with_hint(
                                    "Ensure the default expression matches the declared parameter type"
                                        .to_string(),
                                ),
                            );
                        }
                    }
                }
            } else {
                if seen_default {
                    self.errors.push(
                        TypeError::new(format!(
                            "parameter `{}` without default cannot follow parameters with defaults",
                            param.name
                        ))
                        .with_hint("Move parameters without defaults before parameters that specify defaults".to_string()),
                    );
                }
                param_defaults.push(false);
            }
        }

        let return_type = if let Some(ty) = &function.ret_ty {
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

    fn register_type_definitions(&mut self, statements: &[Statement]) {
        for statement in statements {
            match statement {
                Statement::Struct {
                    name,
                    fields,
                    methods,
                    generics,
                    ..
                } => {
                    let mut field_types = HashMap::new();
                    for (field_name, field_ty) in fields {
                        let ty = self.context.type_from_annotation(field_ty);
                        field_types.insert(field_name.clone(), ty);
                    }

                    for generic in generics {
                        self.context.push_generic(generic.clone());
                    }

                    self.context.define_struct(name.clone(), field_types);

                    for method in methods {
                        let method_name = format!("{}.{}", name, method.name);
                        let sig = self.infer_function_signature(method);
                        self.context.insert_function(method_name.clone(), sig);
                    }

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
                    ..
                } => {
                    let definition = EnumDefinition {
                        name: name.clone(),
                        generics: generics.clone(),
                        variants: variants.clone(),
                    };
                    self.context.define_enum(definition);
                }
                _ => {}
            }
        }
    }

    /// Type check a function
    fn check_function(&mut self, function: &Function) -> Result<()> {
        // Determine function return type
        let return_type = if let Some(ret_ty) = &function.ret_ty {
            self.context.type_from_annotation(ret_ty)
        } else {
            TypeInfo::Unit
        };

        let mut fn_context = TypeContext::with_features(self.features.clone());

        // Add function parameters to context
        for param in &function.params {
            let param_type = if let Some(ty) = &param.ty {
                self.context.type_from_annotation(ty)
            } else {
                TypeInfo::Unknown
            };
            fn_context.insert_variable(param.name.clone(), param_type);
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
        let old_return_type = std::mem::replace(&mut self.current_function_return_type, Some(return_type));
        let result = self.check_block(&function.body);
        self.context = old_context;
        self.current_function_return_type = old_return_type;

        result
    }

    /// Check function with generic type parameters
    /// This handles functions that have generic type parameters in their signature
    #[allow(dead_code)]
    fn check_function_with_generics(&mut self, function: &Function) -> Result<()> {
        // Extract generic type parameters from function signature
        let mut generic_params = Vec::new();

        // Check parameter types for generic parameters
        for param in &function.params {
            if let Some(ty) = &param.ty {
                self.extract_generic_params(ty, &mut generic_params);
            }
        }

        if let Some(ret_ty) = &function.ret_ty {
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
    #[allow(dead_code)]
    fn extract_generic_params(&self, ty: &ast::nodes::Type, params: &mut Vec<String>) {
        match ty {
            ast::nodes::Type::Simple(name) => {
                // Check if this looks like a generic parameter (single uppercase letter or common generic names)
                // In OtterLang, generic parameters are typically single uppercase letters (T, U, etc.)
                if name.len() == 1 && name.chars().next().unwrap().is_uppercase() {
                    if !params.contains(name) {
                        params.push(name.clone());
                    }
                }
            }
            ast::nodes::Type::Generic { base, args } => {
                // Check if base is a generic parameter
                if base.len() == 1 && base.chars().next().unwrap().is_uppercase() {
                    if !params.contains(base) {
                        params.push(base.clone());
                    }
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
        func: &Expr,
        args: &[Expr],
    ) -> Result<Option<TypeInfo>> {
        if let Expr::Member { object, field } = func {
            if let Expr::Identifier(enum_name) = object.as_ref() {
                if let Some(definition) = self.context.get_enum(enum_name).cloned() {
                    let variant = match definition
                        .variants
                        .iter()
                        .find(|variant| variant.name == *field)
                    {
                        Some(v) => v,
                        None => {
                            self.errors.push(TypeError::new(format!(
                                "enum '{}' has no variant '{}'",
                                enum_name, field
                            )));
                            return Ok(Some(TypeInfo::Error));
                        }
                    };

                    let expected_len = variant.fields.len();
                    if expected_len != args.len() {
                        self.errors.push(TypeError::new(format!(
                            "enum variant '{}.{}' expects {} argument(s), got {}",
                            enum_name,
                            field,
                            expected_len,
                            args.len()
                        )));
                    }

                    let mut arg_types = Vec::new();
                    for arg in args {
                        arg_types.push(self.infer_expr_type(arg)?);
                    }

                    for (field_ty, actual_ty) in variant.fields.iter().zip(arg_types.iter()) {
                        let expected_type = self.context.type_from_annotation(field_ty);
                        if !self.type_contains_enum_generic(field_ty, &definition.generics)
                            && !actual_ty.is_compatible_with(&expected_type)
                        {
                            self.errors.push(TypeError::new(format!(
                                "argument for '{}.{}' expects type {}, got {}",
                                enum_name,
                                field,
                                expected_type.display_name(),
                                actual_ty.display_name()
                            )));
                        }
                    }

                    let mut inferred = HashMap::new();
                    for (field_ty, actual_ty) in variant.fields.iter().zip(arg_types.iter()) {
                        self.infer_enum_generics_from_type(
                            field_ty,
                            actual_ty,
                            &definition,
                            &mut inferred,
                        );
                    }

                    let resolved_args = definition
                        .generics
                        .iter()
                        .map(|name| inferred.get(name).cloned().unwrap_or(TypeInfo::Unknown))
                        .collect::<Vec<_>>();

                    if let Some(enum_type) = self.context.build_enum_type(enum_name, resolved_args)
                    {
                        return Ok(Some(enum_type));
                    }

                    return Ok(Some(TypeInfo::Error));
                }
            }
        }
        Ok(None)
    }

    fn type_contains_enum_generic(&self, ty: &ast::nodes::Type, generics: &[String]) -> bool {
        match ty {
            ast::nodes::Type::Simple(name) => generics.contains(name),
            ast::nodes::Type::Generic { base, args } => {
                generics.contains(base)
                    || args
                        .iter()
                        .any(|arg| self.type_contains_enum_generic(arg, generics))
            }
        }
    }

    fn infer_enum_generics_from_type(
        &mut self,
        expected: &ast::nodes::Type,
        actual: &TypeInfo,
        definition: &EnumDefinition,
        inferred: &mut HashMap<String, TypeInfo>,
    ) {
        match expected {
            ast::nodes::Type::Simple(name) => {
                if definition.generics.contains(name) {
                    inferred
                        .entry(name.clone())
                        .or_insert_with(|| actual.clone());
                }
            }
            ast::nodes::Type::Generic { base, args } => {
                if definition.generics.contains(base) && args.is_empty() {
                    inferred
                        .entry(base.clone())
                        .or_insert_with(|| actual.clone());
                } else if base.eq_ignore_ascii_case("List") {
                    if let TypeInfo::List(inner) = actual {
                        if let Some(sub_ty) = args.get(0) {
                            self.infer_enum_generics_from_type(sub_ty, inner, definition, inferred);
                        }
                    }
                } else if base.eq_ignore_ascii_case("Dict") {
                    if let TypeInfo::Dict { key, value } = actual {
                        if let Some(key_ty) = args.get(0) {
                            self.infer_enum_generics_from_type(key_ty, key, definition, inferred);
                        }
                        if let Some(val_ty) = args.get(1) {
                            self.infer_enum_generics_from_type(val_ty, value, definition, inferred);
                        }
                    }
                } else {
                    match actual {
                        TypeInfo::Generic {
                            base: actual_base,
                            args: actual_args,
                        } if actual_base == base => {
                            for (expected_arg, actual_arg) in args.iter().zip(actual_args.iter()) {
                                self.infer_enum_generics_from_type(
                                    expected_arg,
                                    actual_arg,
                                    definition,
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
                                self.infer_enum_generics_from_type(
                                    expected_arg,
                                    actual_arg,
                                    definition,
                                    inferred,
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Bind variables from a pattern into the type checking context
    fn bind_pattern_variables(&mut self, pattern: &ast::nodes::Pattern, ty: &TypeInfo) {
        match pattern {
            ast::nodes::Pattern::Identifier(name) => {
                // Simple identifier pattern binds the whole value
                self.context.insert_variable(name.clone(), ty.clone());
            }
            ast::nodes::Pattern::EnumVariant {
                enum_name,
                variant,
                fields,
            } => {
                // Get variant info from the concrete enum type (with generics already substituted)
                // Strategy: Try to build the enum type directly from the pattern's enum name
                // This works even if the type is still Generic after normalization
                let mut bound = false;
                
                // First, check if the type is already an Enum
                if let TypeInfo::Enum { variants, .. } = &ty {
                    if let Some(variant_info) = variants.get(variant) {
                        if fields.len() == variant_info.fields.len() {
                            for (field_pattern, field_type) in fields.iter().zip(variant_info.fields.iter()) {
                                self.bind_pattern_variables(field_pattern, field_type);
                            }
                            bound = true;
                        }
                    }
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
                    
                    if let Some(args) = args {
                        if let Some(built_enum) = self.context.build_enum_type(enum_name, args) {
                            if let TypeInfo::Enum { variants, .. } = &built_enum {
                                if let Some(variant_info) = variants.get(variant) {
                                    if fields.len() == variant_info.fields.len() {
                                        for (field_pattern, field_type) in fields.iter().zip(variant_info.fields.iter()) {
                                            self.bind_pattern_variables(field_pattern, field_type);
                                        }
                                        bound = true;
                                    }
                                }
                            }
                        }
                    }
                }
                
                // If still not bound, try normalization as last resort
                if !bound {
                    let normalized = self.context.normalize_type(ty.clone());
                    if let TypeInfo::Enum { variants, .. } = &normalized {
                        if let Some(variant_info) = variants.get(variant) {
                            if fields.len() == variant_info.fields.len() {
                                for (field_pattern, field_type) in fields.iter().zip(variant_info.fields.iter()) {
                                    self.bind_pattern_variables(field_pattern, field_type);
                                }
                            }
                        }
                    }
                }
            }
            ast::nodes::Pattern::Struct { name, fields } => {
                // Clone struct fields to avoid borrow conflicts
                let struct_fields = self.context.get_struct(name).cloned();
                if let Some(struct_fields) = struct_fields {
                    for (field_name, nested_pattern) in fields {
                        if let Some(field_type) = struct_fields.get(field_name) {
                            if let Some(pattern) = nested_pattern {
                                self.bind_pattern_variables(pattern, field_type);
                            } else {
                                // No nested pattern, bind the field name directly
                                self.context.insert_variable(field_name.clone(), field_type.clone());
                            }
                        }
                    }
                }
            }
            ast::nodes::Pattern::Array { patterns, rest } => {
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
            ast::nodes::Pattern::Wildcard | ast::nodes::Pattern::Literal(_) => {
                // No variables to bind
            }
        }
    }

    fn validate_pattern_against_type(&mut self, pattern: &ast::nodes::Pattern, ty: &TypeInfo) {
        match pattern {
            ast::nodes::Pattern::Wildcard => {
                // Wildcard matches any type
            }
            ast::nodes::Pattern::Identifier(_) => {
                // Identifier binds any type
            }
            ast::nodes::Pattern::Literal(lit) => {
                // Check literal type matches expected type
                let lit_type = match lit {
                    ast::nodes::Literal::String(_) => TypeInfo::Str,
                    ast::nodes::Literal::Number(n) => {
                        if n.value.fract() == 0.0 && n.value >= i32::MIN as f64 && n.value <= i32::MAX as f64 {
                            TypeInfo::I32
                        } else {
                            TypeInfo::F64
                        }
                    }
                    ast::nodes::Literal::Bool(_) => TypeInfo::Bool,
                    ast::nodes::Literal::None | ast::nodes::Literal::Unit => TypeInfo::Unit,
                };

                if !lit_type.is_compatible_with(ty) {
                    self.errors.push(TypeError::new(format!(
                        "literal pattern type {} does not match expected type {}",
                        lit_type.display_name(),
                        ty.display_name()
                    )));
                }
            }
            ast::nodes::Pattern::EnumVariant {
                enum_name,
                variant,
                fields,
            } => {
                // Check that the value type is an enum and matches the pattern
                match ty {
                    TypeInfo::Enum { name, args: _, variants } => {
                        if name != enum_name {
                            self.errors.push(TypeError::new(format!(
                                "enum pattern '{}' does not match value type {}",
                                enum_name, name
                            )));
                            return;
                        }

                        // Find the variant in the enum definition
                        if let Some(variant_def) = variants.get(variant) {
                            if variant_def.fields.len() != fields.len() {
                                self.errors.push(TypeError::new(format!(
                                    "enum variant '{}.{}' has {} field(s), but pattern destructures {}",
                                    enum_name, variant, variant_def.fields.len(), fields.len()
                                )));
                            } else {
                                // Check that nested patterns match field types
                                for (field_pattern, field_type) in fields.iter().zip(variant_def.fields.iter()) {
                                    self.validate_pattern_against_type(field_pattern, field_type);
                                }
                            }
                        } else {
                            self.errors.push(TypeError::new(format!(
                                "enum '{}' has no variant '{}'",
                                enum_name, variant
                            )));
                        }
                    }
                    TypeInfo::Generic { base, args } if base == enum_name => {
                        // Try to build the enum type from generic
                        if let Some(built_enum) = self.context.build_enum_type(enum_name, args.clone()) {
                            self.validate_pattern_against_type(pattern, &built_enum);
                        } else {
                            self.errors.push(TypeError::new(format!(
                                "cannot resolve generic enum '{}' with args {:?}",
                                enum_name, args
                            )));
                        }
                    }
                    _ => {
                        self.errors.push(TypeError::new(format!(
                            "cannot match enum pattern '{}' against non-enum type {}",
                            enum_name, ty.display_name()
                        )));
                    }
                }
            }
            ast::nodes::Pattern::Struct { name, fields } => {
                // Check that value type is compatible with struct pattern
                match ty {
                    TypeInfo::Struct { name: struct_name, fields: struct_fields } => {
                        if name != struct_name {
                            self.errors.push(TypeError::new(format!(
                                "struct pattern '{}' does not match value type {}",
                                name, struct_name
                            )));
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
                                self.errors.push(TypeError::new(format!(
                                    "struct '{}' has no field '{}'",
                                    name, field_name
                                )));
                            }
                        }
                    }
                    _ => {
                        self.errors.push(TypeError::new(format!(
                            "cannot match struct pattern '{}' against non-struct type {}",
                            name, ty.display_name()
                        )));
                    }
                }
            }
            ast::nodes::Pattern::Array { patterns, rest } => {
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
                        self.errors.push(TypeError::new(format!(
                            "cannot match array pattern against non-list type {}",
                            ty.display_name()
                        )));
                    }
                }

                // Check rest pattern usage
                if let Some(rest_name) = rest {
                    if patterns.is_empty() && rest_name.is_empty() {
                        self.errors.push(TypeError::new(
                            "invalid array pattern: empty rest pattern".to_string()
                        ));
                    }
                }
            }
        }
    }

    /// Type check a block
    fn check_block(&mut self, block: &Block) -> Result<()> {
        for statement in &block.statements {
            self.check_statement(statement)?;
        }
        Ok(())
    }

    /// Type check a statement
    fn check_statement(&mut self, statement: &Statement) -> Result<()> {
        match statement {
            Statement::Let { name, expr, .. } => {
                let expr_type = self.infer_expr_type(expr)?;
                self.context.insert_variable(name.clone(), expr_type);
            }
            Statement::Assignment { name, expr } => {
                let var_type = self
                    .context
                    .get_variable(name)
                    .ok_or_else(|| {
                        TypeError::new(format!("undefined variable: {}", name))
                            .with_hint(format!("did you mean to declare it with `let {}`?", name))
                            .with_help(
                                "Variables must be declared with `let` before they can be assigned"
                                    .to_string(),
                            )
                    })?
                    .clone();

                let expr_type = self.infer_expr_type(expr)?;
                if !expr_type.is_compatible_with(&var_type) {
                    self.errors.push(TypeError::new(format!(
                        "cannot assign {} to {} (expected {})",
                        expr_type.display_name(),
                        name,
                        var_type.display_name()
                    )).with_hint(format!("The variable `{}` is declared as `{}`, but you're trying to assign a value of type `{}`", name, var_type.display_name(), expr_type.display_name()))
                    .with_help("Make sure the types match or are compatible (e.g., i32 can be promoted to i64 or f64)".to_string()));
                }
            }
            Statement::If {
                cond,
                then_block,
                elif_blocks,
                else_block,
            } => {
                let cond_type = self.infer_expr_type(cond)?;
                if !cond_type.is_compatible_with(&TypeInfo::Bool) {
                    self.errors.push(TypeError::new(format!(
                        "if condition must be bool, got {}",
                        cond_type.display_name()
                    )));
                }

                self.check_block(then_block)?;
                for (_, block) in elif_blocks {
                    self.check_block(block)?;
                }
                if let Some(block) = else_block {
                    self.check_block(block)?;
                }
            }
            Statement::For {
                var,
                iterable,
                body,
            } => {
                let iter_type = self.infer_expr_type(iterable)?;
                let element_type = match &iter_type {
                    TypeInfo::List(elem) => elem.as_ref().clone(),
                    TypeInfo::Dict { value, .. } => value.as_ref().clone(),
                    TypeInfo::Str => TypeInfo::Str,
                    _ => {
                        self.errors.push(TypeError::new(format!(
                            "cannot iterate over type {}",
                            iter_type.display_name()
                        )));
                        TypeInfo::Unknown
                    }
                };

                let previous = self.context.remove_variable(var);
                self.context.insert_variable(var.clone(), element_type);
                self.check_block(body)?;
                match previous {
                    Some(prev) => {
                        self.context.insert_variable(var.clone(), prev);
                    }
                    None => {
                        self.context.remove_variable(var);
                    }
                }
            }
            Statement::While { cond, body } => {
                let cond_type = self.infer_expr_type(cond)?;
                if !cond_type.is_compatible_with(&TypeInfo::Bool) {
                    self.errors.push(TypeError::new(format!(
                        "while condition must be bool, got {}",
                        cond_type.display_name()
                    )));
                }
                self.check_block(body)?;
            }
            Statement::Return(expr) => {
                if let Some(expr) = expr {
                    let expr_type = self.infer_expr_type(expr)?;

                    // Check return type matches function signature
                    if let Some(expected_return_type) = &self.current_function_return_type {
                        if !expr_type.is_compatible_with(expected_return_type) {
                            self.errors.push(TypeError::new(format!(
                                "return type mismatch: expected {}, got {}",
                                expected_return_type.display_name(),
                                expr_type.display_name()
                            )));
                        }
                    } else {
                        // We're not inside a function context - this is an error
                        self.errors.push(TypeError::new(
                            "return statement outside of function".to_string()
                        ));
                    }
                } else {
                    // Bare return - check if function expects unit
                    if let Some(expected_return_type) = &self.current_function_return_type {
                        if !expected_return_type.is_compatible_with(&TypeInfo::Unit) {
                            self.errors.push(TypeError::new(format!(
                                "bare return in function that expects return type {}",
                                expected_return_type.display_name()
                            )));
                        }
                    } else {
                        // We're not inside a function context - this is an error
                        self.errors.push(TypeError::new(
                            "return statement outside of function".to_string()
                        ));
                    }
                }
            }
            Statement::Function(_) => {
                // Functions are handled separately
            }
            Statement::Expr(expr) => {
                let _expr_type = self.infer_expr_type(expr)?;
                // Expression statements are allowed (e.g., function calls)
            }
            Statement::Break | Statement::Continue => {
                // These are handled by loop context
            }
            Statement::Pass => {
                // No-op
            }
            Statement::Use { .. } => {
                // Module imports are handled separately
            }
            Statement::Struct { .. } => {
                // Struct definitions are handled at the module level
            }
            Statement::Enum { .. } => {
                // Enums are handled during the module pass
            }
            Statement::TypeAlias { .. } => {
                // Type aliases are handled at the module level
            }
            Statement::Block(block) => {
                self.check_block(block)?;
            }
            Statement::Try {
                body,
                handlers,
                else_block,
                finally_block,
            } => {
                // Type check the try body
                self.check_block(body)?;

                // Type check each handler
                for handler in handlers {
                    if let Some(exception_type) = &handler.exception {
                        // Convert AST type to TypeInfo
                        let handler_ty = self.context.type_from_annotation(exception_type);
                        // Validate that handler type is compatible with Error
                        if !handler_ty.is_compatible_with(&TypeInfo::Error) {
                            self.errors.push(TypeError::new(format!(
                                "Exception handler type '{}' is not compatible with Error type",
                                handler_ty.display_name()
                            )));
                        }
                    }

                    // Create a new scope for the handler body
                    let original_vars = self.context.variables.clone();
                    if let Some(alias) = &handler.alias {
                        // Bind the exception variable in the handler scope
                        self.context.insert_variable(alias.clone(), TypeInfo::Error);
                    }

                    // Type check the handler body in the scoped context
                    self.check_block(&handler.body)?;

                    // Restore the original variable context (handler variables don't leak)
                    self.context.variables = original_vars;
                }

                // Type check else block if present (runs when no exception)
                if let Some(else_block) = else_block {
                    self.check_block(else_block)?;
                }

                // Type check finally block if present (always runs)
                if let Some(finally_block) = finally_block {
                    self.check_block(finally_block)?;
                }
            }
            Statement::Raise(expr) => {
                if let Some(expr) = expr {
                    // Type check the expression being raised
                    let expr_ty = self.infer_expr_type(expr)?;
                    // The raised expression must be Error-compatible or a string (for convenience)
                    if !expr_ty.is_compatible_with(&TypeInfo::Error) && expr_ty != TypeInfo::Str {
                        self.errors.push(TypeError::new(format!(
                            "Cannot raise expression of type '{}', must be Error-compatible or a string",
                            expr_ty.display_name()
                        )));
                    }
                } else {
                    // Bare raise statement - only valid inside exception handlers
                    // For now, we'll allow it (checked at runtime)
                }
            }
        }
        Ok(())
    }

    /// Infer the type of an expression
    pub fn infer_expr_type(&mut self, expr: &Expr) -> Result<TypeInfo> {
        let ty = (|| -> Result<TypeInfo> {
            match expr {
                Expr::Literal(lit) => Ok(match lit {
                    Literal::Number(_num) => {
                        // Always infer numeric literals as F64 for simplicity and to avoid
                        // type inference issues with float contexts
                        // This matches Python's behavior where all numbers are floats by default
                        TypeInfo::F64
                    }
                    Literal::String(_) => TypeInfo::Str,
                    Literal::Bool(_) => TypeInfo::Bool,
                    Literal::None => TypeInfo::Unit,
                    Literal::Unit => TypeInfo::Unit,
                }),
                Expr::Identifier(name) => {
                    self.context.get_variable(name).cloned().ok_or_else(|| {
                        let err = TypeError::new(format!("undefined variable: {}", name))
                            .with_hint(format!("did you mean to declare it with `let {}`?", name))
                            .with_help("Variables must be declared before use".to_string());
                        anyhow::Error::from(err)
                    })
                }
                Expr::Binary { op, left, right } => {
                    let left_type = self.infer_expr_type(left)?;
                    let right_type = self.infer_expr_type(right)?;

                    match op {
                        ast::nodes::BinaryOp::Add
                        | ast::nodes::BinaryOp::Sub
                        | ast::nodes::BinaryOp::Mul
                        | ast::nodes::BinaryOp::Div => {
                            // Numeric operations
                            match (&left_type, &right_type) {
                                // String concatenation (must come before numeric patterns)
                                (TypeInfo::Str, TypeInfo::Str)
                                    if matches!(op, ast::nodes::BinaryOp::Add) =>
                                {
                                    Ok(TypeInfo::Str)
                                }
                                (TypeInfo::Str, TypeInfo::I64 | TypeInfo::I32)
                                    if matches!(op, ast::nodes::BinaryOp::Add) =>
                                {
                                    Ok(TypeInfo::Str)
                                }
                                (TypeInfo::I64 | TypeInfo::I32, TypeInfo::Str)
                                    if matches!(op, ast::nodes::BinaryOp::Add) =>
                                {
                                    Ok(TypeInfo::Str)
                                }
                                // Numeric operations
                                (TypeInfo::F64, _) | (_, TypeInfo::F64) => Ok(TypeInfo::F64),
                                (TypeInfo::I64, _) | (_, TypeInfo::I64) => Ok(TypeInfo::I64),
                                (TypeInfo::I32, TypeInfo::I32) => Ok(TypeInfo::I32),
                                _ => {
                                    self.errors.push(TypeError::new(format!(
                                        "cannot apply {} to {} and {}",
                                        format!("{:?}", op),
                                        left_type.display_name(),
                                        right_type.display_name()
                                    )));
                                    Ok(TypeInfo::Error)
                                }
                            }
                        }
                        ast::nodes::BinaryOp::Eq
                        | ast::nodes::BinaryOp::Ne
                        | ast::nodes::BinaryOp::Lt
                        | ast::nodes::BinaryOp::LtEq
                        | ast::nodes::BinaryOp::Gt
                        | ast::nodes::BinaryOp::GtEq => {
                            // Comparison operations return bool
                            if left_type.is_compatible_with(&right_type) {
                                Ok(TypeInfo::Bool)
                            } else {
                                self.errors.push(TypeError::new(format!(
                                    "cannot compare {} and {}",
                                    left_type.display_name(),
                                    right_type.display_name()
                                )));
                                Ok(TypeInfo::Error)
                            }
                        }
                        ast::nodes::BinaryOp::Is | ast::nodes::BinaryOp::IsNot => {
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
                                self.errors.push(TypeError::new(format!(
                                    "cannot use `is` between {} and {}",
                                    left_type.display_name(),
                                    right_type.display_name()
                                )));
                                Ok(TypeInfo::Error)
                            }
                        }
                        ast::nodes::BinaryOp::And | ast::nodes::BinaryOp::Or => {
                            // Logical operations require bool operands
                            if left_type.is_compatible_with(&TypeInfo::Bool)
                                && right_type.is_compatible_with(&TypeInfo::Bool)
                            {
                                Ok(TypeInfo::Bool)
                            } else {
                                self.errors.push(TypeError::new(format!(
                                    "logical operations require bool operands, got {} and {}",
                                    left_type.display_name(),
                                    right_type.display_name()
                                )));
                                Ok(TypeInfo::Error)
                            }
                        }
                        ast::nodes::BinaryOp::Mod => {
                            // Modulo requires integer operands
                            match (&left_type, &right_type) {
                                (TypeInfo::I32, TypeInfo::I32) => Ok(TypeInfo::I32),
                                (TypeInfo::I64, TypeInfo::I64) => Ok(TypeInfo::I64),
                                _ => {
                                    self.errors.push(TypeError::new(format!(
                                        "modulo requires integer operands, got {} and {}",
                                        left_type.display_name(),
                                        right_type.display_name()
                                    )));
                                    Ok(TypeInfo::Error)
                                }
                            }
                        }
                    }
                }
                Expr::Unary { op, expr } => {
                    let expr_type = self.infer_expr_type(expr)?;
                    match op {
                        ast::nodes::UnaryOp::Not => {
                            if expr_type.is_compatible_with(&TypeInfo::Bool) {
                                Ok(TypeInfo::Bool)
                            } else {
                                self.errors.push(TypeError::new(format!(
                                    "not operator requires bool operand, got {}",
                                    expr_type.display_name()
                                )));
                                Ok(TypeInfo::Error)
                            }
                        }
                        ast::nodes::UnaryOp::Neg => {
                            if expr_type.is_compatible_with(&TypeInfo::I32)
                                || expr_type.is_compatible_with(&TypeInfo::I64)
                                || expr_type.is_compatible_with(&TypeInfo::F64)
                            {
                                Ok(expr_type)
                            } else {
                                self.errors.push(TypeError::new(format!(
                                    "negation requires numeric operand, got {}",
                                    expr_type.display_name()
                                )));
                                Ok(TypeInfo::Error)
                            }
                        }
                    }
                }
                Expr::Call { func, args } => {
                    if let Some(enum_type) = self.try_eval_enum_constructor(func, args)? {
                        return Ok(enum_type);
                    }
                    let func_type = match func.as_ref() {
                        Expr::Identifier(name) => {
                            self.context.get_function(name).cloned().ok_or_else(|| {
                                let err = TypeError::new(format!("undefined function: {}", name));
                                anyhow::Error::from(err)
                            })?
                        }
                        Expr::Member { object, field } => {
                            // Support module.function() syntax for FFI and stdlib calls
                            if let Expr::Identifier(module) = object.as_ref() {
                                let full_name = format!("{}.{}", module, field);
                                // First check registry for exact FFI signatures
                                if let Some(registry) = self.registry {
                                    if let Some(symbol) = registry.resolve(&full_name) {
                                        let params: Vec<TypeInfo> = symbol
                                            .signature
                                            .params
                                            .iter()
                                            .map(|ft| ffi_type_to_typeinfo(ft))
                                            .collect();
                                        let return_type =
                                            ffi_type_to_typeinfo(&symbol.signature.result);
                                        return Ok(TypeInfo::Function {
                                            params,
                                            param_defaults: vec![
                                                false;
                                                symbol.signature.params.len()
                                            ],
                                            return_type: Box::new(return_type),
                                        });
                                    }
                                }
                                // Fall back to context functions
                                self.context
                                    .get_function(&full_name)
                                    .cloned()
                                    .unwrap_or_else(|| {
                                        // Return a generic function type for unknown FFI functions
                                        // This allows the code to pass type checking
                                        TypeInfo::Function {
                                            params: vec![], // Unknown params
                                            param_defaults: vec![],
                                            return_type: Box::new(TypeInfo::Unknown),
                                        }
                                    })
                            } else {
                                // Method call: obj.method() - infer object type and look up method
                                let obj_type = self.infer_expr_type(object)?;
                                if let TypeInfo::Struct { name, .. } = obj_type {
                                    let method_name = format!("{}.{}", name, field);
                                    self.context
                                        .get_function(&method_name)
                                        .cloned()
                                        .unwrap_or_else(|| {
                                            self.errors.push(TypeError::new(format!(
                                                "struct '{}' has no method '{}'",
                                                name, field
                                            )));
                                            TypeInfo::Error
                                        })
                                } else {
                                    self.errors.push(TypeError::new(format!(
                                        "method calls can only be used on struct instances, got {}",
                                        obj_type.display_name()
                                    )));
                                    TypeInfo::Error
                                }
                            }
                        }
                        _ => {
                            self.errors.push(TypeError::new(
                                "function calls must use identifier or module.function syntax"
                                    .to_string(),
                            ));
                            return Ok(TypeInfo::Error);
                        }
                    };

                    match func_type {
                        TypeInfo::Function {
                            params,
                            param_defaults,
                            return_type,
                        } => {
                            if !params.is_empty() {
                                let total_params = params.len();
                                let required_params =
                                    param_defaults.iter().filter(|flag| !**flag).count();

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
                                    ),
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
                                        ),
                                    );
                                    return Ok(TypeInfo::Error);
                                }

                                for (i, (arg, param_type)) in
                                    args.iter().zip(params.iter()).enumerate()
                                {
                                    let arg_type = self.infer_expr_type(arg)?;
                                    if !arg_type.is_compatible_with(param_type) {
                                        self.errors.push(TypeError::new(format!(
                                        "argument {} type mismatch: expected {}, got {}",
                                        i + 1,
                                        param_type.display_name(),
                                        arg_type.display_name()
                                    ))
                                    .with_hint(format!(
                                        "Argument {} should be of type `{}`",
                                        i + 1,
                                        param_type.display_name()
                                    ))
                                    .with_help("Check the function signature and ensure argument types match".to_string()));
                                    }
                                }
                            } else {
                                // For unknown FFI functions, just ensure arguments are type-checked
                                for arg in args {
                                    let _ = self.infer_expr_type(arg)?;
                                }
                            }

                            Ok(*return_type)
                        }
                        _ => {
                            self.errors.push(
                                TypeError::new(format!(
                                    "cannot call non-function type: {}",
                                    func_type.display_name()
                                ))
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

                    if start_type.is_compatible_with(&end_type) {
                        Ok(TypeInfo::Unknown) // Ranges are used for iteration
                    } else {
                        self.errors.push(TypeError::new(format!(
                            "range bounds must have compatible types, got {} and {}",
                            start_type.display_name(),
                            end_type.display_name()
                        )));
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
                        self.errors.push(TypeError::new(format!(
                            "if condition must be bool, got {}",
                            cond_type.display_name()
                        )));
                    }

                    let then_type = self.infer_expr_type(then_branch)?;
                    if let Some(else_expr) = else_branch {
                        let else_type = self.infer_expr_type(else_expr)?;

                        if then_type.is_compatible_with(&else_type) {
                            Ok(then_type)
                        } else {
                            self.errors.push(TypeError::new(format!(
                                "if branches must have compatible types, got {} and {}",
                                then_type.display_name(),
                                else_type.display_name()
                            )));
                            Ok(TypeInfo::Error)
                        }
                    } else {
                        // If expression without else returns unit
                        Ok(TypeInfo::Unit)
                    }
                }
                Expr::Member { object, field } => {
                    if let Expr::Identifier(enum_name) = object.as_ref() {
                        if let Some(definition) = self.context.get_enum(enum_name) {
                            if let Some(variant) = definition.variants.iter().find(|v| v.name == *field) {
                                if variant.fields.is_empty() {
                                    if let Some(enum_type) = self.context.build_enum_type(enum_name, vec![]) {
                                        return Ok(enum_type);
                                    }
                                }
                            }
                            return Ok(TypeInfo::Function {
                                params: vec![],
                                param_defaults: vec![],
                                return_type: Box::new(TypeInfo::Unknown),
                            });
                        }
                    }
                    
                    let object_type = self.infer_expr_type(object)?;
                    match &object_type {
                        TypeInfo::Struct { fields, .. } => {
                            if let Some(field_type) = fields.get(field) {
                                Ok(field_type.clone())
                            } else {
                                self.errors.push(
                                    TypeError::new(format!("struct has no field '{}'", field))
                                        .with_hint(format!(
                                            "Check the struct definition for available fields"
                                        )),
                                );
                                Ok(TypeInfo::Error)
                            }
                        }
                        TypeInfo::List(_) => Ok(TypeInfo::Unknown),
                        TypeInfo::Dict { .. } => Ok(TypeInfo::Unknown),
                        TypeInfo::Error => {
                            // Special handling for Error type fields
                            match field.as_str() {
                                "message" => Ok(TypeInfo::Str),
                                "code" => Ok(TypeInfo::I32),
                                "data" => Ok(TypeInfo::Str),
                                _ => {
                                    self.errors.push(TypeError::new(format!(
                                        "Error type has no field '{}'. Available fields: message, code, data",
                                        field
                                    )));
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
                                .with_hint(format!(
                                    "Only struct types and Error support member access"
                                )),
                            );
                            Ok(TypeInfo::Error)
                        }
                    }
                }
                Expr::FString { parts } => {
                    // F-strings always evaluate to strings
                    // Type check all embedded expressions
                    for part in parts {
                        if let ast::nodes::FStringPart::Expr(expr) = part {
                            let _ = self.infer_expr_type(expr)?;
                            // We don't care about the type, just that it's valid
                        }
                    }
                    Ok(TypeInfo::Str)
                }
                Expr::Lambda {
                    params,
                    ret_ty,
                    body,
                } => {
                    // Create a new context for the lambda
                    let mut lambda_context = self.context.clone();

                    // Add lambda parameters to context
                    let mut param_types = Vec::new();
                    let mut param_defaults = Vec::new();
                    for param in params {
                        let param_type = if let Some(ty) = &param.ty {
                            self.context.type_from_annotation(ty)
                        } else {
                            TypeInfo::Unknown
                        };
                        param_types.push(param_type.clone());
                        lambda_context.insert_variable(param.name.clone(), param_type);
                        param_defaults.push(param.default.is_some());
                    }

                    // Type check lambda body
                    let old_context = std::mem::replace(&mut self.context, lambda_context);
                    let body_result = self.check_block(body);
                    self.context = old_context;

                    if body_result.is_err() {
                        return Ok(TypeInfo::Error);
                    }

                    let return_type = if let Some(ty) = ret_ty {
                        self.context.type_from_annotation(ty)
                    } else {
                        // Try to infer return type from body
                        // For now, assume unit if no return statement
                        TypeInfo::Unit
                    };

                    Ok(TypeInfo::Function {
                        params: param_types,
                        param_defaults,
                        return_type: Box::new(return_type),
                    })
                }
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
                        self.errors.push(TypeError::new(format!(
                            "list comprehension expects list iterable, got {}",
                            iterable_type.display_name()
                        )));
                        TypeInfo::Unknown
                    };

                    let previous = self.context.remove_variable(var);
                    self.context
                        .insert_variable(var.clone(), element_iter_type.clone());

                    if let Some(cond_expr) = condition {
                        let cond_type = self.infer_expr_type(cond_expr)?;
                        if !cond_type.is_compatible_with(&TypeInfo::Bool) {
                            self.errors.push(TypeError::new(format!(
                                "list comprehension condition must be bool, got {}",
                                cond_type.display_name()
                            )));
                        }
                    }

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
                        self.errors.push(TypeError::new(format!(
                            "dict comprehension expects list iterable, got {}",
                            iterable_type.display_name()
                        )));
                        TypeInfo::Unknown
                    };

                    let previous = self.context.remove_variable(var);
                    self.context
                        .insert_variable(var.clone(), element_iter_type.clone());

                    if let Some(cond_expr) = condition {
                        let cond_type = self.infer_expr_type(cond_expr)?;
                        if !cond_type.is_compatible_with(&TypeInfo::Bool) {
                            self.errors.push(TypeError::new(format!(
                                "dict comprehension condition must be bool, got {}",
                                cond_type.display_name()
                            )));
                        }
                    }

                    let key_type = self.infer_expr_type(key)?;
                    if !key_type.is_compatible_with(&TypeInfo::Str) {
                        self.errors.push(TypeError::new(format!(
                            "dict comprehension key must be string, got {}",
                            key_type.display_name()
                        )));
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
                        if let TypeInfo::Generic { base, args } = &normalized_type {
                            if let ast::nodes::Pattern::EnumVariant { enum_name, .. } = &arm.pattern {
                                if base == enum_name {
                                    // Try to build the enum type directly using the pattern's enum name
                                    if let Some(built_enum) = self.context.build_enum_type(enum_name, args.clone()) {
                                        normalized_type = built_enum;
                                    }
                                }
                            }
                        }

                        // Check pattern matches value type with sophisticated validation
                        self.validate_pattern_against_type(&arm.pattern, &normalized_type);

                        let old_vars = self.context.variables.clone();
                        self.bind_pattern_variables(&arm.pattern, &normalized_type);

                        // Check guard if present
                        if let Some(guard) = &arm.guard {
                            let guard_type = self.infer_expr_type(guard)?;
                            if !guard_type.is_compatible_with(&TypeInfo::Bool) {
                                self.errors.push(TypeError::new(format!(
                                    "match guard must be bool, got {}",
                                    guard_type.display_name()
                                )));
                            }
                        }

                        // Get arm body type
                        let body_type = self.infer_expr_type(&arm.body)?;
                        arm_types.push(body_type);

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
                    let struct_fields = match self.context.get_struct(name) {
                        Some(fields) => fields.clone(),
                        None => {
                            self.errors.push(
                                TypeError::new(format!("unknown struct type: {}", name)).with_hint(
                                    "Check that the struct is defined before use".to_string(),
                                ),
                            );
                            return Ok(TypeInfo::Error);
                        }
                    };

                    // Check that all provided fields exist and have correct types
                    let mut provided_fields = std::collections::HashSet::new();
                    for (field_name, field_expr) in fields {
                        if provided_fields.contains(field_name) {
                            self.errors.push(TypeError::new(format!(
                                "duplicate field '{}' in struct initialization",
                                field_name
                            )));
                        }
                        provided_fields.insert(field_name.clone());

                        let field_type = match struct_fields.get(field_name) {
                            Some(ty) => ty,
                            None => {
                                self.errors.push(
                                    TypeError::new(format!(
                                        "struct '{}' has no field '{}'",
                                        name, field_name
                                    ))
                                    .with_hint(format!(
                                        "Available fields: {}",
                                        struct_fields
                                            .keys()
                                            .cloned()
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    )),
                                );
                                continue;
                            }
                        };

                        let expr_type = self.infer_expr_type(field_expr)?;
                        if !expr_type.is_compatible_with(field_type) {
                            self.errors.push(TypeError::new(format!(
                                "field '{}' of struct '{}' expects type {}, got {}",
                                field_name,
                                name,
                                field_type.display_name(),
                                expr_type.display_name()
                            )));
                        }
                    }

                    // Check that all required fields are provided (for now, all fields are required)
                    for field_name in struct_fields.keys() {
                        if !provided_fields.contains(field_name) {
                            self.errors.push(TypeError::new(format!(
                                "missing required field '{}' in struct '{}' initialization",
                                field_name, name
                            )));
                        }
                    }

                    Ok(TypeInfo::Struct {
                        name: name.clone(),
                        fields: struct_fields,
                    })
                }
                Expr::Await(expr) => {
                    // Await expects an async/awaitable type
                    let inner_type = self.infer_expr_type(expr)?;

                    // Check if inner_type is actually awaitable
                    match &inner_type {
                        // Task handles from spawn operations (generic types)
                        TypeInfo::Generic { base, args: _ } if base == "Task" || base == "Future" => {
                            // Explicitly async types are awaitable
                        }
                        // Any type for now - in early development, be permissive
                        // In a full implementation, we'd be more restrictive
                        _ => {
                            // For now, allow awaiting on any type since async system is in development
                            // TODO: Restrict to only proper async types when the async system matures
                        }
                    }

                    // For await, we return the inner type (unwrap the async wrapper)
                    Ok(inner_type)
                }
                Expr::Spawn(expr) => {
                    // Spawn creates a task from an expression
                    // Type check the inner expression
                    let _inner_type = self.infer_expr_type(expr)?;

                    // Spawn returns a task handle (for now, return Unknown)
                    Ok(TypeInfo::Unknown)
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

    pub fn into_expr_type_map(self) -> HashMap<usize, TypeInfo> {
        self.expr_types
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
        FfiType::I64 => TypeInfo::I64,
        FfiType::F64 => TypeInfo::F64,
        FfiType::Str => TypeInfo::Str,
        FfiType::Opaque => TypeInfo::I64, // Opaque handles are i64 at type level
        FfiType::List => TypeInfo::List(Box::new(TypeInfo::Unknown)),
        FfiType::Map => TypeInfo::Dict {
            key: Box::new(TypeInfo::Unknown),
            value: Box::new(TypeInfo::Unknown),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast::nodes::{BinaryOp, Expr, Literal, NumberLiteral};
    use std::f64;

    #[test]
    fn test_type_inference_literal() {
        let mut checker = TypeChecker::new();

        let expr = Expr::Literal(Literal::Number(NumberLiteral::new(42.0, false)));
        let ty = checker.infer_expr_type(&expr).unwrap();
        assert_eq!(ty, TypeInfo::F64);

        let expr = Expr::Literal(Literal::Number(NumberLiteral::new(f64::consts::PI, true)));
        let ty = checker.infer_expr_type(&expr).unwrap();
        assert_eq!(ty, TypeInfo::F64);
    }

    #[test]
    fn test_type_inference_binary() {
        let mut checker = TypeChecker::new();

        let expr = Expr::Binary {
            op: BinaryOp::Add,
            left: Box::new(Expr::Literal(Literal::Number(NumberLiteral::new(
                1.0, true,
            )))),
            right: Box::new(Expr::Literal(Literal::Number(NumberLiteral::new(
                2.0, true,
            )))),
        };
        let ty = checker.infer_expr_type(&expr).unwrap();
        assert_eq!(ty, TypeInfo::F64);
    }
}
