use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

use super::types::CrateSpec;
use super::types::DependencyConfig;
use crate::{
    EnumVariant, EnumVariantKind, FnSig, PublicItem, RustPath, RustTypeRef, StructField,
    TraitMethod,
};
use otterc_cache::path::cache_root;

pub fn generate_rustdoc_json(dep: &DependencyConfig) -> Result<PathBuf> {
    let root = match cache_root() {
        Ok(path) => path.join("ffi").join("rustdoc").join(&dep.name),
        Err(_) => return Err(anyhow!("Failed to get cache root")),
    };
    fs::create_dir_all(&root)
        .with_context(|| format!("failed to create rustdoc cache dir {}", root.display()))?;
    let manifest = root.join("Cargo.toml");
    let src_dir = root.join("src");
    let lib_rs = src_dir.join("lib.rs");
    fs::create_dir_all(&src_dir)
        .with_context(|| format!("failed to create {}", src_dir.display()))?;

    let manifest_contents = format!(
        "[package]\nname = \"otter_rustdoc_{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{} = {}\n",
        dep.name,
        dep.name,
        dep.manifest_entry()
    );
    fs::write(&manifest, manifest_contents)
        .with_context(|| format!("failed writing {}", manifest.display()))?;
    fs::write(&lib_rs, "")?;

    let target_dir = root.join("target");
    fs::create_dir_all(&target_dir)?;

    let try_nightly_doc = || {
        duct::cmd(
            "cargo",
            vec![
                "+nightly",
                "doc",
                "-p",
                &dep.name,
                "--manifest-path",
                manifest.to_str().unwrap(),
            ],
        )
        .dir(&root)
        .env("CARGO_TARGET_DIR", &target_dir)
        .env("RUSTDOCFLAGS", "-Z unstable-options --output-format json")
        .run()
    };

    let mut ran = try_nightly_doc();
    if ran.is_err() || !ran.as_ref().unwrap().status.success() {
        ran = duct::cmd(
            "cargo",
            vec![
                "+nightly",
                "rustdoc",
                "-p",
                &dep.name,
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

    let doc_dir = target_dir.join("doc");
    let json_path = fs::read_dir(&doc_dir)
        .with_context(|| format!("failed to read {}", doc_dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.extension().map(|ext| ext == "json").unwrap_or(false)
                && p.file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s == dep.name || s.starts_with(&dep.name))
                    .unwrap_or(false)
        })
        .ok_or_else(|| {
            anyhow!(
                "rustdoc JSON for crate `{}` not found under {}",
                dep.name,
                doc_dir.display()
            )
        })?;

    Ok(json_path)
}

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

pub fn extract_crate_spec(_dep: &DependencyConfig) -> Result<CrateSpec> {
    let json = generate_rustdoc_json(_dep)?;
    extract_crate_spec_from_json(&_dep.name, _dep.version.clone(), &json)
}

#[derive(Debug, Deserialize)]
#[expect(dead_code, reason = "This is a work in process")]
struct Rustdoc {
    index: serde_json::Map<String, serde_json::Value>,
    paths: serde_json::Map<String, serde_json::Value>,
    crate_version: Option<String>,
    #[serde(default)]
    crate_id: Option<usize>,
    root: Option<serde_json::Value>,
    #[serde(default)]
    external_crates: serde_json::Map<String, serde_json::Value>,
}

fn normalize(name: String, version: Option<String>, doc: Rustdoc) -> CrateSpec {
    use std::collections::{HashMap, HashSet};

    let mut items = Vec::new();
    let mut seen = HashSet::new();

    // Build a map of item IDs to their paths for quick lookup
    let mut id_to_path: HashMap<String, Vec<String>> = HashMap::new();
    for (id, path_value) in &doc.paths {
        if let Some(path_array) = path_value
            .as_object()
            .and_then(|path_obj| path_obj.get("path"))
            .and_then(|p| p.as_array())
        {
            let segments: Vec<String> = path_array
                .iter()
                .filter_map(|seg| {
                    if let Some(s) = seg.as_str() {
                        Some(s.to_string())
                    } else if let Some(obj) = seg.as_object() {
                        obj.get("name")?.as_str().map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect();
            if !segments.is_empty() && segments[0] == name {
                id_to_path.insert(id.clone(), segments);
            }
        }
    }

    // Iterate through all items in the index
    for (item_id, item_value) in &doc.index {
        let Some(item_obj) = item_value.as_object() else {
            continue;
        };

        // Get the path for this item
        let (path_segments, has_explicit_path) = match id_to_path.get(item_id) {
            Some(segments) => (segments.clone(), true),
            None => {
                // Try to get from the item itself; fallback to crate-only path for
                // anonymous items like impl blocks.
                if let Some(name_str) = item_obj.get("name").and_then(|n| n.as_str()) {
                    (vec![name.clone(), name_str.to_string()], false)
                } else {
                    (vec![name.clone()], false)
                }
            }
        };

        // Skip if not from our crate
        if path_segments.is_empty() || path_segments[0] != name {
            continue;
        }

        let item_name = item_obj
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_string();

        let doc_comment = item_obj
            .get("docs")
            .and_then(|d| d.as_str())
            .map(|s| s.to_string());

        let path = RustPath {
            segments: path_segments.clone(),
        };

        // Check if deprecated
        if is_deprecated(item_obj) {
            continue;
        }

        // Extract based on item kind
        if let Some(inner) = item_obj.get("inner").and_then(|i| i.as_object()) {
            // Function
            if let Some(_func_obj) = inner.get("function") {
                if let Some(function_item) = extract_function(
                    item_obj,
                    &path_segments,
                    has_explicit_path,
                    &item_name,
                    doc_comment.clone(),
                ) {
                    let key = format!("fn::{}", path.display_colon());
                    if seen.insert(key) {
                        items.push(function_item);
                    }
                }
            }
            // Struct
            else if let Some(struct_obj) = inner.get("struct") {
                if let Some(struct_item) = extract_struct(
                    struct_obj,
                    &doc.index,
                    &path,
                    &item_name,
                    doc_comment.clone(),
                ) {
                    let key = format!("struct::{}", path.display_colon());
                    if seen.insert(key) {
                        items.push(struct_item);
                    }
                }
            }
            // Enum
            else if let Some(enum_obj) = inner.get("enum") {
                if let Some(enum_item) =
                    extract_enum(enum_obj, &path, &item_name, doc_comment.clone(), &doc.index)
                {
                    let key = format!("enum::{}", path.display_colon());
                    if seen.insert(key) {
                        items.push(enum_item);
                    }
                }
            }
            // Constant
            else if let Some(const_obj) = inner.get("constant") {
                if let Some(const_item) =
                    extract_const(const_obj, &path, &item_name, doc_comment.clone())
                {
                    let key = format!("const::{}", path.display_colon());
                    if seen.insert(key) {
                        items.push(const_item);
                    }
                }
            }
            // Static
            else if let Some(static_obj) = inner.get("static") {
                if let Some(static_item) =
                    extract_static(static_obj, &path, &item_name, doc_comment.clone())
                {
                    let key = format!("static::{}", path.display_colon());
                    if seen.insert(key) {
                        items.push(static_item);
                    }
                }
            }
            // Type Alias
            else if let Some(typedef_obj) = inner.get("type_alias") {
                if let Some(typedef_item) =
                    extract_type_alias(typedef_obj, &path, &item_name, doc_comment.clone())
                {
                    let key = format!("type::{}", path.display_colon());
                    if seen.insert(key) {
                        items.push(typedef_item);
                    }
                }
            } else if let Some(trait_obj) = inner.get("trait") {
                if let Some(trait_item) = extract_trait(
                    trait_obj,
                    &doc.index,
                    &path,
                    &item_name,
                    doc_comment.clone(),
                ) {
                    let key = format!("trait::{}", path.display_colon());
                    if seen.insert(key) {
                        items.push(trait_item);
                    }
                }
            } else if let Some(impl_obj) = inner.get("impl") {
                extract_impl_items(
                    impl_obj,
                    &doc.index,
                    &path_segments,
                    &name,
                    &mut items,
                    &mut seen,
                );
            }
            // Module
            else if inner.contains_key("module") {
                let module_item = PublicItem::Module {
                    name: item_name.clone(),
                    path: path.clone(),
                    doc: doc_comment,
                };
                let key = format!("mod::{}", path.display_colon());
                if seen.insert(key) {
                    items.push(module_item);
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

fn extract_function(
    item: &serde_json::Map<String, serde_json::Value>,
    path_segments: &[String],
    has_explicit_path: bool,
    name: &str,
    doc: Option<String>,
) -> Option<PublicItem> {
    // Skip inherent/trait methods – they are handled via impl extraction
    if is_trait_method(item) {
        return None;
    }

    // If rustdoc didn't record a path for this item it usually belongs to an
    // impl block. We'll let impl extraction surface it to avoid duplicates.
    if !has_explicit_path {
        return None;
    }

    let mut segments = path_segments.to_vec();
    if segments.last().map(|s| s != name).unwrap_or(true) {
        segments.push(name.to_string());
    }

    let sig = extract_function_sig(item)?;

    Some(PublicItem::Function {
        sig,
        path: RustPath { segments },
        doc,
    })
}

fn extract_struct(
    struct_obj: &serde_json::Value,
    index: &serde_json::Map<String, serde_json::Value>,
    path: &RustPath,
    name: &str,
    doc: Option<String>,
) -> Option<PublicItem> {
    use serde_json::Value;

    let struct_obj = struct_obj.as_object()?;
    let mut fields = Vec::new();
    let mut generics = Vec::new();
    let mut is_tuple = false;

    // Extract generics
    if let Some(gen_obj) = struct_obj.get("generics").and_then(Value::as_object)
        && let Some(params_arr) = gen_obj.get("params").and_then(Value::as_array)
    {
        for param in params_arr {
            if let Some(param_obj) = param.as_object()
                && let Some(name_val) = param_obj.get("name").and_then(Value::as_str)
            {
                generics.push(name_val.to_string());
            }
        }
    }

    if let Some(kind_obj) = struct_obj.get("kind").and_then(Value::as_object) {
        if let Some(plain_obj) = kind_obj.get("plain").and_then(Value::as_object)
            && let Some(fields_arr) = plain_obj.get("fields").and_then(Value::as_array)
        {
            for field_id in fields_arr {
                if let Some(field_id_str) = normalize_item_id(field_id)
                    && let Some(field_obj) = index.get(&field_id_str).and_then(Value::as_object)
                {
                    fields.push(parse_named_field(field_obj, &generics));
                }
            }
        } else if let Some(tuple_arr) = kind_obj.get("tuple").and_then(Value::as_array) {
            is_tuple = true;
            for (idx, field_id) in tuple_arr.iter().enumerate() {
                if let Some(field_id_str) = normalize_item_id(field_id)
                    && let Some(field_obj) = index.get(&field_id_str).and_then(Value::as_object)
                {
                    fields.push(parse_tuple_field(field_obj, idx, &generics));
                }
            }
        }
    } else if let Some(fields_arr) = struct_obj.get("fields").and_then(Value::as_array) {
        for field_id in fields_arr {
            if let Some(field_id_str) = normalize_item_id(field_id)
                && let Some(field_obj) = index.get(&field_id_str).and_then(Value::as_object)
            {
                fields.push(parse_named_field(field_obj, &generics));
            }
        }
    }

    Some(PublicItem::Struct {
        name: name.to_string(),
        path: path.clone(),
        doc,
        fields,
        is_tuple,
        generics,
    })
}

fn normalize_item_id(value: &serde_json::Value) -> Option<String> {
    if let Some(id_str) = value.as_str() {
        Some(id_str.to_string())
    } else {
        value.as_u64().map(|id| id.to_string())
    }
}

fn parse_named_field(
    field_obj: &serde_json::Map<String, serde_json::Value>,
    generics: &[String],
) -> StructField {
    use serde_json::Value;

    let name = field_obj
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();

    let ty = field_obj
        .get("inner")
        .and_then(Value::as_object)
        .and_then(|inner| {
            inner
                .get("struct_field")
                .or_else(|| inner.get("tuple_field"))
                .and_then(Value::as_object)
        })
        .and_then(|field| field.get("type"))
        .and_then(|ty_val| parse_rust_type(ty_val, generics))
        .unwrap_or(RustTypeRef::Opaque);

    let doc = field_obj
        .get("docs")
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    let is_public = field_obj
        .get("visibility")
        .and_then(Value::as_str)
        .map(|v| v == "public")
        .unwrap_or(false);

    StructField {
        name,
        ty,
        doc,
        is_public,
    }
}

fn parse_tuple_field(
    field_obj: &serde_json::Map<String, serde_json::Value>,
    idx: usize,
    generics: &[String],
) -> StructField {
    use serde_json::Value;

    let ty = field_obj
        .get("inner")
        .and_then(Value::as_object)
        .and_then(|inner| {
            inner
                .get("struct_field")
                .or_else(|| inner.get("tuple_field"))
                .and_then(Value::as_object)
        })
        .and_then(|field| field.get("type"))
        .and_then(|ty_val| parse_rust_type(ty_val, generics))
        .unwrap_or(RustTypeRef::Opaque);

    let doc = field_obj
        .get("docs")
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    let is_public = field_obj
        .get("visibility")
        .and_then(Value::as_str)
        .map(|v| v == "public")
        .unwrap_or(false);

    StructField {
        name: idx.to_string(),
        ty,
        doc,
        is_public,
    }
}

fn extract_enum(
    enum_obj: &serde_json::Value,
    path: &RustPath,
    name: &str,
    doc: Option<String>,
    index: &serde_json::Map<String, serde_json::Value>,
) -> Option<PublicItem> {
    use serde_json::Value;

    let enum_obj = enum_obj.as_object()?;
    let mut variants = Vec::new();
    let mut generics = Vec::new();

    // Extract generics
    if let Some(gen_obj) = enum_obj.get("generics").and_then(Value::as_object)
        && let Some(params_arr) = gen_obj.get("params").and_then(Value::as_array)
    {
        for param in params_arr {
            if let Some(param_obj) = param.as_object()
                && let Some(name_val) = param_obj.get("name").and_then(Value::as_str)
            {
                generics.push(name_val.to_string());
            }
        }
    }

    // Extract variants
    if let Some(variants_arr) = enum_obj.get("variants").and_then(Value::as_array) {
        for variant_id in variants_arr {
            if let Some(variant) = variant_id
                .as_str()
                .and_then(|id| index.get(id))
                .and_then(Value::as_object)
                .and_then(|variant_obj| extract_enum_variant(variant_obj, &generics))
            {
                variants.push(variant);
            }
        }
    }

    Some(PublicItem::Enum {
        name: name.to_string(),
        path: path.clone(),
        doc,
        variants,
        generics,
    })
}

fn extract_enum_variant(
    variant_obj: &serde_json::Map<String, serde_json::Value>,
    _generics: &[String],
) -> Option<EnumVariant> {
    use serde_json::Value;

    let name = variant_obj.get("name")?.as_str()?.to_string();
    let doc = variant_obj
        .get("docs")
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    let kind = if let Some(inner) = variant_obj.get("inner").and_then(Value::as_object)
        && let Some(variant_inner) = inner.get("variant").and_then(Value::as_object)
    {
        if let Some(kind_str) = variant_inner.get("kind").and_then(Value::as_str) {
            match kind_str {
                "tuple" => {
                    let mut fields = Vec::new();
                    if let Some(fields_arr) = variant_inner.get("fields").and_then(Value::as_array)
                    {
                        for _field in fields_arr {
                            // Would need to parse field types
                            fields.push(RustTypeRef::Opaque);
                        }
                    }
                    EnumVariantKind::Tuple { fields }
                }
                "struct" => {
                    let fields = Vec::new();
                    // Would need to parse struct fields
                    EnumVariantKind::Struct { fields }
                }
                _ => EnumVariantKind::Unit,
            }
        } else {
            EnumVariantKind::Unit
        }
    } else {
        EnumVariantKind::Unit
    };

    Some(EnumVariant { name, doc, kind })
}

fn extract_const(
    const_obj: &serde_json::Value,
    path: &RustPath,
    name: &str,
    doc: Option<String>,
) -> Option<PublicItem> {
    use serde_json::Value;

    let const_obj = const_obj.as_object()?;
    let ty = if let Some(type_val) = const_obj.get("type") {
        parse_rust_type(type_val, &[]).unwrap_or(RustTypeRef::Opaque)
    } else {
        RustTypeRef::Opaque
    };

    let value = const_obj
        .get("const")
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    Some(PublicItem::Const {
        name: name.to_string(),
        ty,
        path: path.clone(),
        doc,
        value,
    })
}

fn extract_static(
    static_obj: &serde_json::Value,
    path: &RustPath,
    name: &str,
    doc: Option<String>,
) -> Option<PublicItem> {
    use serde_json::Value;

    let static_obj = static_obj.as_object()?;
    let ty = if let Some(type_val) = static_obj.get("type") {
        parse_rust_type(type_val, &[]).unwrap_or(RustTypeRef::Opaque)
    } else {
        RustTypeRef::Opaque
    };

    let mutable = static_obj
        .get("mutable")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Some(PublicItem::Static {
        name: name.to_string(),
        ty,
        mutable,
        path: path.clone(),
        doc,
    })
}

fn extract_type_alias(
    typedef_obj: &serde_json::Value,
    path: &RustPath,
    name: &str,
    doc: Option<String>,
) -> Option<PublicItem> {
    use serde_json::Value;

    let typedef_obj = typedef_obj.as_object()?;
    let mut generics = Vec::new();

    // Extract generics
    if let Some(gen_obj) = typedef_obj.get("generics").and_then(Value::as_object)
        && let Some(params_arr) = gen_obj.get("params").and_then(Value::as_array)
    {
        for param in params_arr {
            if let Some(param_obj) = param.as_object()
                && let Some(name_val) = param_obj.get("name").and_then(Value::as_str)
            {
                generics.push(name_val.to_string());
            }
        }
    }

    let aliased = if let Some(type_val) = typedef_obj.get("type") {
        parse_rust_type(type_val, &generics).unwrap_or(RustTypeRef::Opaque)
    } else {
        RustTypeRef::Opaque
    };

    Some(PublicItem::TypeAlias {
        name: name.to_string(),
        aliased,
        path: path.clone(),
        doc,
        generics,
    })
}

fn extract_trait(
    trait_obj: &serde_json::Value,
    index: &serde_json::Map<String, serde_json::Value>,
    path: &RustPath,
    name: &str,
    doc: Option<String>,
) -> Option<PublicItem> {
    use serde_json::Value;

    let trait_obj = trait_obj.as_object()?;
    let mut generics = Vec::new();
    let mut methods = Vec::new();
    let mut associated_types = Vec::new();

    if let Some(gen_obj) = trait_obj.get("generics").and_then(Value::as_object)
        && let Some(params_arr) = gen_obj.get("params").and_then(Value::as_array)
    {
        for param in params_arr {
            if let Some(param_obj) = param.as_object()
                && let Some(name_val) = param_obj.get("name").and_then(Value::as_str)
            {
                if param_obj.get("kind").and_then(Value::as_str) == Some("type") {
                    associated_types.push(name_val.to_string());
                } else {
                    generics.push(name_val.to_string());
                }
            }
        }
    }

    if let Some(items_arr) = trait_obj.get("items").and_then(Value::as_array) {
        for item_id in items_arr {
            if let Some(item_id_str) = item_id.as_str()
                && let Some(item_obj) = index.get(item_id_str).and_then(Value::as_object)
                && let Some(inner) = item_obj.get("inner").and_then(Value::as_object)
            {
                if inner.contains_key("function")
                    && let Some(sig) = extract_function_sig(item_obj)
                {
                    let has_default_impl = inner
                        .get("function")
                        .and_then(Value::as_object)
                        .and_then(|f| f.get("has_body"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false);

                    methods.push(TraitMethod {
                        name: sig.name.clone(),
                        sig,
                        has_default_impl,
                    });
                } else if inner.contains_key("assoc_type")
                    && let Some(type_name) = item_obj.get("name").and_then(Value::as_str)
                {
                    associated_types.push(type_name.to_string());
                }
            }
        }
    }

    let is_unsafe = trait_obj
        .get("is_unsafe")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Some(PublicItem::Trait {
        name: name.to_string(),
        path: path.clone(),
        doc,
        methods,
        associated_types,
        generics,
        is_unsafe,
    })
}

fn extract_impl_items(
    impl_obj: &serde_json::Value,
    index: &serde_json::Map<String, serde_json::Value>,
    _path_segments: &[String],
    crate_name: &str,
    items: &mut Vec<PublicItem>,
    seen: &mut std::collections::HashSet<String>,
) {
    use serde_json::Value;

    let Some(impl_obj) = impl_obj.as_object() else {
        return;
    };

    // Get the type this impl is for
    let Some(for_val) = impl_obj.get("for") else {
        return;
    };
    let impl_for = parse_rust_type(for_val, &[]).unwrap_or(RustTypeRef::Opaque);

    // Extract items from the impl block
    if let Some(items_arr) = impl_obj.get("items").and_then(Value::as_array) {
        for item_id in items_arr {
            let lookup_id = item_id
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| item_id.as_u64().map(|id| id.to_string()));

            if let Some(item_id_str) = lookup_id
                && let Some(item_obj) = index.get(&item_id_str).and_then(Value::as_object)
            {
                // Extract method or associated function
                if let Some(inner) = item_obj.get("inner").and_then(Value::as_object)
                    && inner.contains_key("function")
                {
                    let item_name = item_obj
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();

                    let doc = item_obj
                        .get("docs")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string());

                    // Check if it's a method (has self parameter)
                    let is_method = is_method_with_self(item_obj);

                    // Build a path that reflects the impl target when available
                    let mut path_segments = impl_target_segments(for_val, crate_name)
                        .unwrap_or_else(|| match &impl_for {
                            RustTypeRef::Path { path, .. } => {
                                if path.segments.first().is_some_and(|seg| seg == crate_name) {
                                    path.segments.clone()
                                } else {
                                    let mut with_crate = vec![crate_name.to_string()];
                                    with_crate.extend(path.segments.clone());
                                    with_crate
                                }
                            }
                            _ => vec![crate_name.to_string()],
                        });
                    path_segments.push(item_name.clone());
                    let path = RustPath {
                        segments: path_segments,
                    };

                    if let Some(sig) = extract_function_sig(item_obj) {
                        let key = if is_method {
                            format!("method::{}::{}", impl_for_key(&impl_for), item_name)
                        } else {
                            format!("assoc_fn::{}::{}", impl_for_key(&impl_for), item_name)
                        };

                        if seen.insert(key) {
                            if is_method {
                                items.push(PublicItem::Method {
                                    impl_for: impl_for.clone(),
                                    sig,
                                    path,
                                    doc,
                                    is_instance: true,
                                });
                            } else {
                                items.push(PublicItem::AssocFunction {
                                    impl_for: impl_for.clone(),
                                    sig,
                                    path,
                                    doc,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

fn extract_function_sig(item_obj: &serde_json::Map<String, serde_json::Value>) -> Option<FnSig> {
    use serde_json::Value;

    let inner = item_obj.get("inner").and_then(Value::as_object)?;
    let func = inner.get("function").and_then(Value::as_object)?;

    let mut generics = Vec::new();
    if let Some(gen_obj) = func.get("generics").and_then(Value::as_object)
        && let Some(params_arr) = gen_obj.get("params").and_then(Value::as_array)
    {
        for param in params_arr {
            if let Some(param_obj) = param.as_object()
                && let Some(name_val) = param_obj.get("name").and_then(Value::as_str)
            {
                generics.push(name_val.to_string());
            }
        }
    }

    let sig = func.get("sig").and_then(Value::as_object)?;

    // Extract parameters using the modern rustdoc JSON layout where each
    // argument is represented as [pattern, type].
    let mut params = Vec::new();
    if let Some(inputs) = sig.get("inputs").and_then(Value::as_array) {
        for input in inputs {
            let ty_value = match input {
                Value::Array(arr) if arr.len() == 2 => arr.get(1),
                Value::Object(obj) => obj.get("type"),
                _ => None,
            };

            if let Some(ty) = ty_value {
                params.push(parse_rust_type(ty, &generics).unwrap_or(RustTypeRef::Opaque));
            } else {
                params.push(RustTypeRef::Opaque);
            }
        }
    }

    let return_type = match sig.get("output") {
        Some(Value::Null) | None => None,
        Some(output) => Some(parse_rust_type(output, &generics).unwrap_or(RustTypeRef::Opaque)),
    };

    let is_async = func
        .get("header")
        .and_then(Value::as_object)
        .and_then(|hdr| hdr.get("is_async"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let name = item_obj
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    Some(FnSig {
        name,
        params,
        return_type,
        is_async,
        generics,
    })
}

fn impl_target_segments(for_value: &serde_json::Value, crate_name: &str) -> Option<Vec<String>> {
    use serde_json::Value;

    let obj = for_value.as_object()?;
    let resolved = obj.get("resolved_path").and_then(Value::as_object)?;

    let mut segments = if let Some(path_str) = resolved.get("path").and_then(Value::as_str) {
        path_str
            .split("::")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    } else if let Some(name) = resolved.get("name").and_then(Value::as_str) {
        vec![name.to_string()]
    } else {
        Vec::new()
    };

    if segments.is_empty() {
        segments.push(crate_name.to_string());
    } else if segments.first().is_none_or(|seg| seg != crate_name) {
        let mut with_crate = Vec::with_capacity(segments.len() + 1);
        with_crate.push(crate_name.to_string());
        with_crate.extend(segments);
        segments = with_crate;
    }

    Some(segments)
}

fn impl_for_key(ty: &RustTypeRef) -> String {
    match ty {
        RustTypeRef::Path { path, .. } => path.display_colon(),
        _ => "unknown".to_string(),
    }
}

fn is_method_with_self(item: &serde_json::Map<String, serde_json::Value>) -> bool {
    use serde_json::Value;

    if let Some(inner) = item.get("inner").and_then(Value::as_object)
        && let Some(func) = inner.get("function").and_then(Value::as_object)
        && let Some(sig) = func.get("sig").and_then(Value::as_object)
        && let Some(inputs) = sig.get("inputs").and_then(Value::as_array)
        && let Some(first_input) = inputs.first()
    {
        let name = match first_input {
            Value::Array(arr) => arr.first().and_then(Value::as_str),
            Value::Object(obj) => obj.get("name").and_then(Value::as_str),
            _ => None,
        };

        if let Some(name) = name {
            let mut ident = name.trim();
            if let Some(stripped) = ident.strip_prefix('&') {
                ident = stripped.trim();
            }
            if let Some(stripped) = ident.strip_prefix("mut ") {
                ident = stripped.trim();
            }
            return ident == "self";
        }
    }

    false
}

fn is_trait_method(item: &serde_json::Map<String, serde_json::Value>) -> bool {
    is_method_with_self(item)
}

fn is_deprecated(item: &serde_json::Map<String, serde_json::Value>) -> bool {
    use serde_json::Value;

    item.get("attrs")
        .and_then(Value::as_array)
        .is_some_and(|attrs| {
            attrs.iter().filter_map(Value::as_object).any(|attr_obj| {
                if let Some(attr_str) = attr_obj.get("value").and_then(|v| v.as_str()) {
                    return attr_str.contains("deprecated");
                }

                if let Some(attr_str) = attr_obj
                    .get("kind")
                    .and_then(|k| k.as_object())
                    .and_then(|k| k.get("kind"))
                    .and_then(|k| k.as_str())
                {
                    return attr_str == "deprecated";
                }

                false
            })
        })
}

fn parse_rust_type(ty_value: &serde_json::Value, generics: &[String]) -> Option<RustTypeRef> {
    use serde_json::Value;

    // Handle string types (primitive type names)
    if let Some(ty_str) = ty_value.as_str() {
        return match ty_str {
            "()" => Some(RustTypeRef::Unit),
            "bool" => Some(RustTypeRef::Bool),
            "i8" => Some(RustTypeRef::I8),
            "i16" => Some(RustTypeRef::I16),
            "i32" => Some(RustTypeRef::I32),
            "i64" => Some(RustTypeRef::I64),
            "i128" => Some(RustTypeRef::I128),
            "u8" => Some(RustTypeRef::U8),
            "u16" => Some(RustTypeRef::U16),
            "u32" => Some(RustTypeRef::U32),
            "u64" => Some(RustTypeRef::U64),
            "u128" => Some(RustTypeRef::U128),
            "usize" => Some(RustTypeRef::Usize),
            "isize" => Some(RustTypeRef::Isize),
            "f32" => Some(RustTypeRef::F32),
            "f64" => Some(RustTypeRef::F64),
            "char" => Some(RustTypeRef::Char),
            "&str" | "str" => Some(RustTypeRef::Str),
            "String" => Some(RustTypeRef::String),
            _ => {
                // Check if it's a generic parameter
                if generics.contains(&ty_str.to_string()) {
                    Some(RustTypeRef::Generic {
                        name: ty_str.to_string(),
                    })
                } else {
                    // Unknown primitive or path
                    Some(RustTypeRef::Path {
                        path: RustPath {
                            segments: vec![ty_str.to_string()],
                        },
                        args: Vec::new(),
                    })
                }
            }
        };
    }

    // Handle complex types (objects)
    let obj = ty_value.as_object()?;

    // Check for different type kinds
    if let Some(kind) = obj.get("kind").and_then(Value::as_str) {
        match kind {
            "resolved_path" => {
                // This is a named type with a path
                let path = if let Some(name) = obj.get("name").and_then(Value::as_str) {
                    RustPath {
                        segments: vec![name.to_string()],
                    }
                } else if let Some(path_obj) = obj.get("path").and_then(Value::as_object)
                    && let Some(segments_arr) = path_obj.get("segments").and_then(Value::as_array)
                {
                    RustPath {
                        segments: segments_arr
                            .iter()
                            .filter_map(|s| s.as_str().map(|s| s.to_string()))
                            .collect(),
                    }
                } else {
                    return Some(RustTypeRef::Opaque);
                };

                // Extract generic arguments
                let mut args = Vec::new();
                if let Some(args_obj) = obj.get("args").and_then(Value::as_object)
                    && let Some(angle_bracketed) =
                        args_obj.get("angle_bracketed").and_then(Value::as_object)
                    && let Some(args_arr) = angle_bracketed.get("args").and_then(Value::as_array)
                {
                    for arg in args_arr {
                        if let Some(arg_obj) = arg.as_object()
                            && let Some(type_val) = arg_obj.get("type")
                        {
                            args.push(
                                parse_rust_type(type_val, generics).unwrap_or(RustTypeRef::Opaque),
                            );
                        }
                    }
                }

                // Check for special types
                let name = path.segments.last().map(|s| s.as_str()).unwrap_or("");
                match name {
                    "Option" if args.len() == 1 => {
                        return Some(RustTypeRef::Option {
                            inner: Box::new(args.into_iter().next().unwrap()),
                        });
                    }
                    "Result" if args.len() == 2 => {
                        let mut iter = args.into_iter();
                        return Some(RustTypeRef::Result {
                            ok: Box::new(iter.next().unwrap()),
                            err: Box::new(iter.next().unwrap()),
                        });
                    }
                    "Vec" if args.len() == 1 => {
                        return Some(RustTypeRef::Vec {
                            elem: Box::new(args.into_iter().next().unwrap()),
                        });
                    }
                    "Box" if args.len() == 1 => {
                        return Some(RustTypeRef::Box {
                            inner: Box::new(args.into_iter().next().unwrap()),
                        });
                    }
                    "Arc" if args.len() == 1 => {
                        return Some(RustTypeRef::Arc {
                            inner: Box::new(args.into_iter().next().unwrap()),
                        });
                    }
                    "Rc" if args.len() == 1 => {
                        return Some(RustTypeRef::Rc {
                            inner: Box::new(args.into_iter().next().unwrap()),
                        });
                    }
                    "HashMap" if args.len() == 2 => {
                        let mut iter = args.into_iter();
                        return Some(RustTypeRef::HashMap {
                            key: Box::new(iter.next().unwrap()),
                            value: Box::new(iter.next().unwrap()),
                        });
                    }
                    "HashSet" if args.len() == 1 => {
                        return Some(RustTypeRef::HashSet {
                            elem: Box::new(args.into_iter().next().unwrap()),
                        });
                    }
                    _ => {}
                }

                Some(RustTypeRef::Path { path, args })
            }
            "borrowed_ref" => {
                let mutable = obj.get("mutable").and_then(Value::as_bool).unwrap_or(false);
                let inner = if let Some(type_val) = obj.get("type") {
                    Box::new(parse_rust_type(type_val, generics).unwrap_or(RustTypeRef::Opaque))
                } else {
                    Box::new(RustTypeRef::Opaque)
                };
                let lifetime = obj
                    .get("lifetime")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string());

                Some(RustTypeRef::Ref {
                    mutable,
                    inner,
                    lifetime,
                })
            }
            "slice" => {
                let elem = if let Some(type_val) = obj.get("inner") {
                    Box::new(parse_rust_type(type_val, generics).unwrap_or(RustTypeRef::Opaque))
                } else {
                    Box::new(RustTypeRef::Opaque)
                };
                Some(RustTypeRef::Slice { elem })
            }
            "array" => {
                let elem = if let Some(type_val) = obj.get("type") {
                    Box::new(parse_rust_type(type_val, generics).unwrap_or(RustTypeRef::Opaque))
                } else {
                    Box::new(RustTypeRef::Opaque)
                };
                let len = obj
                    .get("len")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                Some(RustTypeRef::Array { elem, len })
            }
            "tuple" => {
                let mut elems = Vec::new();
                if let Some(elems_arr) = obj.get("inner").and_then(Value::as_array) {
                    for elem_val in elems_arr {
                        elems.push(
                            parse_rust_type(elem_val, generics).unwrap_or(RustTypeRef::Opaque),
                        );
                    }
                }
                Some(RustTypeRef::Tuple { elems })
            }
            "generic" => {
                if let Some(name) = obj.get("name").and_then(Value::as_str) {
                    Some(RustTypeRef::Generic {
                        name: name.to_string(),
                    })
                } else {
                    Some(RustTypeRef::Opaque)
                }
            }
            "primitive" => {
                if let Some(name) = obj.get("name").and_then(Value::as_str) {
                    parse_rust_type(&Value::String(name.to_string()), generics)
                } else {
                    Some(RustTypeRef::Opaque)
                }
            }
            _ => Some(RustTypeRef::Opaque),
        }
    } else {
        // Fallback for types without a kind field
        Some(RustTypeRef::Opaque)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn impl_target_segments_prefixes_crate_when_missing() {
        let value = json!({
            "resolved_path": {
                "path": "outer::Inner"
            }
        });

        let segments = impl_target_segments(&value, "my_crate").expect("segments");
        assert_eq!(segments, vec!["my_crate", "outer", "Inner"]);
    }

    #[test]
    fn impl_target_segments_preserves_existing_crate() {
        let value = json!({
            "resolved_path": {
                "path": "my_crate::Thing"
            }
        });

        let segments = impl_target_segments(&value, "my_crate").expect("segments");
        assert_eq!(segments, vec!["my_crate", "Thing"]);
    }
}
