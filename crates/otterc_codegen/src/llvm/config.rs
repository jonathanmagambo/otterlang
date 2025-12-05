use std::path::{Path, PathBuf};
use std::process::Command;

use inkwell::targets::TargetTriple as LlvmTargetTriple;

pub(crate) fn llvm_triple_to_string(triple: &LlvmTargetTriple) -> String {
    triple
        .as_str()
        .to_str()
        .unwrap_or("unknown-unknown-unknown")
        .to_string()
}

pub(crate) fn preferred_target_flag(driver: &str) -> &'static str {
    if driver_prefers_clang_style(driver) {
        "-target"
    } else {
        "--target"
    }
}

fn driver_prefers_clang_style(driver: &str) -> bool {
    let lower = driver.to_ascii_lowercase();
    if lower.contains("clang") || lower.contains("wasm-ld") {
        return true;
    }

    match Path::new(driver)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase())
    {
        Some(ref name) if name == "cc" || name == "c++" => compiler_reports_clang(driver),
        _ => false,
    }
}

fn compiler_reports_clang(driver: &str) -> bool {
    Command::new(driver)
        .arg("--version")
        .output()
        .ok()
        .map(|output| {
            let mut text = String::new();
            text.push_str(&String::from_utf8_lossy(&output.stdout));
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            text.to_ascii_lowercase().contains("clang")
        })
        .unwrap_or(false)
}

pub struct BuildArtifact {
    pub binary: PathBuf,
    pub ir: Option<String>,
}
