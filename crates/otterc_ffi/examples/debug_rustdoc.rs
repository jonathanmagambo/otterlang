#![expect(
    clippy::print_stdout,
    reason = "Printing to stdout is acceptable in examples"
)]
#![expect(
    clippy::print_stderr,
    reason = "Printing to stderr is acceptable in examples"
)]

use otterc_ffi::{DependencyConfig, generate_rustdoc_json};
use std::fs;

fn main() {
    let test_crate_dir = std::env::temp_dir().join("test_ffi_debug");
    std::fs::create_dir_all(&test_crate_dir).unwrap();

    let manifest = "[package]
name = \"test_debug\"
version = \"0.1.0\"
edition = \"2021\"
";

    let lib_rs = "
pub struct Person {
    pub name: String,
    pub age: u32,
}
";

    std::fs::write(test_crate_dir.join("Cargo.toml"), manifest).unwrap();
    std::fs::create_dir_all(test_crate_dir.join("src")).unwrap();
    std::fs::write(test_crate_dir.join("src/lib.rs"), lib_rs).unwrap();

    let dep = DependencyConfig {
        name: "test_debug".to_string(),
        version: None,
        path: Some(test_crate_dir.clone()),
        features: vec![],
        default_features: true,
    };

    match generate_rustdoc_json(&dep) {
        Ok(json_path) => {
            println!("Rustdoc JSON path: {}", json_path.display());
            let content = fs::read_to_string(&json_path).unwrap();
            let json: serde_json::Value = serde_json::from_str(&content).unwrap();

            println!("\n=== Index Keys ===");
            if let Some(index) = json.get("index").and_then(|i| i.as_object()) {
                for (id, item) in index.iter().take(20) {
                    if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                        let kind = item
                            .get("inner")
                            .and_then(|i| i.as_object())
                            .and_then(|o| o.keys().next())
                            .map(|k| k.as_str())
                            .unwrap_or("unknown");
                        println!("  {} -> {} ({})", id, name, kind);
                    }
                }
            }

            println!("\n=== Person Struct Details ===");
            if let Some(index) = json.get("index").and_then(|i| i.as_object()) {
                for (id, item) in index.iter() {
                    if let Some(name) = item.get("name").and_then(|n| n.as_str())
                        && name == "Person"
                    {
                        println!("Found Person at ID: {}", id);
                        println!("Full item: {}", serde_json::to_string_pretty(item).unwrap());
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
        }
    }

    let _ = std::fs::remove_dir_all(test_crate_dir);
}
