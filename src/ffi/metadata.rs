use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;

use super::rust_stubgen::{CallTemplate, FunctionSpec, TypeSpec};

#[derive(Clone, Debug, Deserialize)]
struct RawMetadata {
    #[serde(default)]
    dependency: Option<RawDependency>,
    #[serde(default)]
    functions: Vec<FunctionEntry>,
}

#[derive(Clone, Debug, Deserialize)]
struct RawDependency {
    name: Option<String>,
    version: Option<String>,
    path: Option<String>,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default = "default_true")]
    default_features: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct FunctionEntry {
    /// Canonical OtterLang export name (e.g. "reqwest:get").
    name: String,
    /// Optional symbol override. Defaults to a mangled variant of the export name.
    #[serde(default)]
    symbol: Option<String>,
    /// Fully-qualified Rust path for the function body (e.g. "reqwest::blocking::get").
    #[serde(default)]
    rust_path: Option<String>,
    /// Parameter type identifiers (Unit, Bool, I32, I64, F64, Str).
    #[serde(default)]
    params: Vec<String>,
    /// Return type identifier.
    result: String,
    /// Optional documentation string propagated into the generated stub.
    #[serde(default)]
    doc: Option<String>,
    #[serde(default)]
    call: CallConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
struct CallConfig {
    #[serde(default)]
    kind: CallKind,
    /// Optional expression template using placeholders {0}, {1}, ...
    expr: Option<String>,
}

impl Default for CallConfig {
    fn default() -> Self {
        Self {
            kind: CallKind::Direct,
            expr: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum CallKind {
    Direct,
    Result,
    Expr,
}

impl Default for CallKind {
    fn default() -> Self {
        CallKind::Direct
    }
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
    fn from_raw(crate_name: &str, base_dir: &Path, raw: Option<RawDependency>) -> Self {
        if let Some(raw) = raw {
            let path = raw.path.map(|p| resolve_dependency_path(base_dir, p));
            Self {
                name: raw.name.unwrap_or_else(|| crate_name.to_string()),
                version: raw.version,
                path,
                features: raw.features,
                default_features: raw.default_features,
            }
        } else {
            Self {
                name: crate_name.to_string(),
                version: None,
                path: None,
                features: Vec::new(),
                default_features: true,
            }
        }
    }

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

impl BridgeMetadata {
    fn from_raw(crate_name: &str, raw: RawMetadata) -> Result<Self> {
        let metadata_path = metadata_root().join(crate_name).join("bridge.yaml");
        let base_dir = metadata_path.parent().unwrap_or_else(|| Path::new("."));

        let dependency = DependencyConfig::from_raw(crate_name, base_dir, raw.dependency);
        let functions = raw
            .functions
            .into_iter()
            .map(|entry| entry.try_into_spec(&dependency))
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            crate_name: crate_name.to_string(),
            dependency,
            functions,
        })
    }
}

/// Load the metadata for a given crate from `ffi/<crate>/bridge.yaml`. Absent files
/// resolve to an empty metadata structure to keep the bridge pipeline lenient.
pub fn load_bridge_metadata(crate_name: &str) -> Result<BridgeMetadata> {
    let metadata_path = metadata_root().join(crate_name).join("bridge.yaml");
    if !metadata_path.exists() {
        let base_dir = metadata_root();
        return Ok(BridgeMetadata {
            crate_name: crate_name.to_string(),
            dependency: DependencyConfig::from_raw(crate_name, base_dir.as_path(), None),
            functions: Vec::new(),
        });
    }

    let raw = fs::read_to_string(&metadata_path).with_context(|| {
        format!(
            "failed to read bridge metadata for crate `{crate_name}` at {}",
            metadata_path.display()
        )
    })?;

    let parsed: RawMetadata = serde_yaml::from_str(&raw).with_context(|| {
        format!(
            "failed to parse bridge metadata for crate `{crate_name}` at {}",
            metadata_path.display()
        )
    })?;

    BridgeMetadata::from_raw(crate_name, parsed)
}

impl FunctionEntry {
    fn try_into_spec(self, dependency: &DependencyConfig) -> Result<FunctionSpec> {
        let params = self
            .params
            .iter()
            .map(|ident| {
                parse_type(ident).with_context(|| type_error(&dependency.name, &self.name, ident))
            })
            .collect::<Result<Vec<_>>>()?;
        let result = parse_type(&self.result)
            .with_context(|| type_error(&dependency.name, &self.name, &self.result))?;

        let symbol = self
            .symbol
            .unwrap_or_else(|| default_symbol(&dependency.name, &self.name));

        let call = if let Some(expr) = self.call.expr.clone() {
            CallTemplate::Expr(expr)
        } else {
            match self.call.kind {
                CallKind::Direct => CallTemplate::Direct,
                CallKind::Result => CallTemplate::Result,
                CallKind::Expr => {
                    bail!(
                        "call.kind set to `expr` but no `expr` provided for {}:{}",
                        dependency.name,
                        self.name
                    )
                }
            }
        };

        Ok(FunctionSpec {
            name: self.name,
            symbol,
            params,
            result,
            doc: self.doc,
            rust_path: self.rust_path,
            call,
        })
    }
}

fn parse_type(identifier: &str) -> Result<TypeSpec> {
    match identifier.to_ascii_lowercase().as_str() {
        "unit" | "void" => Ok(TypeSpec::Unit),
        "bool" => Ok(TypeSpec::Bool),
        "i32" | "int32" => Ok(TypeSpec::I32),
        "i64" | "int64" => Ok(TypeSpec::I64),
        "f64" | "float64" | "double" => Ok(TypeSpec::F64),
        "str" | "string" => Ok(TypeSpec::Str),
        "opaque" | "handle" => Ok(TypeSpec::Opaque),
        other => Err(anyhow!(
            "unsupported FFI type identifier `{}` (expected unit, bool, i32, i64, f64, str, or opaque)",
            other
        )),
    }
}

fn default_symbol(crate_name: &str, export_name: &str) -> String {
    let mut base = export_name
        .chars()
        .map(|ch| match ch {
            ':' | '.' => '_',
            other => other,
        })
        .collect::<String>();
    if !base.starts_with(&format!("{crate_name}_")) {
        base = format!("{crate_name}_{base}");
    }
    format!("otter_{base}")
}

fn metadata_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ffi")
}

fn type_error(crate_name: &str, function: &str, ident: &str) -> String {
    format!("unsupported FFI type identifier `{ident}` in {crate_name}:{function}")
}

const fn default_true() -> bool {
    true
}

fn resolve_dependency_path(base_dir: &Path, path: String) -> PathBuf {
    let candidate = PathBuf::from(&path);
    let joined = if candidate.is_absolute() {
        candidate
    } else {
        base_dir.join(candidate)
    };

    joined.canonicalize().unwrap_or(joined)
}

/// Convenience helper retained for older call sites that only require the
/// function list.
pub fn load_bridge_functions(crate_name: &str) -> Result<Vec<FunctionSpec>> {
    Ok(load_bridge_metadata(crate_name)?.functions)
}

// ===== Transparent Crate Metadata (auto-extracted via rustdoc JSON) =====

use serde::{Deserialize as DeDeserialize, Serialize as DeSerialize};

/// Normalized representation of a Rust crate's public API for transparent bridging.
#[derive(Clone, Debug, DeSerialize, DeDeserialize)]
pub struct CrateSpec {
    pub name: String,
    pub version: Option<String>,
    /// Flattened list of all public items with fully-qualified paths
    pub items: Vec<PublicItem>,
}

/// A fully-qualified Rust path represented as segments (e.g., ["chrono","Utc"]).
#[derive(Clone, Debug, DeSerialize, DeDeserialize, Eq, PartialEq, Hash)]
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
#[derive(Clone, Debug, DeSerialize, DeDeserialize)]
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
    },
    Enum {
        name: String,
        path: RustPath,
        doc: Option<String>,
    },
    TypeAlias {
        name: String,
        aliased: RustTypeRef,
        path: RustPath,
        doc: Option<String>,
    },
    Module {
        name: String,
        path: RustPath,
        doc: Option<String>,
    },
}

/// Function signature (sync or async) with parameter and return types.
#[derive(Clone, Debug, DeSerialize, DeDeserialize)]
pub struct FnSig {
    pub name: String,
    pub params: Vec<RustTypeRef>,
    pub return_type: Option<RustTypeRef>,
    #[serde(default)]
    pub is_async: bool,
}

/// Abstract type description sufficient for generating FFI shims and Otter types.
#[derive(Clone, Debug, DeSerialize, DeDeserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum RustTypeRef {
    Unit,
    Bool,
    I32,
    I64,
    F64,
    Str,
    /// Fully-qualified nominal type
    Path {
        path: RustPath,
    },
    /// Reference types
    Ref {
        mutable: bool,
        inner: Box<RustTypeRef>,
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
    /// Opaque for types we cannot (or don't need to) structurally encode
    Opaque,
}

impl RustTypeRef {
    pub fn is_unit(&self) -> bool {
        matches!(self, RustTypeRef::Unit)
    }
}
