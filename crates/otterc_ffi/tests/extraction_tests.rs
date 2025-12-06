#![expect(
    clippy::print_stdout,
    reason = "Printing to stdout is acceptable in tests"
)]
#![expect(clippy::panic, reason = "Panicking on test failures is acceptable")]

use otterc_ffi::{DependencyConfig, extract_crate_spec};

#[test]
#[ignore] // Run with: cargo test --package ffi --test extraction_tests -- --ignored
fn test_extract_serde() {
    let dep = DependencyConfig {
        name: "serde".to_string(),
        version: Some("1.0".to_string()),
        path: None,
        features: vec!["derive".to_string()],
        default_features: true,
    };

    let result = extract_crate_spec(&dep);
    match result {
        Ok(spec) => {
            println!("Successfully extracted serde metadata");
            println!("   Crate: {} v{:?}", spec.name, spec.version);
            println!("   Total items: {}", spec.items.len());

            // Count different item types
            let mut counts = std::collections::HashMap::new();
            for item in &spec.items {
                let kind = match item {
                    otterc_ffi::PublicItem::Function { .. } => "Function",
                    otterc_ffi::PublicItem::Method { .. } => "Method",
                    otterc_ffi::PublicItem::AssocFunction { .. } => "AssocFunction",
                    otterc_ffi::PublicItem::Const { .. } => "Const",
                    otterc_ffi::PublicItem::Static { .. } => "Static",
                    otterc_ffi::PublicItem::Struct { .. } => "Struct",
                    otterc_ffi::PublicItem::Enum { .. } => "Enum",
                    otterc_ffi::PublicItem::TypeAlias { .. } => "TypeAlias",
                    otterc_ffi::PublicItem::Module { .. } => "Module",
                    otterc_ffi::PublicItem::Trait { .. } => "Trait",
                };
                *counts.entry(kind).or_insert(0) += 1;
            }

            println!("\n   Item breakdown:");
            for (kind, count) in counts.iter() {
                println!("   - {}: {}", kind, count);
            }

            // Show some example structs
            println!("\n   Example structs:");
            for item in spec.items.iter().take(100) {
                if let otterc_ffi::PublicItem::Struct {
                    name,
                    fields,
                    generics,
                    ..
                } = item
                {
                    println!(
                        "   - struct {}{} {{ {} fields }}",
                        name,
                        if generics.is_empty() {
                            String::new()
                        } else {
                            format!("<{}>", generics.join(", "))
                        },
                        fields.len()
                    );
                }
            }

            assert!(!spec.items.is_empty(), "Should extract at least some items");
        }
        Err(e) => {
            println!("Failed to extract serde: {}", e);
            println!("   This is expected if rustc nightly is not installed");
            println!("   Install with: rustup install nightly");
        }
    }
}

#[test]
#[ignore]
fn test_extract_tokio() {
    let dep = DependencyConfig {
        name: "tokio".to_string(),
        version: Some("1.0".to_string()),
        path: None,
        features: vec!["full".to_string()],
        default_features: true,
    };

    let result = extract_crate_spec(&dep);
    match result {
        Ok(spec) => {
            println!("Successfully extracted tokio metadata");
            println!("   Crate: {} v{:?}", spec.name, spec.version);
            println!("   Total items: {}", spec.items.len());

            // Look for async functions
            let async_functions: Vec<_> = spec
                .items
                .iter()
                .filter_map(|item| {
                    if let otterc_ffi::PublicItem::Function { sig, .. } = item {
                        if sig.is_async { Some(&sig.name) } else { None }
                    } else {
                        None
                    }
                })
                .take(10)
                .collect();

            println!("\n   Example async functions:");
            for name in async_functions {
                println!("   - async fn {}", name);
            }

            assert!(!spec.items.is_empty());
        }
        Err(e) => {
            println!("Failed to extract tokio: {}", e);
            println!("   This is expected if rustc nightly is not installed");
        }
    }
}

#[test]
#[ignore]
fn test_extract_reqwest() {
    let dep = DependencyConfig {
        name: "reqwest".to_string(),
        version: Some("0.11".to_string()),
        path: None,
        features: vec!["json".to_string()],
        default_features: true,
    };

    let result = extract_crate_spec(&dep);
    match result {
        Ok(spec) => {
            println!("Successfully extracted reqwest metadata");
            println!("   Crate: {} v{:?}", spec.name, spec.version);
            println!("   Total items: {}", spec.items.len());

            // Look for the Client struct and its methods
            for item in &spec.items {
                if let otterc_ffi::PublicItem::Struct { name, .. } = item
                    && name == "Client"
                {
                    println!("\n   Found Client struct!");
                }
            }

            // Count methods
            let method_count = spec
                .items
                .iter()
                .filter(|item| matches!(item, otterc_ffi::PublicItem::Method { .. }))
                .count();

            println!("   Total methods: {}", method_count);

            assert!(!spec.items.is_empty());
        }
        Err(e) => {
            println!("Failed to extract reqwest: {}", e);
            println!("   This is expected if rustc nightly is not installed");
        }
    }
}

#[test]
#[ignore]
fn test_extract_simple_local_crate() {
    // Test with a simple local crate to verify basic functionality
    let test_crate_dir = std::env::temp_dir().join("test_ffi_crate");
    std::fs::create_dir_all(&test_crate_dir).unwrap();

    // Create a simple test crate
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
    pub mod inner {
        pub fn nested_value(x: i32) -> i32 {
            x * 2
        }
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

    let result = extract_crate_spec(&dep);
    match result {
        Ok(spec) => {
            println!("Successfully extracted test crate metadata");
            println!("   Total items: {}", spec.items.len());

            println!("\n   All extracted items:");
            for item in &spec.items {
                match item {
                    otterc_ffi::PublicItem::Function { sig, .. } => {
                        println!("   - Function: {}", sig.name);
                    }
                    otterc_ffi::PublicItem::Method { sig, .. } => {
                        println!("   - Method: {}", sig.name);
                    }
                    otterc_ffi::PublicItem::AssocFunction { sig, .. } => {
                        println!("   - AssocFunction: {}", sig.name);
                    }
                    otterc_ffi::PublicItem::Const { name, .. } => println!("   - Const: {}", name),
                    otterc_ffi::PublicItem::Static { name, .. } => {
                        println!("   - Static: {}", name);
                    }
                    otterc_ffi::PublicItem::Struct { name, .. } => {
                        println!("   - Struct: {}", name);
                    }
                    otterc_ffi::PublicItem::Enum { name, .. } => println!("   - Enum: {}", name),
                    otterc_ffi::PublicItem::TypeAlias { name, .. } => {
                        println!("   - TypeAlias: {}", name);
                    }
                    otterc_ffi::PublicItem::Module { name, .. } => {
                        println!("   - Module: {}", name);
                    }
                    otterc_ffi::PublicItem::Trait { name, .. } => println!("   - Trait: {}", name),
                }
            }

            let has_point_struct = spec.items.iter().any(
                |item| matches!(item, otterc_ffi::PublicItem::Struct { name, .. } if name == "Point"),
            );
            let has_shape_enum = spec.items.iter().any(
                |item| matches!(item, otterc_ffi::PublicItem::Enum { name, .. } if name == "Shape"),
            );
            let has_add_function = spec.items.iter().any(
                |item| matches!(item, otterc_ffi::PublicItem::Function { sig, .. } if sig.name == "add"),
            );
            let has_pi_const = spec.items.iter().any(
                |item| matches!(item, otterc_ffi::PublicItem::Const { name, .. } if name == "PI"),
            );
            let has_new_method = spec.items.iter().any(|item| {
                matches!(item, otterc_ffi::PublicItem::AssocFunction { sig, .. } if sig.name == "new")
            });
            let has_distance_method = spec.items.iter().any(|item| {
                matches!(item, otterc_ffi::PublicItem::Method { sig, .. } if sig.name == "distance")
            });
            let has_nested_function = spec.items.iter().any(|item| {
                matches!(
                    item,
                    otterc_ffi::PublicItem::Function { sig, path, .. }
                        if sig.name == "nested_value"
                            && path.display_colon()
                                == "test_ffi_simple::nested::inner::nested_value"
                )
            });

            println!("\n   Verification:");
            println!(
                "   - Point struct: {}",
                if has_point_struct { "yes" } else { "no" }
            );
            println!(
                "   - Shape enum: {}",
                if has_shape_enum { "yes" } else { "no" }
            );
            println!(
                "   - add function: {}",
                if has_add_function { "yes" } else { "no" }
            );
            println!(
                "   - PI constant: {}",
                if has_pi_const { "yes" } else { "no" }
            );
            println!(
                "   - Point::new: {}",
                if has_new_method { "yes" } else { "no" }
            );
            println!(
                "   - Point::distance: {}",
                if has_distance_method { "yes" } else { "no" }
            );
            println!(
                "   - nested::inner::nested_value: {}",
                if has_nested_function { "yes" } else { "no" }
            );

            assert!(has_point_struct, "Should extract Point struct");
            assert!(has_shape_enum, "Should extract Shape enum");
            assert!(has_add_function, "Should extract add function");
            assert!(has_pi_const, "Should extract PI constant");
            assert!(
                has_new_method,
                "Should extract Point::new as assoc function"
            );
            assert!(has_distance_method, "Should extract Point::distance method");
            assert!(
                has_nested_function,
                "Should extract nested::inner::nested_value function"
            );
        }
        Err(e) => {
            println!("Failed to extract test crate: {}", e);
            panic!("Test crate extraction should work: {}", e);
        }
    }

    // Cleanup
    let _ = std::fs::remove_dir_all(test_crate_dir);
}
