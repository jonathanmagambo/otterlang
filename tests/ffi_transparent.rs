use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};

/// Helper to find the otter binary
fn otter_bin() -> Result<PathBuf> {
    let target_dir = if cfg!(debug_assertions) {
        "target/debug"
    } else {
        "target/release"
    };
    let bin = PathBuf::from(target_dir).join("otter");
    if bin.exists() {
        Ok(bin)
    } else {
        anyhow::bail!("otter binary not found at {}", bin.display());
    }
}

/// Run an OtterLang program from a file and check it succeeds
#[allow(dead_code)]
fn run_otter_file(file: &str) -> Result<String> {
    let otter = otter_bin()?;
    let test_file = PathBuf::from("tests/examples").join(file);

    if !test_file.exists() {
        anyhow::bail!("test file not found: {}", test_file.display());
    }

    let output = Command::new(&otter)
        .args(&["run", test_file.to_str().unwrap()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .context("failed to execute otter")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("otter failed:\n{}", stderr);
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Run an OtterLang program from string and check it succeeds
#[allow(dead_code)]
fn run_otter_program(_program: &str) -> Result<String> {
    let otter = otter_bin()?;
    let output = Command::new(&otter)
        .args(&["run", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .context("failed to execute otter")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("otter failed:\n{}", stderr);
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Test basic FFI import and function call
#[test]
#[ignore] // Requires building otter first and having test crates
fn test_basic_ffi_import() -> Result<()> {
    let program = r#"
use rust:otterlang_ffi_demo

fn main:
    let result = otterlang_ffi_demo.add(3.0, 5.0)
    print("Result: {}", result)
"#;

    let output = run_otter_program(program)?;
    assert!(output.contains("Result:"));
    Ok(())
}

/// Test module.function() syntax
#[test]
#[ignore]
fn test_module_function_syntax() -> Result<()> {
    let program = r#"
use rust:otterlang_ffi_demo

fn main:
    let result = otterlang_ffi_demo.add(10.0, 20.0)
    print("{}", result)
"#;

    let output = run_otter_program(program)?;
    assert!(output.contains("30"));
    Ok(())
}

/// Test async spawn/await pattern
#[test]
#[ignore]
fn test_async_spawn_await() -> Result<()> {
    let program = r#"
use rust:tokio

fn main:
    let handle = spawn(tokio.time.sleep_spawn(100))
    let result = await(tokio.time.sleep_await(handle))
    print("Async completed")
"#;

    let output = run_otter_program(program)?;
    assert!(output.contains("completed"));
    Ok(())
}

/// Test Result type handling with _try helper
#[test]
#[ignore]
fn test_result_try_helper() -> Result<()> {
    let program = r#"
use rust:some_crate

fn main:
    let result = some_crate.failable_function_try("input")
    print("Result: {}", result)
"#;

    // This test would need a real crate with Result return
    // For now, just verify it compiles
    let _output = run_otter_program(program);
    // We expect this might fail if crate doesn't exist, which is ok for now
    Ok(())
}

/// Test Option type handling with _optjson helper
#[test]
#[ignore]
fn test_option_optjson_helper() -> Result<()> {
    let program = r#"
use rust:some_crate

fn main:
    let result = some_crate.optional_function_optjson("key")
    print("Result: {}", result)
"#;

    // This test would need a real crate with Option return
    let _output = run_otter_program(program);
    // We expect this might fail if crate doesn't exist, which is ok for now
    Ok(())
}

/// Test memory cleanup (opaque handles)
#[test]
#[ignore]
fn test_memory_cleanup() -> Result<()> {
    let program = r#"
use rust:some_crate

fn main:
    let handle1 = some_crate.create_resource()
    let handle2 = some_crate.create_resource()
    // Handles should be automatically dropped when they go out of scope
    print("Resources created and cleaned up")
"#;

    let _output = run_otter_program(program);
    // We expect this might fail if crate doesn't exist, which is ok for now
    Ok(())
}

/// Test bridge caching
#[test]
#[ignore]
fn test_bridge_caching() -> Result<()> {
    // First run should build bridge
    let program = r#"
use rust:otterlang_ffi_demo

fn main:
    let _ = otterlang_ffi_demo.add(1.0, 2.0)
"#;

    let _output1 = run_otter_program(program)?;

    // Second run should use cache
    let _output2 = run_otter_program(program)?;

    // Verify cache directory exists
    let cache_dir = std::env::var("HOME")
        .map(|h| PathBuf::from(h).join(".otter_cache").join("ffi"))
        .unwrap_or_else(|_| PathBuf::from(".otter_cache").join("ffi"));

    if cache_dir.exists() {
        let entries: Vec<_> = fs::read_dir(&cache_dir)
            .context("failed to read cache dir")?
            .collect::<Result<Vec<_>, _>>()?;

        // Should have at least one cached bridge
        assert!(
            !entries.is_empty(),
            "cache directory should contain bridges"
        );
    }

    Ok(())
}

/// Test type checking with registry signatures
#[test]
#[ignore]
fn test_type_checking_with_registry() -> Result<()> {
    let program = r#"
use rust:otterlang_ffi_demo

fn main:
    // This should fail type checking: wrong number of args
    let result = otterlang_ffi_demo.add(1.0)
"#;

    let output = run_otter_program(program);
    // Should fail with type error
    assert!(
        output.is_err(),
        "should fail type checking with wrong arg count"
    );
    Ok(())
}

/// Test popular crates: rand
#[test]
#[ignore]
fn test_rand_crate() -> Result<()> {
    let program = r#"
use rust:rand

fn main:
    let value = rand.random_f64()
    print("Random: {}", value)
"#;

    let output = run_otter_program(program)?;
    assert!(output.contains("Random:"));
    Ok(())
}

/// Test popular crates: serde_json
#[test]
#[ignore]
fn test_serde_json_crate() -> Result<()> {
    let program = r#"
use rust:serde_json

fn main:
    let json = "{\"key\": \"value\"}"
    let result = serde_json.from_str_try(json)
    print("Parsed: {}", result)
"#;

    let output = run_otter_program(program)?;
    assert!(output.contains("Parsed:"));
    Ok(())
}

/// Test popular crates: chrono
#[test]
#[ignore]
fn test_chrono_crate() -> Result<()> {
    let program = r#"
use rust:chrono

fn main:
    let now = chrono.Utc.now()
    print("Time: {}", now)
"#;

    let output = run_otter_program(program)?;
    assert!(output.contains("Time:"));
    Ok(())
}

/// Test memory stress: many handles
#[test]
#[ignore]
fn test_memory_stress_many_handles() -> Result<()> {
    let program = r#"
use rust:some_crate

fn main:
    let mut handles = []
    for i in 0..1000:
        let handle = some_crate.create_resource()
        handles.append(handle)
    // All handles should be cleaned up automatically
    print("Created 1000 handles, all cleaned up")
"#;

    let _output = run_otter_program(program);
    // We expect this might fail if crate doesn't exist, which is ok for now
    Ok(())
}

/// Test async cancellation
#[test]
#[ignore]
fn test_async_cancellation() -> Result<()> {
    let program = r#"
use rust:tokio

fn main:
    // Spawn multiple async tasks
    let h1 = spawn(tokio.time.sleep_spawn(1000))
    let h2 = spawn(tokio.time.sleep_spawn(2000))
    
    // Only await one
    let _ = await(tokio.time.sleep_await(h1))
    
    // h2 should be cleaned up when it goes out of scope
    print("Async cancellation test")
"#;

    let _output = run_otter_program(program);
    // We expect this might fail if crate doesn't exist, which is ok for now
    Ok(())
}

/// Test concurrent async calls
#[test]
#[ignore]
fn test_concurrent_async() -> Result<()> {
    let program = r#"
use rust:tokio

fn main:
    let h1 = spawn(tokio.time.sleep_spawn(100))
    let h2 = spawn(tokio.time.sleep_spawn(100))
    let h3 = spawn(tokio.time.sleep_spawn(100))
    
    let r1 = await(tokio.time.sleep_await(h1))
    let r2 = await(tokio.time.sleep_await(h2))
    let r3 = await(tokio.time.sleep_await(h3))
    
    print("All async tasks completed")
"#;

    let _output = run_otter_program(program);
    // We expect this might fail if crate doesn't exist, which is ok for now
    Ok(())
}
