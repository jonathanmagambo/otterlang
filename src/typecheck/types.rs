use std::collections::HashMap;

use otterc_ast::nodes::{EnumVariant, Node, Type};
use otterc_span::Span;

use otterc_language::LanguageFeatureFlags;

/// Represents a type in the type system
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeInfo {
    /// Unit type (no value)
    Unit,
    /// Boolean type
    Bool,
    /// 32-bit integer
    I32,
    /// 64-bit integer
    I64,
    /// 64-bit floating point
    F64,
    /// String type
    Str,
    /// List type with element type information
    List(Box<TypeInfo>),
    /// Dictionary type with key/value types
    Dict {
        key: Box<TypeInfo>,
        value: Box<TypeInfo>,
    },
    /// Range iterable type
    Range(Box<TypeInfo>, Box<TypeInfo>),
    /// Function type with parameter and return types
    Function {
        params: Vec<TypeInfo>,
        param_defaults: Vec<bool>,
        return_type: Box<TypeInfo>,
    },
    /// Generic type (e.g., List<T>, Map<K, V>)
    Generic { base: String, args: Vec<TypeInfo> },
    /// Struct type (record type)
    Struct {
        name: String,
        fields: HashMap<String, TypeInfo>,
    },
    /// Enum type with named variants
    Enum {
        name: String,
        args: Vec<TypeInfo>,
        variants: HashMap<String, EnumVariantInfo>,
    },
    /// Strong type alias (newtype-style)
    Alias {
        name: String,
        underlying: Box<TypeInfo>,
        is_public: bool,
    },
    /// Unknown type (needs inference)
    Unknown,
    /// Error type (used for error recovery)
    Error,
    /// Module type (for FFI modules like `rand`, `chrono`, etc.)
    Module(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumVariantInfo {
    pub name: String,
    pub fields: Vec<TypeInfo>,
}

impl TypeInfo {
    /// Check if this type is a generic type parameter
    pub fn is_generic_param(&self) -> bool {
        matches!(self, TypeInfo::Generic { base: _, args } if args.is_empty())
    }

    /// Substitute generic type parameters with concrete types
    pub fn substitute(&self, substitutions: &HashMap<String, TypeInfo>) -> TypeInfo {
        match self {
            TypeInfo::Generic { base, args } if args.is_empty() => {
                // This is a generic type parameter
                substitutions
                    .get(base)
                    .cloned()
                    .unwrap_or_else(|| self.clone())
            }
            TypeInfo::Generic { base, args } => {
                // This is a generic type with arguments
                TypeInfo::Generic {
                    base: base.clone(),
                    args: args.iter().map(|a| a.substitute(substitutions)).collect(),
                }
            }
            TypeInfo::Function {
                params,
                param_defaults,
                return_type,
            } => TypeInfo::Function {
                params: params.iter().map(|p| p.substitute(substitutions)).collect(),
                param_defaults: param_defaults.clone(),
                return_type: Box::new(return_type.substitute(substitutions)),
            },
            TypeInfo::List(element) => TypeInfo::List(Box::new(element.substitute(substitutions))),
            TypeInfo::Dict { key, value } => TypeInfo::Dict {
                key: Box::new(key.substitute(substitutions)),
                value: Box::new(value.substitute(substitutions)),
            },
            TypeInfo::Struct { name, fields } => TypeInfo::Struct {
                name: name.clone(),
                fields: fields
                    .iter()
                    .map(|(k, v)| (k.clone(), v.substitute(substitutions)))
                    .collect(),
            },
            TypeInfo::Enum {
                name,
                args,
                variants,
            } => TypeInfo::Enum {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|arg| arg.substitute(substitutions))
                    .collect(),
                variants: variants
                    .iter()
                    .map(|(variant_name, variant)| {
                        (
                            variant_name.clone(),
                            EnumVariantInfo {
                                name: variant.name.clone(),
                                fields: variant
                                    .fields
                                    .iter()
                                    .map(|field| field.substitute(substitutions))
                                    .collect(),
                            },
                        )
                    })
                    .collect(),
            },
            TypeInfo::Alias {
                name,
                underlying,
                is_public,
            } => TypeInfo::Alias {
                name: name.clone(),
                underlying: Box::new(underlying.substitute(substitutions)),
                is_public: *is_public,
            },
            TypeInfo::Module(name) => TypeInfo::Module(name.clone()),
            _ => self.clone(),
        }
    }

    /// Check if this type is compatible with another type
    pub fn is_compatible_with(&self, other: &TypeInfo) -> bool {
        match (self, other) {
            // Same types are compatible
            (TypeInfo::Unit, TypeInfo::Unit)
            | (TypeInfo::Bool, TypeInfo::Bool)
            | (TypeInfo::I32, TypeInfo::I32)
            | (TypeInfo::I64, TypeInfo::I64)
            | (TypeInfo::F64, TypeInfo::F64)
            | (TypeInfo::Str, TypeInfo::Str)
            // Numeric promotions
            | (TypeInfo::I32, TypeInfo::I64) | (TypeInfo::I32, TypeInfo::F64)
            | (TypeInfo::I64, TypeInfo::F64)
            // Unknown types are compatible with anything (during inference)
            | (TypeInfo::Unknown, _) | (_, TypeInfo::Unknown)
            // Error types are compatible with strings (for convenience) and themselves
            | (TypeInfo::Error, TypeInfo::Error)
            | (TypeInfo::Str, TypeInfo::Error) // Allow raising strings as errors
            | (TypeInfo::Error, _) // Error types are not compatible with anything else
            | (_, TypeInfo::Error) => false, // Nothing else is compatible with Error (except strings above)

            // Struct types must match exactly
            (TypeInfo::Struct { name: n1, .. }, TypeInfo::Struct { name: n2, .. })
            // Module types are compatible with themselves
            | (TypeInfo::Module(n1), TypeInfo::Module(n2)) => n1 == n2,

            // Generic types must match structure
            (
                TypeInfo::Generic { base: b1, args: a1 },
                TypeInfo::Generic { base: b2, args: a2 },
            ) => {
                if b1 != b2 {
                    return false;
                }

                // If one is a generic parameter and the other is concrete, they're compatible
                if a1.is_empty() && !a2.is_empty() {
                    return true; // Generic parameter accepts any concrete type
                }
                if !a1.is_empty() && a2.is_empty() {
                    return true; // Generic parameter accepts any concrete type
                }

                // Both have arguments, check compatibility
                a1.len() == a2.len()
                    && a1
                        .iter()
                        .zip(a2.iter())
                        .all(|(t1, t2)| t1.is_compatible_with(t2))
            }
            (TypeInfo::List(elem1), TypeInfo::List(elem2)) => elem1.is_compatible_with(elem2),
            (TypeInfo::Dict { key: k1, value: v1 }, TypeInfo::Dict { key: k2, value: v2 }) => {
                k1.is_compatible_with(k2) && v1.is_compatible_with(v2)
            }
            (
                TypeInfo::Enum {
                    name: name_a,
                    args: args_a,
                    ..
                },
                TypeInfo::Enum {
                    name: name_b,
                    args: args_b,
                    ..
                },
            ) => {
                name_a == name_b
                    && args_a.len() == args_b.len()
                    && args_a
                        .iter()
                        .zip(args_b.iter())
                        .all(|(a, b)| a.is_compatible_with(b))
            }
            (
                TypeInfo::Enum { name, args, .. },
                TypeInfo::Generic {
                    base,
                    args: other_args,
                },
            )
            | (
                TypeInfo::Generic {
                    base,
                    args: other_args,
                },
                TypeInfo::Enum { name, args, .. },
            ) => {
                if name != base {
                    return false;
                }
                if other_args.is_empty() {
                    true
                } else {
                    args.len() == other_args.len()
                        && args
                            .iter()
                            .zip(other_args.iter())
                            .all(|(a, b)| a.is_compatible_with(b))
                }
            }
            (
                TypeInfo::Alias {
                    underlying: alias, ..
                },
                other,
            ) => alias.is_compatible_with(other),
            (
                other,
                TypeInfo::Alias {
                    underlying: alias, ..
                },
            ) => other.is_compatible_with(alias),

            // Function types must match signature
            (
                TypeInfo::Function {
                    params: p1,
                    param_defaults: d1,
                    return_type: r1,
                },
                TypeInfo::Function {
                    params: p2,
                    param_defaults: d2,
                    return_type: r2,
                },
            ) => {
                p1.len() == p2.len()
                    && d1.len() == d2.len()
                    && p1
                        .iter()
                        .zip(p2.iter())
                        .all(|(t1, t2)| t1.is_compatible_with(t2))
                    && d1.iter().zip(d2.iter()).all(|(a, b)| a == b)
                    && r1.is_compatible_with(r2)
            }
            _ => false,
        }
    }

    /// Get a display name for the type
    pub fn display_name(&self) -> String {
        match self {
            TypeInfo::Unit => "None".to_string(),
            TypeInfo::Bool => "bool".to_string(),
            TypeInfo::I32 => "i32".to_string(),
            TypeInfo::I64 => "i64".to_string(),
            TypeInfo::F64 => "f64".to_string(),
            TypeInfo::Str => "str".to_string(),
            TypeInfo::Function {
                params,
                param_defaults: _,
                return_type,
            } => {
                let params_str = params
                    .iter()
                    .map(|t| t.display_name())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("fn({}) -> {}", params_str, return_type.display_name())
            }
            TypeInfo::List(element) => format!("list<{}>", element.display_name()),
            TypeInfo::Dict { key, value } => {
                format!("dict<{}, {}>", key.display_name(), value.display_name())
            }
            TypeInfo::Range(start, end) => {
                format!("range<{}, {}>", start.display_name(), end.display_name())
            }
            TypeInfo::Generic { base, args } => {
                if args.is_empty() {
                    base.clone()
                } else {
                    let args_str = args
                        .iter()
                        .map(|t| t.display_name())
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{}<{}>", base, args_str)
                }
            }
            TypeInfo::Struct { name, fields } => {
                if fields.is_empty() {
                    name.clone()
                } else {
                    let fields_str = fields
                        .iter()
                        .map(|(f, t)| format!("{}: {}", f, t.display_name()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{} {{ {} }}", name, fields_str)
                }
            }
            TypeInfo::Enum { name, args, .. } => {
                if args.is_empty() {
                    name.clone()
                } else {
                    let args_str = args
                        .iter()
                        .map(|t| t.display_name())
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{}<{}>", name, args_str)
                }
            }
            TypeInfo::Alias { name, .. } => name.clone(),
            TypeInfo::Unknown => "?".to_string(),
            TypeInfo::Error => "<error>".to_string(),
            TypeInfo::Module(name) => format!("module<{}>", name),
        }
    }

    /// Check if the type is an integer type (i32 or i64)
    pub fn is_integer(&self) -> bool {
        matches!(self, TypeInfo::I32 | TypeInfo::I64)
    }
}

impl From<&Type> for TypeInfo {
    fn from(ty: &Type) -> Self {
        match ty {
            Type::Simple(name) => match name.as_str() {
                "unit" | "None" | "none" => TypeInfo::Unit,
                "bool" => TypeInfo::Bool,
                "i32" => TypeInfo::I32,
                "i64" | "int" => TypeInfo::I64,
                "f64" | "float" => TypeInfo::F64,
                "str" | "string" => TypeInfo::Str,
                "list" | "List" => TypeInfo::List(Box::new(TypeInfo::Unknown)),
                "dict" | "Dict" => TypeInfo::Dict {
                    key: Box::new(TypeInfo::Unknown),
                    value: Box::new(TypeInfo::Unknown),
                },
                "Error" => TypeInfo::Error,
                _ => TypeInfo::Generic {
                    base: name.clone(),
                    args: Vec::new(),
                },
            },
            Type::Generic { base, args } => match base.as_str() {
                "List" | "list" => {
                    let element = args
                        .first()
                        .map(TypeInfo::from)
                        .unwrap_or(TypeInfo::Unknown);
                    TypeInfo::List(Box::new(element))
                }
                "Dict" | "dict" => {
                    let key = args
                        .first()
                        .map(TypeInfo::from)
                        .unwrap_or(TypeInfo::Unknown);
                    let value = args.get(1).map(TypeInfo::from).unwrap_or(TypeInfo::Unknown);
                    TypeInfo::Dict {
                        key: Box::new(key),
                        value: Box::new(value),
                    }
                }
                _ => TypeInfo::Generic {
                    base: base.clone(),
                    args: args.iter().map(|t| t.into()).collect(),
                },
            },
        }
    }
}

impl From<&Node<Type>> for TypeInfo {
    fn from(node: &Node<Type>) -> Self {
        TypeInfo::from(node.as_ref())
    }
}

impl From<&str> for TypeInfo {
    fn from(name: &str) -> Self {
        match name {
            "unit" => TypeInfo::Unit,
            "bool" => TypeInfo::Bool,
            "i32" => TypeInfo::I32,
            "i64" => TypeInfo::I64,
            "f64" => TypeInfo::F64,
            "str" => TypeInfo::Str,
            "list" | "List" => TypeInfo::List(Box::new(TypeInfo::Unknown)),
            "dict" | "Dict" => TypeInfo::Dict {
                key: Box::new(TypeInfo::Unknown),
                value: Box::new(TypeInfo::Unknown),
            },
            _ => TypeInfo::Generic {
                base: name.to_string(),
                args: Vec::new(),
            },
        }
    }
}

/// Type checking error
#[derive(Debug, Clone)]
pub struct TypeError {
    pub message: String,
    pub hint: Option<String>,
    pub help: Option<String>,
    pub suggestion: Option<String>,
    pub span: Option<Span>,
}

impl TypeError {
    pub fn new(message: String) -> Self {
        Self {
            message,
            hint: None,
            help: None,
            suggestion: None,
            span: None,
        }
    }

    pub fn with_hint(mut self, hint: String) -> Self {
        self.hint = Some(hint);
        self
    }

    pub fn with_help(mut self, help: String) -> Self {
        self.help = Some(help);
        self
    }

    pub fn with_suggestion(mut self, suggestion: String) -> Self {
        self.suggestion = Some(suggestion);
        self
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    pub fn with_optional_span(mut self, span: Option<Span>) -> Self {
        self.span = span;
        self
    }
}

impl std::fmt::Display for TypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(hint) = &self.hint {
            write!(f, "\nSuggestion: {}", hint)?;
        }
        if let Some(help) = &self.help {
            write!(f, "\n{}", help)?;
        }
        Ok(())
    }
}

impl std::error::Error for TypeError {}

/// Context for type checking
#[derive(Debug, Clone)]
pub struct TypeContext {
    /// Variables and their types
    pub variables: HashMap<String, TypeInfo>,
    /// Functions and their signatures
    pub functions: HashMap<String, TypeInfo>,
    /// Generic type parameters in scope
    pub generic_params: Vec<String>,
    /// Struct definitions: name -> definition
    pub structs: HashMap<String, StructDefinition>,
    /// Type aliases: name -> actual type
    pub type_aliases: HashMap<String, TypeInfo>,
    /// Enum definitions available in the current module
    pub enums: HashMap<String, EnumDefinition>,
    /// Active language feature flags
    pub features: LanguageFeatureFlags,
}

impl TypeContext {
    pub fn new() -> Self {
        Self::with_features(LanguageFeatureFlags::default())
    }

    pub fn with_features(features: LanguageFeatureFlags) -> Self {
        Self {
            variables: HashMap::new(),
            functions: HashMap::new(),
            generic_params: Vec::new(),
            structs: HashMap::new(),
            type_aliases: HashMap::new(),
            enums: HashMap::new(),
            features,
        }
    }

    pub fn with_function(mut self, name: String, ty: TypeInfo) -> Self {
        self.functions.insert(name, ty);
        self
    }

    pub fn insert_function(&mut self, name: String, ty: TypeInfo) {
        self.functions.insert(name, ty);
    }

    pub fn insert_variable(&mut self, name: String, ty: TypeInfo) {
        self.variables.insert(name, ty);
    }

    pub fn get_variable(&self, name: &str) -> Option<&TypeInfo> {
        self.variables.get(name)
    }

    pub fn remove_variable(&mut self, name: &str) -> Option<TypeInfo> {
        self.variables.remove(name)
    }

    pub fn get_function(&self, name: &str) -> Option<&TypeInfo> {
        self.functions.get(name)
    }

    pub fn push_generic(&mut self, param: String) {
        self.generic_params.push(param);
    }

    pub fn pop_generic(&mut self) {
        self.generic_params.pop();
    }

    pub fn is_generic(&self, name: &str) -> bool {
        self.generic_params.contains(&name.to_string())
    }

    pub fn define_struct(&mut self, definition: StructDefinition) {
        self.structs.insert(definition.name.clone(), definition);
    }

    pub fn get_struct(&self, name: &str) -> Option<&StructDefinition> {
        self.structs.get(name)
    }

    pub fn define_type_alias(&mut self, name: String, ty: TypeInfo, is_public: bool) {
        let stored_type = if self.features.newtype_aliases {
            TypeInfo::Alias {
                name: name.clone(),
                underlying: Box::new(ty),
                is_public,
            }
        } else {
            ty
        };
        self.type_aliases.insert(name, stored_type);
    }

    pub fn resolve_type_alias(&self, name: &str) -> Option<&TypeInfo> {
        self.type_aliases.get(name)
    }

    pub fn define_enum(&mut self, definition: EnumDefinition) {
        self.enums.insert(definition.name.clone(), definition);
    }

    pub fn get_enum(&self, name: &str) -> Option<&EnumDefinition> {
        self.enums.get(name)
    }

    pub fn enum_variant(&self, enum_name: &str, variant: &str) -> Option<&EnumVariant> {
        self.enums
            .get(enum_name)
            .and_then(|definition| definition.variants.iter().find(|v| v.name == variant))
    }

    pub fn enum_layouts(&self) -> HashMap<String, EnumLayout> {
        self.enums
            .iter()
            .map(|(name, definition)| {
                let variant_fields = definition
                    .variants
                    .iter()
                    .map(|variant| {
                        let fields = variant
                            .fields
                            .iter()
                            .map(|ty| TypeInfo::from(ty.as_ref()))
                            .collect();
                        (variant.name.clone(), fields)
                    })
                    .collect();
                (
                    name.clone(),
                    EnumLayout {
                        name: name.clone(),
                        generics: definition.generics.clone(),
                        variants: definition
                            .variants
                            .iter()
                            .map(|variant| variant.name.clone())
                            .collect(),
                        variant_fields,
                    },
                )
            })
            .collect()
    }

    pub fn build_enum_type(&self, name: &str, args: Vec<TypeInfo>) -> Option<TypeInfo> {
        let definition = self.enums.get(name)?;
        let mut normalized_args = if args.is_empty() {
            vec![TypeInfo::Unknown; definition.generics.len()]
        } else if args.len() < definition.generics.len() {
            let mut padded = args;
            padded.extend(std::iter::repeat_n(
                TypeInfo::Unknown,
                definition.generics.len() - padded.len(),
            ));
            padded
        } else {
            args
        };

        if normalized_args.len() > definition.generics.len() {
            normalized_args.truncate(definition.generics.len());
        }

        let substitutions: HashMap<String, TypeInfo> = definition
            .generics
            .iter()
            .cloned()
            .zip(normalized_args.iter().cloned())
            .collect();

        let variants = definition
            .variants
            .iter()
            .map(|variant| {
                let fields = variant
                    .fields
                    .iter()
                    .map(|ty| {
                        let raw: TypeInfo = ty.into();
                        raw.substitute(&substitutions)
                    })
                    .collect();
                (
                    variant.name.clone(),
                    EnumVariantInfo {
                        name: variant.name.clone(),
                        fields,
                    },
                )
            })
            .collect();

        Some(TypeInfo::Enum {
            name: name.to_string(),
            args: normalized_args,
            variants,
        })
    }

    pub fn normalize_type(&self, ty: TypeInfo) -> TypeInfo {
        match ty {
            TypeInfo::Generic { base, args } => {
                if args.is_empty()
                    && let Some(struct_def) = self.structs.get(&base)
                {
                    return TypeInfo::Struct {
                        name: struct_def.name.clone(),
                        fields: struct_def.fields.clone(),
                    };
                }
                if let Some(enum_ty) = self.build_enum_type(&base, args.clone()) {
                    enum_ty
                } else {
                    TypeInfo::Generic { base, args }
                }
            }
            other => other,
        }
    }

    pub fn type_from_annotation(&self, ty: &Node<Type>) -> TypeInfo {
        let mut info = TypeInfo::from(ty);
        if let TypeInfo::Generic { base, args } = &info
            && args.is_empty()
            && let Some(aliased_type) = self.resolve_type_alias(base)
        {
            info = aliased_type.clone();
        }
        self.normalize_type(info)
    }

    pub fn set_language_features(&mut self, features: LanguageFeatureFlags) {
        self.features = features;
    }
}

impl Default for TypeContext {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct StructDefinition {
    pub name: String,
    pub generics: Vec<String>,
    pub fields: HashMap<String, TypeInfo>,
}

#[derive(Debug, Clone)]
pub struct EnumDefinition {
    pub name: String,
    pub generics: Vec<String>,
    pub variants: Vec<EnumVariant>,
}

#[derive(Debug, Clone)]
pub struct EnumLayout {
    pub name: String,
    pub generics: Vec<String>,
    pub variants: Vec<String>,
    pub variant_fields: HashMap<String, Vec<TypeInfo>>,
}

impl EnumLayout {
    pub fn tag_of(&self, variant: &str) -> Option<u32> {
        self.variants
            .iter()
            .position(|name| name == variant)
            .map(|idx| idx as u32)
    }

    pub fn fields_of(&self, variant: &str, args: &[TypeInfo]) -> Option<Vec<TypeInfo>> {
        let fields = self.variant_fields.get(variant)?;
        if self.generics.is_empty() {
            return Some(fields.clone());
        }

        let mut subs = HashMap::new();
        for (generic, arg) in self.generics.iter().zip(args.iter()) {
            subs.insert(generic.clone(), arg.clone());
        }

        // If the caller supplied fewer args than required, treat the remainder as Unknown
        if args.len() < self.generics.len() {
            for generic in self.generics.iter().skip(args.len()) {
                subs.entry(generic.clone()).or_insert(TypeInfo::Unknown);
            }
        }

        Some(fields.iter().map(|ty| ty.substitute(&subs)).collect())
    }
}
