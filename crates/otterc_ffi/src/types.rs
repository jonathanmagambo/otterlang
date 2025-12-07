use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Template for how a stub should invoke the underlying Rust function.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum CallTemplate {
    Direct,
    Result,
    Expr(String),
}

/// Describes a single extern "C" function to be exposed through the bridge.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FunctionSpec {
    pub name: String,
    pub symbol: String,
    pub params: Vec<TypeSpec>,
    pub result: TypeSpec,
    pub doc: Option<String>,
    pub rust_path: Option<String>,
    pub call: CallTemplate,
}

impl FunctionSpec {
    pub fn simple(name: &str, params: Vec<TypeSpec>, result: TypeSpec) -> Self {
        Self {
            name: name.to_string(),
            symbol: format!("otter_{}", name.to_lowercase()),
            params,
            result,
            doc: None,
            rust_path: None,
            call: CallTemplate::Direct,
        }
    }
}

/// Supported primitive value categories for the generated stub.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum TypeSpec {
    Unit,
    Bool,
    I32,
    I64,
    F64,
    Str,
    Opaque,
}

impl TypeSpec {
    pub fn to_rust(&self) -> &'static str {
        match self {
            TypeSpec::Unit => "()",
            TypeSpec::Bool => "bool",
            TypeSpec::I32 => "i32",
            TypeSpec::I64 | TypeSpec::Opaque => "i64",
            TypeSpec::F64 => "f64",
            TypeSpec::Str => "*const ::std::os::raw::c_char",
        }
    }

    pub fn default_return(&self) -> &'static str {
        match self {
            TypeSpec::Unit => "()",
            TypeSpec::Bool => "false",
            TypeSpec::I32 | TypeSpec::I64 | TypeSpec::Opaque => "0",
            TypeSpec::F64 => "0.0",
            TypeSpec::Str => "::std::ptr::null_mut()",
        }
    }

    pub fn ffi_variant(&self) -> &'static str {
        match self {
            TypeSpec::Unit => "FfiType::Unit",
            TypeSpec::Bool => "FfiType::Bool",
            TypeSpec::I32 => "FfiType::I32",
            TypeSpec::I64 => "FfiType::I64",
            TypeSpec::F64 => "FfiType::F64",
            TypeSpec::Str => "FfiType::Str",
            TypeSpec::Opaque => "FfiType::Opaque",
        }
    }
}

/// Source artifacts that comprise the generated stub crate.
#[derive(Clone, Debug)]
pub struct StubSource {
    pub manifest: String,
    pub source: String,
}

#[derive(Clone, Debug)]
pub struct DependencyConfig {
    pub name: String,
    pub version: Option<String>,
    pub path: Option<PathBuf>,
    pub features: Vec<String>,
    pub default_features: bool,
}

impl DependencyConfig {
    pub fn manifest_entry(&self) -> String {
        let mut items = Vec::new();
        if let Some(version) = &self.version {
            items.push(format!("version = \"{}\"", version));
        }
        if let Some(path) = &self.path {
            items.push(format!("path = \"{}\"", path.display()));
        }
        if !self.features.is_empty() {
            let features = self
                .features
                .iter()
                .map(|f| format!("\"{}\"", f))
                .collect::<Vec<_>>()
                .join(", ");
            items.push(format!("features = [{}]", features));
        }
        if !self.default_features {
            items.push("default-features = false".to_string());
        }

        if items.is_empty() {
            "\"*\"".to_string()
        } else {
            format!("{{ {} }}", items.join(", "))
        }
    }

    /// Compute a deterministic hash for caching based on crate name, version, path, and features.
    pub fn cache_hash(&self) -> String {
        use sha1::{Digest, Sha1};
        let mut hasher = Sha1::new();
        hasher.update(self.name.as_bytes());
        if let Some(version) = &self.version {
            hasher.update(b"version:");
            hasher.update(version.as_bytes());
        }
        if let Some(path) = &self.path {
            hasher.update(b"path:");
            hasher.update(path.to_string_lossy().as_bytes());
        }
        // Sort features for deterministic hashing
        let mut features = self.features.clone();
        features.sort();
        for feature in &features {
            hasher.update(b"feature:");
            hasher.update(feature.as_bytes());
        }
        if !self.default_features {
            hasher.update(b"no-default-features");
        }
        format!("{:x}", hasher.finalize())
    }
}

#[derive(Clone, Debug)]
pub struct BridgeMetadata {
    pub crate_name: String,
    pub dependency: DependencyConfig,
    pub functions: Vec<FunctionSpec>,
}

// ===== Transparent Crate Metadata (auto-extracted via rustdoc JSON) =====

/// Normalized representation of a Rust crate's public API for transparent bridging.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrateSpec {
    pub name: String,
    pub version: Option<String>,
    /// Flattened list of all public items with fully-qualified paths
    pub items: Vec<PublicItem>,
}

/// A fully-qualified Rust path represented as segments (e.g., ["chrono","Utc"]).
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
pub struct RustPath {
    pub segments: Vec<String>,
}

impl RustPath {
    pub fn display_dot(&self) -> String {
        self.segments.join(".")
    }
    pub fn display_colon(&self) -> String {
        self.segments.join("::")
    }
}

/// Public API surface normalized for binding generation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PublicItem {
    Function {
        sig: FnSig,
        path: RustPath,
        doc: Option<String>,
    },
    Method {
        impl_for: RustTypeRef,
        sig: FnSig,
        path: RustPath,
        doc: Option<String>,
        /// Whether this is a &self method
        is_instance: bool,
    },
    AssocFunction {
        impl_for: RustTypeRef,
        sig: FnSig,
        path: RustPath,
        doc: Option<String>,
    },
    Const {
        name: String,
        ty: RustTypeRef,
        path: RustPath,
        doc: Option<String>,
        /// String representation of the constant value, if available
        value: Option<String>,
    },
    Static {
        name: String,
        ty: RustTypeRef,
        mutable: bool,
        path: RustPath,
        doc: Option<String>,
    },
    Struct {
        name: String,
        path: RustPath,
        doc: Option<String>,
        /// Fields of the struct (empty for tuple structs or unit structs)
        fields: Vec<StructField>,
        /// Whether this is a tuple struct
        is_tuple: bool,
        /// Type parameters (e.g., ["T", "U"] for struct Foo<T, U>)
        generics: Vec<String>,
    },
    Enum {
        name: String,
        path: RustPath,
        doc: Option<String>,
        /// Variants of the enum
        variants: Vec<EnumVariant>,
        /// Type parameters
        generics: Vec<String>,
    },
    TypeAlias {
        name: String,
        aliased: RustTypeRef,
        path: RustPath,
        doc: Option<String>,
        /// Type parameters
        generics: Vec<String>,
    },
    Module {
        name: String,
        path: RustPath,
        doc: Option<String>,
    },
    Trait {
        name: String,
        path: RustPath,
        doc: Option<String>,
        /// Methods defined in the trait
        methods: Vec<TraitMethod>,
        /// Associated types
        associated_types: Vec<String>,
        /// Type parameters
        generics: Vec<String>,
        /// Whether this trait is unsafe
        is_unsafe: bool,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TraitMethod {
    pub name: String,
    pub sig: FnSig,
    pub has_default_impl: bool,
}

/// Field in a struct
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StructField {
    pub name: String,
    pub ty: RustTypeRef,
    pub doc: Option<String>,
    /// Whether the field is public
    pub is_public: bool,
}

/// Variant in an enum
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnumVariant {
    pub name: String,
    pub doc: Option<String>,
    /// The kind of variant (unit, tuple, struct)
    pub kind: EnumVariantKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "variant_kind")]
pub enum EnumVariantKind {
    /// Unit variant (e.g., `None`)
    Unit,
    /// Tuple variant (e.g., `Some(T)`)
    Tuple { fields: Vec<RustTypeRef> },
    /// Struct variant (e.g., `Point { x: i32, y: i32 }`)
    Struct { fields: Vec<StructField> },
}

/// Function signature (sync or async) with parameter and return types.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FnSig {
    pub name: String,
    pub params: Vec<RustTypeRef>,
    pub return_type: Option<RustTypeRef>,
    #[serde(default)]
    pub is_async: bool,
    /// Type parameters (e.g., ["T", "U"] for fn foo<T, U>())
    #[serde(default)]
    pub generics: Vec<String>,
}

/// Abstract type description sufficient for generating FFI shims and Otter types.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum RustTypeRef {
    Unit,
    Bool,
    I32,
    I64,
    I8,
    I16,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
    Usize,
    Isize,
    F32,
    F64,
    Char,
    Str,
    String,
    /// Fully-qualified nominal type
    Path {
        path: RustPath,
        /// Generic arguments if any (e.g., for HashMap<K, V>)
        #[serde(default)]
        args: Vec<RustTypeRef>,
    },
    /// Reference types
    Ref {
        mutable: bool,
        inner: Box<RustTypeRef>,
        /// Lifetime if specified
        #[serde(default)]
        lifetime: Option<String>,
    },
    /// Owned container types
    Vec {
        elem: Box<RustTypeRef>,
    },
    Slice {
        elem: Box<RustTypeRef>,
    },
    Array {
        elem: Box<RustTypeRef>,
        len: usize,
    },
    Tuple {
        elems: Vec<RustTypeRef>,
    },
    /// Option<T>
    Option {
        inner: Box<RustTypeRef>,
    },
    /// Result<T, E>
    Result {
        ok: Box<RustTypeRef>,
        err: Box<RustTypeRef>,
    },
    /// Future<Output = T>
    Future {
        output: Box<RustTypeRef>,
    },
    /// HashMap<K, V>
    HashMap {
        key: Box<RustTypeRef>,
        value: Box<RustTypeRef>,
    },
    /// HashSet<T>
    HashSet {
        elem: Box<RustTypeRef>,
    },
    /// Box<T>
    Box {
        inner: Box<RustTypeRef>,
    },
    /// Rc<T>
    Rc {
        inner: Box<RustTypeRef>,
    },
    /// Arc<T>
    Arc {
        inner: Box<RustTypeRef>,
    },
    /// Cow<'a, T>
    Cow {
        inner: Box<RustTypeRef>,
        #[serde(default)]
        lifetime: Option<String>,
    },
    /// Function pointer or closure
    Fn {
        params: Vec<RustTypeRef>,
        return_type: Box<RustTypeRef>,
    },
    /// Generic type parameter (e.g., T in fn foo<T>())
    Generic {
        name: String,
    },
    /// Opaque for types we cannot (or don't need to) structurally encode
    Opaque,
}

impl RustTypeRef {
    pub fn is_unit(&self) -> bool {
        matches!(self, RustTypeRef::Unit)
    }
}
