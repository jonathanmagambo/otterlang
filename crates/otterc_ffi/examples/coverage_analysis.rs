#![expect(
    clippy::print_stdout,
    reason = "Printing to stdout is acceptable in examples"
)]

use otterc_ffi::{DependencyConfig, PublicItem, extract_crate_spec};

fn main() {
    println!("=== FFI Extraction Coverage Analysis ===\n");

    // Test with our local test crate
    test_local_crate();

    // Test with real crates
    test_crate("serde", "1.0", vec!["derive"]);
    test_crate("anyhow", "1.0", vec![]);
    test_crate("thiserror", "1.0", vec![]);
}

fn test_local_crate() {
    println!("Testing: test_ffi_simple (local)");

    let test_crate_dir = std::env::temp_dir().join("test_ffi_crate_analysis");
    std::fs::create_dir_all(&test_crate_dir).unwrap();

    let manifest = "[package]
name = \"test_ffi_simple\"
version = \"0.1.0\"
edition = \"2021\"
";

    let lib_rs = "
pub struct Point {
    pub x: f64,
    pub y: f64,
}

pub enum Shape {
    Circle { radius: f64 },
    Rectangle { width: f64, height: f64 },
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }
    
    pub fn distance(&self, other: &Point) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

pub const PI: f64 = 3.14159;

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub mod nested {
    pub fn nested_fn() -> i32 {
        42
    }
}
";

    std::fs::write(test_crate_dir.join("Cargo.toml"), manifest).unwrap();
    std::fs::create_dir_all(test_crate_dir.join("src")).unwrap();
    std::fs::write(test_crate_dir.join("src/lib.rs"), lib_rs).unwrap();

    let dep = DependencyConfig {
        name: "test_ffi_simple".to_string(),
        version: None,
        path: Some(test_crate_dir.clone()),
        features: vec![],
        default_features: true,
    };

    match extract_crate_spec(&dep) {
        Ok(spec) => {
            analyze_extraction(&spec, 7);
        }
        Err(e) => {
            println!("  Failed: {}\n", e);
        }
    }

    let _ = std::fs::remove_dir_all(test_crate_dir);
}

fn test_crate(name: &str, version: &str, features: Vec<&str>) {
    println!("Testing: {} v{}", name, version);

    let dep = DependencyConfig {
        name: name.to_string(),
        version: Some(version.to_string()),
        path: None,
        features: features.iter().map(|s| s.to_string()).collect(),
        default_features: true,
    };

    match extract_crate_spec(&dep) {
        Ok(spec) => {
            analyze_extraction(&spec, 0);
        }
        Err(e) => {
            println!("  Failed: {}", e);
            println!("  (This may be expected if rustc nightly is not installed)\n");
        }
    }
}

fn analyze_extraction(spec: &otterc_ffi::CrateSpec, expected_core_items: usize) {
    let mut counts = std::collections::HashMap::new();
    let mut user_defined_items = 0;

    for item in &spec.items {
        let kind = match item {
            PublicItem::Function { path, .. } => {
                if !is_std_trait_impl(path) {
                    user_defined_items += 1;
                }
                "Function"
            }
            PublicItem::Method { path, .. } => {
                if !is_std_trait_impl(path) {
                    user_defined_items += 1;
                }
                "Method"
            }
            PublicItem::AssocFunction { path, .. } => {
                if !is_std_trait_impl(path) {
                    user_defined_items += 1;
                }
                "AssocFunction"
            }
            PublicItem::Const { .. } => {
                user_defined_items += 1;
                "Const"
            }
            PublicItem::Static { .. } => {
                user_defined_items += 1;
                "Static"
            }
            PublicItem::Struct { .. } => {
                user_defined_items += 1;
                "Struct"
            }
            PublicItem::Enum { .. } => {
                user_defined_items += 1;
                "Enum"
            }
            PublicItem::TypeAlias { .. } => {
                user_defined_items += 1;
                "TypeAlias"
            }
            PublicItem::Module { .. } => "Module",
            PublicItem::Trait { .. } => {
                user_defined_items += 1;
                "Trait"
            }
        };
        *counts.entry(kind).or_insert(0) += 1;
    }

    println!(
        "  Extracted {} total items ({} user-defined)",
        spec.items.len(),
        user_defined_items
    );

    if expected_core_items > 0 {
        let coverage = (user_defined_items as f64 / expected_core_items as f64 * 100.0).min(100.0);
        println!(
            "  Coverage: {:.1}% ({}/{})",
            coverage, user_defined_items, expected_core_items
        );
    }

    println!("  Breakdown:");
    let mut sorted: Vec<_> = counts.iter().collect();
    sorted.sort_by_key(|(k, _)| *k);
    for (kind, count) in sorted {
        println!("     - {}: {}", kind, count);
    }
    println!();
}

fn is_std_trait_impl(path: &otterc_ffi::RustPath) -> bool {
    let path_str = path.display_colon();
    path_str.contains("::From::from")
        || path_str.contains("::Into::into")
        || path_str.contains("::TryFrom::try_from")
        || path_str.contains("::TryInto::try_into")
        || path_str.contains("::Borrow::borrow")
        || path_str.contains("::BorrowMut::borrow_mut")
        || path_str.contains("::AsRef::as_ref")
        || path_str.contains("::AsMut::as_mut")
        || path_str.contains("::Clone::clone")
        || path_str.contains("::Default::default")
        || path_str.contains("::type_id")
}
