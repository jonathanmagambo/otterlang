use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use ast::nodes::Program;
use inkwell::OptimizationLevel;
use inkwell::context::Context as LlvmContext;
use inkwell::targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target};

use crate::codegen::target::TargetTriple;
use crate::typecheck::TypeInfo;

use super::bridges::prepare_rust_bridges;
use super::compiler::Compiler;
use super::config::{
    BuildArtifact, CodegenOptLevel, CodegenOptions, llvm_triple_to_string, preferred_target_flag,
};

pub fn current_llvm_version() -> String {
    "15.0".to_string()
}

pub fn build_executable(
    program: &Program,
    expr_types: &HashMap<usize, TypeInfo>,
    output: &Path,
    options: &CodegenOptions,
) -> Result<BuildArtifact> {
    let context = LlvmContext::create();
    let module = context.create_module("otter");
    let builder = context.create_builder();
    let registry = crate::runtime::ffi::bootstrap_stdlib();
    let bridge_libraries = prepare_rust_bridges(program, registry)?;
    let mut compiler = Compiler::new(&context, module, builder, registry, expr_types);

    compiler.lower_program(program, true)?; // Require main for executables
    compiler
        .module
        .verify()
        .map_err(|e| anyhow!("LLVM module verification failed: {e}"))?;

    if options.emit_ir {
        // Ensure IR snapshot happens before LLVM potentially mutates the module during codegen.
        compiler.cached_ir = Some(compiler.module.print_to_string().to_string());
    }

    // Initialize all LLVM targets before creating any target triples
    Target::initialize_all(&InitializationConfig::default());

    // Determine target triple: use provided target or fall back to native
    let runtime_triple = options.target.clone().unwrap_or_else(|| {
        let native_triple = inkwell::targets::TargetMachine::get_default_triple();
        TargetTriple::parse(&llvm_triple_to_string(&native_triple))
            .unwrap_or_else(|_| TargetTriple::new("x86_64", "unknown", "linux", Some("gnu")))
    });

    // Convert to LLVM triple format
    let triple_str = runtime_triple.to_llvm_triple();
    let llvm_triple = inkwell::targets::TargetTriple::create(&triple_str);

    // Check if we're compiling for the native target
    let native_triple = inkwell::targets::TargetMachine::get_default_triple();
    let is_native_target =
        llvm_triple_to_string(&llvm_triple) == llvm_triple_to_string(&native_triple);
    compiler.module.set_triple(&llvm_triple);

    let target = Target::from_triple(&llvm_triple)
        .map_err(|e| anyhow!("failed to create target from triple {}: {e}", triple_str))?;

    let optimization: OptimizationLevel = options.opt_level.into();
    let reloc_mode = if runtime_triple.needs_pic() {
        RelocMode::PIC
    } else {
        RelocMode::Default
    };
    let target_machine = target
        .create_target_machine(
            &llvm_triple,
            "generic",
            "",
            optimization,
            reloc_mode,
            CodeModel::Default,
        )
        .ok_or_else(|| anyhow!("failed to create target machine"))?;

    compiler
        .module
        .set_data_layout(&target_machine.get_target_data().get_data_layout());

    compiler.run_default_passes(
        options.opt_level,
        options.enable_pgo,
        options.pgo_profile_file.as_deref(),
        options.inline_threshold,
        &target_machine,
    );

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create output directory {}", parent.display())
        })?;
    }

    let object_path = output.with_extension("o");
    target_machine
        .write_to_file(&compiler.module, FileType::Object, &object_path)
        .map_err(|e| {
            anyhow!(
                "failed to emit object file at {}: {e}",
                object_path.display()
            )
        })?;

    // Create a C runtime shim for the FFI functions (target-specific)
    let runtime_c = if runtime_triple.is_wasm() {
        None
    } else {
        let runtime_c = output.with_extension("runtime.c");
        let runtime_c_content = runtime_triple.runtime_c_code();
        fs::write(&runtime_c, runtime_c_content).context("failed to write runtime C file")?;
        Some(runtime_c)
    };

    // Compile runtime C file (target-specific)
    let runtime_o = if let Some(ref rt_c) = runtime_c {
        let runtime_o = output.with_extension("runtime.o");
        let c_compiler = runtime_triple.c_compiler();
        let mut cc = Command::new(&c_compiler);

        // Add target-specific compiler flags
        cc.arg("-c");
        if runtime_triple.needs_pic() && !runtime_triple.is_windows() {
            cc.arg("-fPIC");
        }
        // Add target triple for cross-compilation (skip for native target)
        if !is_native_target {
            let compiler_target_flag = preferred_target_flag(&c_compiler);
            cc.arg(compiler_target_flag).arg(&triple_str);
        }

        cc.arg(rt_c).arg("-o").arg(&runtime_o);

        let cc_status = cc.status().context("failed to compile runtime C file")?;

        if !cc_status.success() {
            bail!("failed to compile runtime C file");
        }

        Some(runtime_o)
    } else {
        None
    };

    // Link the object files together (target-specific)
    let linker = runtime_triple.linker();
    let mut cc = Command::new(&linker);

    // Add target-specific linker flags
    if runtime_triple.is_wasm() {
        // WebAssembly linking
        let linker_target_flag = preferred_target_flag(&linker);
        cc.arg(linker_target_flag)
            .arg(&triple_str)
            .arg("--no-entry")
            .arg("--export-dynamic")
            .arg(&object_path)
            .arg("-o")
            .arg(output);
    } else {
        // Standard linking
        if !is_native_target {
            let linker_target_flag = preferred_target_flag(&linker);
            cc.arg(linker_target_flag).arg(&triple_str);
        }
        if let Some(ref rt_o) = runtime_o {
            cc.arg(&object_path).arg(rt_o).arg("-o").arg(output);
        } else {
            cc.arg(&object_path).arg("-o").arg(output);
        }
    }

    // Apply target-specific linker flags
    for flag in runtime_triple.linker_flags() {
        cc.arg(&flag);
    }

    if options.enable_lto && !runtime_triple.is_wasm() {
        cc.arg("-flto");
        // Note: clang doesn't support -flto=O2/O3, use -O flags instead
        match options.opt_level {
            CodegenOptLevel::None => {}
            CodegenOptLevel::Default => {
                cc.arg("-O2");
            }
            CodegenOptLevel::Aggressive => {
                cc.arg("-O3");
            }
        }
    }

    // PGO support: if profile file is provided, use it for optimization
    if options.enable_pgo && !runtime_triple.is_wasm() {
        if let Some(ref profile_file) = options.pgo_profile_file {
            cc.arg("-fprofile-use");
            cc.arg(profile_file);
        } else {
            // Generate profile instrumentation
            cc.arg("-fprofile-instr-generate");
        }
    }

    for lib in &bridge_libraries {
        cc.arg(lib);
    }

    let status = cc.status().context("failed to invoke system linker (cc)")?;

    if !status.success() {
        bail!("linker invocation failed with status {status}");
    }

    // Clean up temporary files
    if let Some(ref rt_c) = runtime_c {
        fs::remove_file(rt_c).ok();
    }
    if let Some(ref rt_o) = runtime_o {
        fs::remove_file(rt_o).ok();
    }

    fs::remove_file(&object_path).ok();

    Ok(BuildArtifact {
        binary: output.to_path_buf(),
        ir: compiler.cached_ir.take(),
    })
}

/// Build a shared library (.so/.dylib) for JIT execution
pub fn build_shared_library(
    program: &Program,
    expr_types: &HashMap<usize, TypeInfo>,
    output: &Path,
    options: &CodegenOptions,
) -> Result<BuildArtifact> {
    let context = LlvmContext::create();
    let module = context.create_module("otter_jit");
    let builder = context.create_builder();
    let registry = crate::runtime::ffi::bootstrap_stdlib();
    let bridge_libraries = prepare_rust_bridges(program, registry)?;
    let mut compiler = Compiler::new(&context, module, builder, registry, expr_types);

    compiler.lower_program(program, false)?; // Don't require main for shared libraries
    compiler
        .module
        .verify()
        .map_err(|e| anyhow!("LLVM module verification failed: {e}"))?;

    if options.emit_ir {
        compiler.cached_ir = Some(compiler.module.print_to_string().to_string());
    }

    // Initialize all LLVM targets before creating any target triples
    Target::initialize_all(&InitializationConfig::default());

    // Determine target triple: use provided target or fall back to native
    let runtime_triple = if let Some(ref target) = options.target {
        target.clone()
    } else {
        // Get native target triple directly from LLVM
        let native_triple = inkwell::targets::TargetMachine::get_default_triple();
        let native_str = llvm_triple_to_string(&native_triple);
        TargetTriple::parse(&native_str)
            .unwrap_or_else(|_| TargetTriple::new("x86_64", "unknown", "linux", Some("gnu")))
    };

    // Convert to LLVM triple format
    let triple_str = runtime_triple.to_llvm_triple();
    let llvm_triple = inkwell::targets::TargetTriple::create(&triple_str);
    compiler.module.set_triple(&llvm_triple);

    let target = Target::from_triple(&llvm_triple)
        .map_err(|e| anyhow!("failed to create target from triple {}: {e}", triple_str))?;

    let optimization: OptimizationLevel = options.opt_level.into();
    let reloc_mode = RelocMode::PIC;
    let target_machine = target
        .create_target_machine(
            &llvm_triple,
            "generic",
            "",
            optimization,
            reloc_mode,
            CodeModel::Default,
        )
        .ok_or_else(|| anyhow!("failed to create target machine"))?;

    compiler
        .module
        .set_data_layout(&target_machine.get_target_data().get_data_layout());

    compiler.run_default_passes(
        options.opt_level,
        options.enable_pgo,
        options.pgo_profile_file.as_deref(),
        options.inline_threshold,
        &target_machine,
    );

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create output directory {}", parent.display())
        })?;
    }

    // Compile to object file with position-independent code
    let object_path = output.with_extension("o");
    target_machine
        .write_to_file(&compiler.module, FileType::Object, &object_path)
        .map_err(|e| {
            anyhow!(
                "failed to emit object file at {}: {e}",
                object_path.display()
            )
        })?;

    // Create runtime C file (target-specific)
    let runtime_c = if runtime_triple.is_wasm() {
        None
    } else {
        let runtime_c = output.with_extension("runtime.c");
        let runtime_c_content = runtime_triple.runtime_c_code();
        fs::write(&runtime_c, runtime_c_content).context("failed to write runtime C file")?;
        Some(runtime_c)
    };

    // Compile runtime C file (target-specific)
    let runtime_o = if let Some(ref rt_c) = runtime_c {
        let runtime_o = output.with_extension("runtime.o");
        let c_compiler = runtime_triple.c_compiler();
        let mut cc = Command::new(&c_compiler);

        // Add target-specific compiler flags
        cc.arg("-c");
        if runtime_triple.needs_pic() && !runtime_triple.is_windows() {
            cc.arg("-fPIC");
        }
        let compiler_target_flag = preferred_target_flag(&c_compiler);
        cc.arg(compiler_target_flag).arg(&triple_str);

        cc.arg(rt_c).arg("-o").arg(&runtime_o);

        let cc_status = cc.status().context("failed to compile runtime C file")?;

        if !cc_status.success() {
            bail!("failed to compile runtime C file");
        }

        Some(runtime_o)
    } else {
        None
    };

    // Determine shared library extension (target-specific)
    let lib_ext = if runtime_triple.is_wasm() {
        "wasm"
    } else if runtime_triple.is_windows() {
        "dll"
    } else if runtime_triple.os == "darwin" {
        "dylib"
    } else {
        "so"
    };

    let lib_path = if output.extension().is_some() && output.extension().unwrap() == lib_ext {
        output.to_path_buf()
    } else {
        output.with_extension(lib_ext)
    };

    // Link as shared library (target-specific)
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

    // Apply target-specific linker flags
    for flag in runtime_triple.linker_flags() {
        cc.arg(&flag);
    }

    if options.enable_lto && !runtime_triple.is_wasm() {
        cc.arg("-flto");
        // Note: clang doesn't support -flto=O2/O3, use -O flags instead
        match options.opt_level {
            CodegenOptLevel::None => {}
            CodegenOptLevel::Default => {
                cc.arg("-O2");
            }
            CodegenOptLevel::Aggressive => {
                cc.arg("-O3");
            }
        }
    }

    // PGO support: if profile file is provided, use it for optimization
    if options.enable_pgo && !runtime_triple.is_wasm() {
        if let Some(ref profile_file) = options.pgo_profile_file {
            cc.arg("-fprofile-use");
            cc.arg(profile_file);
        } else {
            // Generate profile instrumentation
            cc.arg("-fprofile-instr-generate");
        }
    }

    for lib in &bridge_libraries {
        cc.arg(lib);
    }

    let status = cc.status().context("failed to invoke system linker (cc)")?;

    if !status.success() {
        bail!("linker invocation failed with status {status}");
    }

    // Clean up temporary files
    if let Some(ref rt_c) = runtime_c {
        fs::remove_file(rt_c).ok();
    }
    if let Some(ref rt_o) = runtime_o {
        fs::remove_file(rt_o).ok();
    }
    fs::remove_file(&object_path).ok();

    Ok(BuildArtifact {
        binary: lib_path,
        ir: compiler.cached_ir.take(),
    })
}
