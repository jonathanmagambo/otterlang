use std::collections::HashMap;

use ast::nodes::Type;

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
    /// Unknown type (needs inference)
    Unknown,
    /// Error type (used for error recovery)
    Error,
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
            | (TypeInfo::Str, TypeInfo::Str) => true,

            // Numeric promotions
            (TypeInfo::I32, TypeInfo::I64) | (TypeInfo::I32, TypeInfo::F64) => true,
            (TypeInfo::I64, TypeInfo::F64) => true,

            // Unknown types are compatible with anything (during inference)
            (TypeInfo::Unknown, _) | (_, TypeInfo::Unknown) => true,

            // Struct types must match exactly
            (TypeInfo::Struct { name: n1, .. }, TypeInfo::Struct { name: n2, .. }) => n1 == n2,

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

            // Error types are compatible with strings (for convenience) and themselves
            (TypeInfo::Error, TypeInfo::Error) => true,
            (TypeInfo::Str, TypeInfo::Error) => true, // Allow raising strings as errors
            (TypeInfo::Error, _) => false, // Error types are not compatible with anything else
            (_, TypeInfo::Error) => false, // Nothing else is compatible with Error (except strings above)

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
            TypeInfo::Unknown => "?".to_string(),
            TypeInfo::Error => "<error>".to_string(),
        }
    }
}

impl From<&Type> for TypeInfo {
    fn from(ty: &Type) -> Self {
        match ty {
            Type::Simple(name) => match name.as_str() {
                "unit" | "None" | "none" => TypeInfo::Unit,
                "bool" => TypeInfo::Bool,
                "i32" => TypeInfo::I32,
                "i64" => TypeInfo::I64,
                "f64" | "float" => TypeInfo::F64,
                "str" => TypeInfo::Str,
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
                    let element = args.get(0).map(TypeInfo::from).unwrap_or(TypeInfo::Unknown);
                    TypeInfo::List(Box::new(element))
                }
                "Dict" | "dict" => {
                    let key = args.get(0).map(TypeInfo::from).unwrap_or(TypeInfo::Unknown);
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
}

impl TypeError {
    pub fn new(message: String) -> Self {
        Self {
            message,
            hint: None,
            help: None,
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
}

impl std::fmt::Display for TypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(hint) = &self.hint {
            write!(f, "\n💡 Suggestion: {}", hint)?;
        }
        if let Some(help) = &self.help {
            write!(f, "\nℹ️  {}", help)?;
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
    /// Struct definitions: name -> (field_name -> field_type)
    pub structs: HashMap<String, HashMap<String, TypeInfo>>,
    /// Type aliases: name -> actual type
    pub type_aliases: HashMap<String, TypeInfo>,
}

impl TypeContext {
    pub fn new() -> Self {
        Self {
            variables: HashMap::new(),
            functions: HashMap::new(),
            generic_params: Vec::new(),
            structs: HashMap::new(),
            type_aliases: HashMap::new(),
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

    pub fn define_struct(&mut self, name: String, fields: HashMap<String, TypeInfo>) {
        self.structs.insert(name, fields);
    }

    pub fn get_struct(&self, name: &str) -> Option<&HashMap<String, TypeInfo>> {
        self.structs.get(name)
    }

    pub fn define_type_alias(&mut self, name: String, ty: TypeInfo) {
        self.type_aliases.insert(name, ty);
    }

    pub fn resolve_type_alias(&self, name: &str) -> Option<&TypeInfo> {
        self.type_aliases.get(name)
    }
}

impl Default for TypeContext {
    fn default() -> Self {
        Self::new()
    }
}
