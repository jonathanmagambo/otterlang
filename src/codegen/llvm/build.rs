use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use glob::glob;
use inkwell::OptimizationLevel;
use inkwell::context::Context as LlvmContext;
use inkwell::targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target};
use otterc_ast::nodes::Program;
use otterc_span::Span;

use crate::codegen::target::TargetTriple;
use crate::typecheck::{EnumLayout, TypeInfo};

use super::bridges::prepare_rust_bridges;
use super::compiler::Compiler;
use super::config::{
    BuildArtifact, CodegenOptLevel, CodegenOptions, llvm_triple_to_string, preferred_target_flag,
};

/// Check if a library is available on the system
fn check_library_available(lib_name: &str) -> bool {
    // Try pkg-config first
    if Command::new("pkg-config")
        .args(["--exists", lib_name])
        .output()
        .is_ok_and(|output| output.status.success())
    {
        return true;
    }

    // Try checking common library paths
    let common_paths = [
        "/usr/lib",
        "/usr/local/lib",
        "/lib",
        "/lib64",
        "/usr/lib64",
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
    ];

    for path in &common_paths {
        let lib_file = format!("lib{}.so", lib_name);
        let lib_file_a = format!("lib{}.a", lib_name);

        if Path::new(path).join(&lib_file).exists() || Path::new(path).join(&lib_file_a).exists() {
            return true;
        }

        // Check for versioned .so files
        if fs::read_dir(path).is_ok_and(|entries| {
            entries.flatten().any(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .map(|name| name.starts_with(&format!("lib{}.so", lib_name)))
                    .unwrap_or(false)
            })
        }) {
            return true;
        }
    }

    false
}

pub fn current_llvm_version() -> String {
    "15.0".to_string()
}

/// Build the Rust runtime as a static library
fn ensure_runtime_library() -> Result<PathBuf> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let runtime_lib_dir = Path::new(&manifest_dir).join("target").join("runtime");
    let runtime_lib = runtime_lib_dir.join("libotterlang_runtime.a");

    // Create runtime library directory
    fs::create_dir_all(&runtime_lib_dir).context("failed to create runtime library directory")?;

    // Build the runtime as a static library
    // Use --no-default-features to exclude LLVM dependencies
    let mut cmd = Command::new("cargo");
    cmd.args([
        "rustc",
        "--release",
        "--lib",
        "--crate-type=staticlib",
        "--no-default-features",
        "--target-dir",
        runtime_lib_dir.to_str().unwrap(),
    ]);

    // Set macOS deployment target for compatibility
    if cfg!(target_os = "macos") {
        cmd.env("MACOSX_DEPLOYMENT_TARGET", "11.0");
    }

    let status = cmd
        .status()
        .context("failed to build runtime static library")?;

    if !status.success() {
        bail!("failed to build runtime static library");
    }

    // Find the generated library (it will be in target/runtime/release/)
    let release_dir = runtime_lib_dir.join("release");
    let generated_lib = if cfg!(target_os = "windows") {
        release_dir.join("otterlang.lib")
    } else {
        release_dir.join("libotterlang.a")
    };

    if !generated_lib.exists() {
        bail!(
            "runtime library was not generated at expected location: {}",
            generated_lib.display()
        );
    }

    // Copy to expected location
    fs::copy(&generated_lib, &runtime_lib).context("failed to copy runtime library")?;

    Ok(runtime_lib)
}

pub fn build_executable(
    program: &Program,
    expr_types: &HashMap<usize, TypeInfo>,
    expr_types_by_span: &HashMap<Span, TypeInfo>,
    comprehension_var_types: &HashMap<Span, TypeInfo>,
    enum_layouts: &HashMap<String, EnumLayout>,
    output: &Path,
    options: &CodegenOptions,
) -> Result<BuildArtifact> {
    let context = LlvmContext::create();
    let module = context.create_module("otter");
    let builder = context.create_builder();
    let registry = crate::runtime::ffi::bootstrap_stdlib();
    let bridge_libraries = prepare_rust_bridges(program, registry)?;

    // Determine target triple early so compiler can use it for ABI decisions
    Target::initialize_all(&InitializationConfig::default());
    let runtime_triple = options.target.clone().unwrap_or_else(|| {
        let native_triple = inkwell::targets::TargetMachine::get_default_triple();
        TargetTriple::parse(&llvm_triple_to_string(&native_triple))
            .unwrap_or_else(|_| TargetTriple::new("x86_64", "unknown", "linux", Some("gnu")))
    });

    let mut compiler = Compiler::new(
        &context,
        module,
        builder,
        registry,
        expr_types.clone(),
        expr_types_by_span.clone(),
        comprehension_var_types.clone(),
        enum_layouts.clone(),
        Some(runtime_triple.clone()),
    );

    compiler.lower_program(program, true)?; // Require main for executables
    compiler
        .module
        .verify()
        .map_err(|e| anyhow!("LLVM module verification failed: {e}"))?;

    if options.emit_ir {
        // Ensure IR snapshot happens before LLVM potentially mutates the module during codegen.
        compiler.cached_ir = Some(compiler.module.print_to_string().to_string());
    }

    // runtime_triple was computed earlier for compiler ABI decisions

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

    // macOS on x86_64 needs explicit SSE feature flags; other targets don't
    let (cpu, features) = if runtime_triple.os == "darwin" && runtime_triple.arch == "x86_64" {
        ("generic", "+sse,+sse2,+sse3,+ssse3")
    } else {
        ("generic", "")
    };

    let target_machine = target
        .create_target_machine(
            &llvm_triple,
            cpu,
            features,
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
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory {}", parent.display()))?;
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

    // Build and link the runtime static library (check once)
    let runtime_lib = ensure_runtime_library()?;
    let use_rust_runtime = runtime_lib.exists();

    // Create a C runtime shim for the FFI functions (target-specific)
    let runtime_c = if runtime_triple.is_wasm() {
        None
    } else {
        let runtime_c = output.with_extension("runtime.c");
        let runtime_c_content = if use_rust_runtime {
            runtime_triple.runtime_c_shim_code()
        } else {
            runtime_triple.runtime_c_code()
        };
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

        // Add macOS version minimum
        if runtime_triple.os == "darwin" {
            cc.arg("-mmacosx-version-min=11.0");
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

        // Add macOS version minimum and suppress compatibility warnings
        if runtime_triple.os == "darwin" {
            cc.arg("-mmacosx-version-min=11.0");
            cc.arg("-w"); // Suppress compiler warnings
            // Suppress linker warnings about object files built for newer macOS versions
            cc.arg("-Wl,-w"); // Suppress linker warnings
        }
        // Disable conflicting vcrt library and add windows SDK path on Windows
        if runtime_triple.os == "windows" {
            cc.arg("-Wl,/NODEFAULTLIB:MSVCRT");
            // Search for Windows SDK version
            let mut path = glob("C:/Program Files (x86)/Windows Kits/10/Lib/*")
                .expect("No Windows SDK found")
                .filter_map(Result::ok)
                .find(|p| p.is_dir())
                .expect("No Windows SDK found");
            path.push("ucrt");
            path.push("x64");
            cc.arg(format!("-Wl,/LIBPATH:{}", path.display()));
        }

        // Always link the generated object first
        cc.arg(&object_path);

        if let Some(ref rt_o) = runtime_o {
            cc.arg(rt_o);
        }

        // Delay specifying the output until after we've queued all inputs and flags
        // so the linker sees additional libraries before the final output parameter.
        // (This also matches the convention `cc obj ... libs ... -o binary`.)
        cc.arg("-o").arg(output);
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

    // Link the Rust runtime library (skip if we used C runtime fallback)
    if use_rust_runtime {
        // Link the runtime library - use -force_load on macOS to ensure all symbols are included
        if runtime_triple.os == "darwin" {
            cc.arg(&runtime_lib);
            // Link against system libraries required by LLVM dependencies in runtime
            // Use pkg-config to get proper library paths
            if let Ok(output) = std::process::Command::new("pkg-config")
                .args(["--libs", "libxml-2.0", "libzstd"])
                .output()
            {
                if output.status.success() {
                    let libs = String::from_utf8_lossy(&output.stdout);
                    for lib_flag in libs.split_whitespace() {
                        cc.arg(lib_flag);
                    }
                } else {
                    // Fallback: try Homebrew paths
                    cc.arg("-L/opt/homebrew/lib");
                    cc.arg("-L/opt/homebrew/opt/zstd/lib");
                    cc.arg("-lxml2").arg("-lzstd");
                }
            } else {
                // Fallback: try Homebrew paths
                cc.arg("-L/opt/homebrew/lib");
                cc.arg("-L/opt/homebrew/opt/zstd/lib");
                cc.arg("-lxml2").arg("-lzstd");
            }
            // Standard system libraries
            cc.arg("-lreadline")
                .arg("-lncurses")
                .arg("-lz")
                .arg("-lffi")
                .arg("-lc++");
        } else if runtime_triple.is_windows() {
            cc.arg(&runtime_lib);
            // Link against Windows system libraries required by dependencies (sysinfo, std, etc.)
            cc.arg("-lws2_32")
                .arg("-lpdh")
                .arg("-liphlpapi")
                .arg("-lnetapi32")
                .arg("-luserenv")
                .arg("-ladvapi32")
                .arg("-lpowrprof")
                .arg("-lole32")
                .arg("-loleaut32")
                .arg("-lpsapi")
                .arg("-lntdll")
                .arg("-lshell32")
                .arg("-lsecur32")
                .arg("-lbcrypt")
                .arg("-luser32");
        } else {
            cc.arg(&runtime_lib)
                .arg("-lstdc++")
                .arg("-lm")
                .arg("-ldl")
                .arg("-lpthread")
                .arg("-lz")
                .arg("-lxml2")
                .arg("-lffi")
                .arg("-lzstd");

            // LLVM's LineEditor requires libedit, try to link it
            // If not available, try readline which provides compatible history functions
            if check_library_available("edit") {
                cc.arg("-ledit");
            } else if check_library_available("readline") {
                cc.arg("-lreadline");
            } else {
                // Try both anyway - let the linker fail with a clear error if neither is installed
                cc.arg("-ledit");
            }

            cc.arg("-ltinfo");
        }
    }

    if std::env::var_os("OTTER_LINK_VERBOSE").is_some() {
        cc.arg("-v");
    }

    let status = cc.status().context("failed to invoke system linker (cc)")?;

    if !status.success() {
        bail!("linker invocation failed with status {status}");
    }

    // Clean up temporary files
    if let Some(ref rt_c) = runtime_c {
        fs::remove_file(rt_c)?;
    }
    if let Some(ref rt_o) = runtime_o {
        fs::remove_file(rt_o)?;
    }

    fs::remove_file(&object_path)?;

    Ok(BuildArtifact {
        binary: output.to_path_buf(),
        ir: compiler.cached_ir.take(),
    })
}

/// Build a shared library (.so/.dylib) for JIT execution
pub fn build_shared_library(
    program: &Program,
    expr_types: &HashMap<usize, TypeInfo>,
    expr_types_by_span: &HashMap<Span, TypeInfo>,
    comprehension_var_types: &HashMap<Span, TypeInfo>,
    enum_layouts: &HashMap<String, EnumLayout>,
    output: &Path,
    options: &CodegenOptions,
) -> Result<BuildArtifact> {
    let context = LlvmContext::create();
    let module = context.create_module("otter_jit");
    let builder = context.create_builder();
    let registry = crate::runtime::ffi::bootstrap_stdlib();
    let bridge_libraries = prepare_rust_bridges(program, registry)?;

    // Initialize all LLVM targets before creating any target triples
    Target::initialize_all(&InitializationConfig::default());

    // Determine target triple early so compiler can use it for ABI decisions
    let runtime_triple = if let Some(ref target) = options.target {
        target.clone()
    } else {
        // Get native target triple directly from LLVM
        let native_triple = inkwell::targets::TargetMachine::get_default_triple();
        let native_str = llvm_triple_to_string(&native_triple);
        TargetTriple::parse(&native_str)
            .unwrap_or_else(|_| TargetTriple::new("x86_64", "unknown", "linux", Some("gnu")))
    };

    let mut compiler = Compiler::new(
        &context,
        module,
        builder,
        registry,
        expr_types.clone(),
        expr_types_by_span.clone(),
        comprehension_var_types.clone(),
        enum_layouts.clone(),
        Some(runtime_triple.clone()),
    );

    compiler.lower_program(program, false)?; // Don't require main for shared libraries
    compiler
        .module
        .verify()
        .map_err(|e| anyhow!("LLVM module verification failed: {e}"))?;

    if options.emit_ir {
        compiler.cached_ir = Some(compiler.module.print_to_string().to_string());
    }

    // Convert to LLVM triple format
    let triple_str = runtime_triple.to_llvm_triple();
    let llvm_triple = inkwell::targets::TargetTriple::create(&triple_str);
    compiler.module.set_triple(&llvm_triple);

    let target = Target::from_triple(&llvm_triple)
        .map_err(|e| anyhow!("failed to create target from triple {}: {e}", triple_str))?;

    let optimization: OptimizationLevel = options.opt_level.into();
    let reloc_mode = RelocMode::PIC;

    // macOS on x86_64 needs explicit SSE feature flags; other targets don't
    let (cpu, features) = if runtime_triple.os == "darwin" && runtime_triple.arch == "x86_64" {
        ("generic", "+sse,+sse2,+sse3,+ssse3")
    } else {
        ("generic", "")
    };

    let target_machine = target
        .create_target_machine(
            &llvm_triple,
            cpu,
            features,
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
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory {}", parent.display()))?;
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

        // Add macOS version minimum
        if runtime_triple.os == "darwin" {
            cc.arg("-mmacosx-version-min=11.0");
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

    // Build and check runtime static library (check once)
    let runtime_lib = ensure_runtime_library()?;
    let use_rust_runtime = runtime_lib.exists();

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

        // Add macOS version minimum to avoid platform load command warning
        if runtime_triple.os == "darwin" {
            cc.arg("-mmacosx-version-min=11.0");
            // Suppress linker warnings about object files built for newer macOS versions
            cc.arg("-Wl,-w"); // Suppress linker warnings
        }

        if let Some(ref rt_o) = runtime_o {
            cc.arg(rt_o);
        }

        cc.arg("-o").arg(&lib_path).arg(&object_path);
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

    // Link the Rust runtime library (skip if we used C runtime fallback)
    if use_rust_runtime {
        if runtime_triple.os == "darwin" {
            cc.arg(&runtime_lib);
            // Link against system libraries required by LLVM dependencies in runtime
            cc.arg("-lxml2")
                .arg("-lreadline")
                .arg("-lncurses")
                .arg("-lz")
                .arg("-lffi")
                .arg("-lc++")
                .arg("-lzstd");
        } else if runtime_triple.is_windows() {
            cc.arg(&runtime_lib);
            // Link against Windows system libraries required by dependencies
            cc.arg("-lws2_32")
                .arg("-lpdh")
                .arg("-liphlpapi")
                .arg("-lnetapi32")
                .arg("-luserenv")
                .arg("-ladvapi32")
                .arg("-lpowrprof")
                .arg("-lole32")
                .arg("-loleaut32")
                .arg("-lpsapi")
                .arg("-lntdll")
                .arg("-lshell32")
                .arg("-lsecur32")
                .arg("-lbcrypt")
                .arg("-luser32");
        } else {
            cc.arg(&runtime_lib)
                .arg("-lstdc++")
                .arg("-lm")
                .arg("-ldl")
                .arg("-lpthread")
                .arg("-lz")
                .arg("-lxml2")
                .arg("-lffi")
                .arg("-lzstd");

            // LLVM's LineEditor requires libedit, try to link it
            // If not available, try readline which provides compatible history functions
            if check_library_available("edit") {
                cc.arg("-ledit");
            } else if check_library_available("readline") {
                cc.arg("-lreadline");
            } else {
                // Try both anyway - let the linker fail with a clear error if neither is installed
                cc.arg("-ledit");
            }

            cc.arg("-ltinfo");
        }
    }

    let status = cc.status().context("failed to invoke system linker (cc)")?;

    if !status.success() {
        bail!("linker invocation failed with status {status}");
    }

    // Clean up temporary files
    if let Some(ref rt_c) = runtime_c {
        fs::remove_file(rt_c)?;
    }
    if let Some(ref rt_o) = runtime_o {
        fs::remove_file(rt_o)?;
    }
    fs::remove_file(&object_path)?;

    Ok(BuildArtifact {
        binary: lib_path,
        ir: compiler.cached_ir.take(),
    })
}
