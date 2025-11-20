use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use tracing::warn;

use crate::codegen::llvm::{preferred_target_flag, prepare_rust_bridges};
use crate::codegen::target::TargetTriple;
use crate::codegen::{BuildArtifact, CodegenOptions};
use crate::runtime;
use crate::typecheck::TypeInfo;
use ast::nodes::Program;

use super::backend::CraneliftBackend;

/// Build a native executable using the Cranelift backend
pub fn build_executable(
    program: &Program,
    expr_types: &HashMap<usize, TypeInfo>,
    output: &Path,
    options: &CodegenOptions,
) -> Result<BuildArtifact> {
    let registry = runtime::ffi::bootstrap_stdlib();
    let bridge_libraries = prepare_rust_bridges(program, registry)?;

    if options.emit_ir {
        warn!(
            "Cranelift backend does not currently support emitting textual IR; ignoring --dump-ir"
        );
    }
    if options.enable_lto {
        warn!("Cranelift backend does not support LTO; disabling -flto for this build");
    }
    if options.enable_pgo {
        warn!("Cranelift backend does not support PGO; ignoring PGO-related flags");
    }

    // Ensure we have an entry point for executable builds
    if !program.functions().any(|f| f.name == "main") {
        bail!("entry function `main` not found");
    }

    let runtime_triple = resolve_runtime_triple(options);
    let triple_str = runtime_triple.to_llvm_triple();

    let mut backend =
        CraneliftBackend::new(&runtime_triple, registry, clone_expr_type_map(expr_types))?;

    let mut declared = Vec::new();
    for function in program.functions() {
        let func_id = backend.declare_function(function)?;
        declared.push((func_id, function));
    }
    for (func_id, function) in declared {
        backend.build_function(func_id, function)?;
    }

    let compiled = backend.finalize_module()?;
    let object_bytes = compiled
        .object_bytes
        .ok_or_else(|| anyhow!("Cranelift backend did not produce object bytes"))?;

    let object_path = output.with_extension("o");
    fs::write(&object_path, &object_bytes).with_context(|| {
        format!(
            "failed to write Cranelift object file {}",
            object_path.display()
        )
    })?;

    let runtime_c = if runtime_triple.is_wasm() {
        None
    } else {
        let runtime_c = output.with_extension("runtime.c");
        let runtime_c_content = runtime_triple.runtime_c_code();
        fs::write(&runtime_c, runtime_c_content)
            .context("failed to write runtime C shim for Cranelift backend")?;
        Some(runtime_c)
    };

    let runtime_o = if let Some(ref rt_c) = runtime_c {
        let runtime_o = output.with_extension("runtime.o");
        let c_compiler = runtime_triple.c_compiler();
        let mut cc = Command::new(&c_compiler);
        cc.arg("-c");
        if runtime_triple.needs_pic() && !runtime_triple.is_windows() {
            cc.arg("-fPIC");
        }
        let compiler_target_flag = preferred_target_flag(&c_compiler);
        cc.arg(compiler_target_flag).arg(&triple_str);
        cc.arg(rt_c).arg("-o").arg(&runtime_o);

        let cc_status = cc
            .status()
            .context("failed to compile Cranelift runtime C shim")?;
        if !cc_status.success() {
            bail!("failed to compile runtime C shim for Cranelift backend");
        }
        Some(runtime_o)
    } else {
        None
    };

    let linker = runtime_triple.linker();
    let mut cc = Command::new(&linker);
    let linker_target_flag = preferred_target_flag(&linker);

    if runtime_triple.is_wasm() {
        cc.arg(linker_target_flag)
            .arg(&triple_str)
            .arg("--no-entry")
            .arg("--export-dynamic")
            .arg(&object_path)
            .arg("-o")
            .arg(output);
    } else {
        cc.arg(linker_target_flag).arg(&triple_str);
        cc.arg(&object_path);
        if let Some(ref rt_o) = runtime_o {
            cc.arg(rt_o);
        }
        cc.arg("-o").arg(output);
    }

    for flag in runtime_triple.linker_flags() {
        cc.arg(&flag);
    }

    for lib in &bridge_libraries {
        cc.arg(lib);
    }

    let status = cc.status().context("failed to link Cranelift executable")?;

    if !status.success() {
        bail!("linker invocation failed with status {status}");
    }

    if let Some(ref rt_c) = runtime_c {
        fs::remove_file(rt_c).ok();
    }
    if let Some(ref rt_o) = runtime_o {
        fs::remove_file(rt_o).ok();
    }
    fs::remove_file(&object_path).ok();

    Ok(BuildArtifact {
        binary: output.to_path_buf(),
        ir: None,
    })
}

/// Build a shared library using the Cranelift backend
pub fn build_shared_library(
    program: &Program,
    expr_types: &HashMap<usize, TypeInfo>,
    output: &Path,
    options: &CodegenOptions,
) -> Result<BuildArtifact> {
    let registry = runtime::ffi::bootstrap_stdlib();
    let bridge_libraries = prepare_rust_bridges(program, registry)?;

    if options.emit_ir {
        warn!(
            "Cranelift backend does not currently support emitting textual IR; ignoring --dump-ir"
        );
    }
    if options.enable_lto {
        warn!("Cranelift backend does not support LTO; disabling -flto for this build");
    }
    if options.enable_pgo {
        warn!("Cranelift backend does not support PGO; ignoring PGO-related flags");
    }

    let runtime_triple = resolve_runtime_triple(options);
    let triple_str = runtime_triple.to_llvm_triple();

    let mut backend =
        CraneliftBackend::new(&runtime_triple, registry, clone_expr_type_map(expr_types))?;

    // Two-pass lowering: declare all functions before defining bodies
    let mut declared = Vec::new();
    for function in program.functions() {
        let func_id = backend.declare_function(function)?;
        declared.push((func_id, function));
    }
    for (func_id, function) in declared {
        backend.build_function(func_id, function)?;
    }

    let compiled = backend.finalize_module()?;
    let object_bytes = compiled
        .object_bytes
        .ok_or_else(|| anyhow!("Cranelift backend did not produce object bytes"))?;

    let object_path = output.with_extension("o");
    fs::write(&object_path, &object_bytes).with_context(|| {
        format!(
            "failed to write Cranelift object file {}",
            object_path.display()
        )
    })?;

    let runtime_c = if runtime_triple.is_wasm() {
        None
    } else {
        let runtime_c = output.with_extension("runtime.c");
        let runtime_c_content = runtime_triple.runtime_c_code();
        fs::write(&runtime_c, runtime_c_content)
            .context("failed to write runtime C shim for Cranelift backend")?;
        Some(runtime_c)
    };

    let runtime_o = if let Some(ref rt_c) = runtime_c {
        let runtime_o = output.with_extension("runtime.o");
        let c_compiler = runtime_triple.c_compiler();
        let mut cc = Command::new(&c_compiler);
        cc.arg("-c");
        if runtime_triple.needs_pic() && !runtime_triple.is_windows() {
            cc.arg("-fPIC");
        }
        let compiler_target_flag = preferred_target_flag(&c_compiler);
        cc.arg(compiler_target_flag).arg(&triple_str);
        cc.arg(rt_c).arg("-o").arg(&runtime_o);

        let cc_status = cc
            .status()
            .context("failed to compile Cranelift runtime C shim")?;
        if !cc_status.success() {
            bail!("failed to compile runtime C shim for Cranelift backend");
        }
        Some(runtime_o)
    } else {
        None
    };

    let lib_ext = if runtime_triple.is_wasm() {
        "wasm"
    } else if runtime_triple.is_windows() {
        "dll"
    } else if runtime_triple.os == "darwin" {
        "dylib"
    } else {
        "so"
    };
    let lib_path = if output
        .extension()
        .map(|ext| ext == lib_ext)
        .unwrap_or(false)
    {
        output.to_path_buf()
    } else {
        output.with_extension(lib_ext)
    };

    let linker = runtime_triple.linker();
    let mut cc = Command::new(&linker);
    let linker_target_flag = preferred_target_flag(&linker);

    if runtime_triple.is_wasm() {
        cc.arg(linker_target_flag)
            .arg(&triple_str)
            .arg("--no-entry")
            .arg("--export-dynamic")
            .arg("-o")
            .arg(&lib_path)
            .arg(&object_path);
    } else {
        cc.arg("-shared");
        if runtime_triple.needs_pic() {
            cc.arg("-fPIC");
        }
        cc.arg(linker_target_flag).arg(&triple_str);
        cc.arg("-o").arg(&lib_path).arg(&object_path);
        if let Some(ref rt_o) = runtime_o {
            cc.arg(rt_o);
        }
    }

    for flag in runtime_triple.linker_flags() {
        cc.arg(&flag);
    }

    for lib in &bridge_libraries {
        cc.arg(lib);
    }

    let status = cc
        .status()
        .context("failed to invoke system linker for Cranelift backend")?;
    if !status.success() {
        bail!("Cranelift linker invocation failed with status {status}");
    }

    if let Some(ref rt_c) = runtime_c {
        fs::remove_file(rt_c).ok();
    }
    if let Some(ref rt_o) = runtime_o {
        fs::remove_file(rt_o).ok();
    }
    fs::remove_file(&object_path).ok();

    Ok(BuildArtifact {
        binary: lib_path,
        ir: None,
    })
}

fn resolve_runtime_triple(options: &CodegenOptions) -> TargetTriple {
    if let Some(ref target) = options.target {
        return target.clone();
    }

    let native = target_lexicon::Triple::host();
    let native_str = native.to_string();
    TargetTriple::parse(&native_str)
        .unwrap_or_else(|_| TargetTriple::new("x86_64", "unknown", "linux", Some("gnu")))
}

fn clone_expr_type_map(expr_types: &HashMap<usize, TypeInfo>) -> HashMap<usize, TypeInfo> {
    expr_types
        .iter()
        .map(|(id, ty)| (*id, ty.clone()))
        .collect::<HashMap<_, _>>()
}
