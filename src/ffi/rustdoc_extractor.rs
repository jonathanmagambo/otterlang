use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use super::metadata::DependencyConfig;
use super::metadata::{CrateSpec, FnSig, PublicItem, RustPath, RustTypeRef};
use crate::cache::path::cache_root;

/// Generate rustdoc JSON for a dependency crate by creating a minimal cargo
/// project and invoking cargo doc with JSON output enabled. The resulting JSON
/// file path is returned.
///
/// Notes:
/// - Prefers stable where possible; falls back to nightly flags when required.
/// - Uses a per-crate cache directory under ~/.otter_cache/ffi/rustdoc/<crate>.
pub fn generate_rustdoc_json(dep: &DependencyConfig) -> Result<PathBuf> {
    let root = cache_root()?.join("ffi").join("rustdoc").join(&dep.name);
    fs::create_dir_all(&root)
        .with_context(|| format!("failed to create rustdoc cache dir {}", root.display()))?;
    let manifest = root.join("Cargo.toml");
    let src_dir = root.join("src");
    let lib_rs = src_dir.join("lib.rs");
    fs::create_dir_all(&src_dir)
        .with_context(|| format!("failed to create {}", src_dir.display()))?;

    // Minimal crate that depends on the target dep.
    let manifest_contents = format!(
        "[package]\nname = \"otter_rustdoc_{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{} = {}\n",
        dep.name,
        dep.name,
        dep.manifest_entry()
    );
    fs::write(&manifest, manifest_contents)
        .with_context(|| format!("failed writing {}", manifest.display()))?;
    fs::write(&lib_rs, "")?;

    // Target path where cargo places rustdoc JSON (nightly-only flag). We'll attempt both forms.
    let target_dir = root.join("target");
    fs::create_dir_all(&target_dir)?;

    // Try stable-ish form first (some distributions permit output-format via RUSTDOCFLAGS)
    let try_stable = || {
        let cmd = duct::cmd(
            "cargo",
            vec![
                "doc",
                "--no-deps",
                "--manifest-path",
                manifest.to_str().unwrap(),
            ],
        )
        .dir(&root)
        .env("CARGO_TARGET_DIR", &target_dir)
        .env("RUSTDOCFLAGS", "-Z unstable-options --output-format json")
        .run();
        cmd
    };

    let mut ran = try_stable();
    if ran.is_err() || !ran.as_ref().unwrap().status.success() {
        // Fall back to explicit rustdoc invocation via cargo rustdoc
        ran = duct::cmd(
            "cargo",
            vec![
                "rustdoc",
                "--no-deps",
                "--manifest-path",
                manifest.to_str().unwrap(),
                "--",
                "-Z",
                "unstable-options",
                "--output-format",
                "json",
            ],
        )
        .dir(&root)
        .env("CARGO_TARGET_DIR", &target_dir)
        .run();
    }

    if ran.is_err() || !ran.as_ref().unwrap().status.success() {
        return Err(anyhow!("failed to produce rustdoc JSON for `{}`", dep.name));
    }

    // Heuristic: locate the single .json file under target/doc
    let doc_dir = target_dir.join("doc");
    let json_path = fs::read_dir(&doc_dir)
        .with_context(|| format!("failed to read {}", doc_dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().map(|ext| ext == "json").unwrap_or(false))
        .ok_or_else(|| anyhow!("rustdoc JSON not found under {}", doc_dir.display()))?;

    Ok(json_path)
}

/// Extract a CrateSpec for a dependency using a rustdoc JSON file already produced.
/// In a later step, this will run `cargo doc -Z unstable-options --output-format json` and
/// point to the generated file; for now we accept a path for testability and to stage integration.
pub fn extract_crate_spec_from_json(
    crate_name: &str,
    version: Option<String>,
    rustdoc_json_path: &Path,
) -> Result<CrateSpec> {
    let data = fs::read_to_string(rustdoc_json_path).with_context(|| {
        format!(
            "failed to read rustdoc JSON from {}",
            rustdoc_json_path.display()
        )
    })?;
    let doc: Rustdoc = serde_json::from_str(&data).with_context(|| {
        format!(
            "failed to parse rustdoc JSON at {}",
            rustdoc_json_path.display()
        )
    })?;
    Ok(normalize(crate_name.to_string(), version, doc))
}

/// Placeholder entry to allow wiring without invoking cargo yet.
#[allow(unused_variables)]
pub fn extract_crate_spec(_dep: &DependencyConfig) -> Result<CrateSpec> {
    let json = generate_rustdoc_json(_dep)?;
    extract_crate_spec_from_json(&_dep.name, _dep.version.clone(), &json)
}

// --- Minimal rustdoc JSON façade (subset) ---

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Rustdoc {
    index: serde_json::Map<String, serde_json::Value>,
    paths: serde_json::Map<String, serde_json::Value>,
    crate_version: Option<String>,
    crate_id: usize,
}

fn normalize(name: String, version: Option<String>, doc: Rustdoc) -> CrateSpec {
    let mut items = Vec::new();

    // Extract public functions from rustdoc index
    for (_item_id, item_value) in &doc.index {
        if let Some(item_obj) = item_value.as_object() {
            if let Some(kind) = item_obj.get("kind").and_then(|k| k.as_str()) {
                if kind == "function" {
                    if let Some(path_segments) = extract_path(item_obj) {
                        if let Some(function_item) = extract_function(item_obj, &path_segments) {
                            items.push(function_item);
                        }
                    }
                }
            }
        }
    }

    CrateSpec {
        name,
        version,
        items,
    }
}

fn extract_path(item: &serde_json::Map<String, serde_json::Value>) -> Option<Vec<String>> {
    item.get("path")?
        .as_object()?
        .get("segments")?
        .as_array()?
        .iter()
        .filter_map(|seg| {
            seg.as_object()?
                .get("name")?
                .as_str()
                .map(|s| s.to_string())
        })
        .collect::<Vec<_>>()
        .into()
}

fn extract_function(
    item: &serde_json::Map<String, serde_json::Value>,
    path_segments: &[String],
) -> Option<PublicItem> {
    let name = item.get("name")?.as_str()?.to_string();

    // Extract signature (simplified - just count params for now)
    let mut params = Vec::new();
    let return_type = if let Some(sig) = item.get("sig").and_then(|s| s.as_object()) {
        // Try to extract params
        if let Some(decl) = sig.get("decl").and_then(|d| d.as_object()) {
            if let Some(inputs) = decl.get("inputs").and_then(|i| i.as_array()) {
                for input in inputs {
                    // Try to parse each input type
                    if let Some(input_obj) = input.as_object() {
                        if let Some(ty) = input_obj.get("type") {
                            params.push(parse_rust_type(ty).unwrap_or(RustTypeRef::Opaque));
                        } else {
                            params.push(RustTypeRef::Opaque);
                        }
                    } else {
                        params.push(RustTypeRef::Opaque);
                    }
                }
            }
            // Try to extract return type
            let ret_ty = if let Some(output) = decl.get("output") {
                parse_rust_type(output).unwrap_or(RustTypeRef::Opaque)
            } else {
                RustTypeRef::Unit
            };
            Some(ret_ty)
        } else {
            None
        }
    } else {
        None
    };

    // Check if async
    let is_async = item
        .get("sig")
        .and_then(|s| s.as_object())
        .and_then(|s| s.get("asyncness"))
        .is_some();

    let doc = item
        .get("docs")
        .and_then(|d| d.as_str())
        .map(|s| s.to_string());

    let path = RustPath {
        segments: path_segments.to_vec(),
    };

    Some(PublicItem::Function {
        sig: FnSig {
            name,
            params,
            return_type,
            is_async,
        },
        path,
        doc,
    })
}

fn parse_rust_type(ty_value: &serde_json::Value) -> Option<RustTypeRef> {
    // Basic type parsing - check for common primitives in the type string
    if let Some(ty_str) = ty_value.as_str() {
        match ty_str {
            "()" => return Some(RustTypeRef::Unit),
            "bool" => return Some(RustTypeRef::Bool),
            "i32" => return Some(RustTypeRef::I32),
            "i64" => return Some(RustTypeRef::I64),
            "f64" => return Some(RustTypeRef::F64),
            "f32" => return Some(RustTypeRef::F64), // Promote f32 to f64
            "&str" | "str" | "String" => return Some(RustTypeRef::Str),
            _ => {}
        }
    }

    // Check for Option<T>
    if let Some(obj) = ty_value.as_object() {
        if let Some(ty_name) = obj.get("name").and_then(|n| n.as_str()) {
            if ty_name == "Option" {
                if let Some(inner) = obj
                    .get("inner")
                    .and_then(|i| i.as_array())
                    .and_then(|a| a.first())
                {
                    return Some(RustTypeRef::Option {
                        inner: Box::new(parse_rust_type(inner).unwrap_or(RustTypeRef::Opaque)),
                    });
                }
            }
            if ty_name == "Result" {
                if let Some(inner) = obj.get("inner").and_then(|i| i.as_array()) {
                    let ok = inner
                        .get(0)
                        .and_then(|t| parse_rust_type(t))
                        .unwrap_or(RustTypeRef::Opaque);
                    let err = inner
                        .get(1)
                        .and_then(|t| parse_rust_type(t))
                        .unwrap_or(RustTypeRef::Opaque);
                    return Some(RustTypeRef::Result {
                        ok: Box::new(ok),
                        err: Box::new(err),
                    });
                }
            }
            if ty_name == "Future" {
                if let Some(inner) = obj
                    .get("inner")
                    .and_then(|i| i.as_array())
                    .and_then(|a| a.first())
                {
                    return Some(RustTypeRef::Future {
                        output: Box::new(parse_rust_type(inner).unwrap_or(RustTypeRef::Opaque)),
                    });
                }
            }
        }
    }

    Some(RustTypeRef::Opaque)
}

// --- Helpers to construct normalized nodes (used by the real normalizer later) ---

#[allow(dead_code)]
fn fq_path(parts: &[&str]) -> RustPath {
    RustPath {
        segments: parts.iter().map(|s| s.to_string()).collect(),
    }
}

#[allow(dead_code)]
fn fn_item(
    path: RustPath,
    name: &str,
    params: Vec<RustTypeRef>,
    return_type: Option<RustTypeRef>,
    is_async: bool,
    doc: Option<String>,
) -> PublicItem {
    PublicItem::Function {
        sig: FnSig {
            name: name.to_string(),
            params,
            return_type,
            is_async,
        },
        path,
        doc,
    }
}
