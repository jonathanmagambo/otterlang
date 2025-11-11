use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use tracing::{debug, info, warn};

use crate::codegen::{
    self, build_executable, BuildArtifact, CodegenOptLevel, CodegenOptions, TargetTriple,
};
use crate::runtime::ffi;
use crate::typecheck::TypeChecker;
use crate::version::VERSION;
use cache::{CacheBuildOptions, CacheEntry, CacheManager, CacheMetadata, CompilationInputs};
use language::LanguageFeatureFlags;
use lexer::{tokenize, LexerError};
use module::ModuleProcessor;
use parser::{parse, ParserError};
use utils::errors::{emit_diagnostics, Diagnostic};
use utils::logger;
use utils::profiler::{PhaseTiming, Profiler};

#[derive(Parser, Debug)]
#[command(name = "otter", version = VERSION, about = "OtterLang compiler")]
pub struct OtterCli {
    #[arg(long, global = true)]
    /// Dump the token stream before parsing.
    dump_tokens: bool,

    #[arg(long, global = true)]
    /// Dump the parsed AST before code generation.
    dump_ast: bool,

    #[arg(long, global = true)]
    /// Dump the generated LLVM IR.
    dump_ir: bool,

    #[arg(long, global = true)]
    /// Display phase timing information.
    time: bool,

    #[arg(long, global = true)]
    /// Emit profiling summary for the compilation.
    profile: bool,

    #[arg(long, global = true)]
    /// Enable release mode (O3 + LTO) when building binaries.
    release: bool,

    #[arg(long, global = true)]
    /// Enable the experimental async task runtime when executing programs.
    tasks: bool,

    #[arg(long, global = true)]
    /// Emit verbose scheduler diagnostics from the task runtime.
    tasks_debug: bool,

    #[arg(long, global = true)]
    /// Trace task lifecycle events from the runtime.
    tasks_trace: bool,

    #[arg(long, global = true)]
    /// Enable debug mode with stack traces.
    debug: bool,

    #[arg(long, global = true)]
    /// Disable cache for this compilation.
    no_cache: bool,

    #[arg(long, global = true, value_name = "list")]
    /// Enable experimental language features (comma-separated names or use OTTER_FEATURES env var).
    features: Option<String>,

    #[arg(long, global = true)]
    /// Target triple for cross-compilation (e.g., wasm32-unknown-unknown, thumbv7m-none-eabi)
    target: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Lexes, parses, and executes the specified source file via the cached native pipeline.
    Run { path: PathBuf },
    /// Builds a native executable from the specified source file.
    Build {
        path: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Start an interactive REPL (Read-Eval-Print Loop).
    Repl,
    /// Format OtterLang source code.
    Fmt {
        /// Files to format (defaults to all .ot files in current directory)
        #[arg(default_value = ".")]
        paths: Vec<PathBuf>,
    },
    /// Profile OtterLang programs (memory or performance)
    Profile {
        #[command(subcommand)]
        subcommand: crate::tools::profiler::ProfileCommand,
    },
}

pub fn run() -> Result<()> {
    logger::init_logging();
    ffi::bootstrap_stdlib();
    let cli = OtterCli::parse();

    match &cli.command {
        Command::Run { path } => handle_run(&cli, path),
        Command::Build { path, output } => handle_build(&cli, path, output.clone()),
        Command::Repl => handle_repl(),
        Command::Fmt { paths } => handle_fmt(paths),
        Command::Profile { subcommand } => {
            crate::tools::profiler::run_profiler_subcommand(subcommand)
        }
    }
}

fn handle_run(cli: &OtterCli, path: &Path) -> Result<()> {
    let settings = CompilationSettings::from_cli(cli);
    let source = read_source(path)?;
    let stage = compile_pipeline(path, &source, &settings)?;

    match &stage.result {
        CompilationResult::CacheHit(entry) => {
            println!("cache hit ({} bytes)", entry.metadata.binary_size);
            if settings.profile {
                print_profile(&entry.metadata);
            }
            execute_binary(&entry.binary_path, &settings)?;
        }
        CompilationResult::Compiled { artifact, metadata } => {
            println!("building {}", artifact.binary.display());
            execute_binary(&artifact.binary, &settings)?;
            if settings.dump_ir {
                if let Some(ir) = &artifact.ir {
                    println!("\n== LLVM IR ==");
                    println!("{ir}");
                }
            }
            if settings.profile {
                print_profile(metadata);
            }
        }
    }

    if settings.time {
        print_timings(&stage);
    }

    Ok(())
}

fn handle_build(cli: &OtterCli, path: &Path, output: Option<PathBuf>) -> Result<()> {
    let settings = CompilationSettings::from_cli(cli);
    let source = read_source(path)?;
    let stage = compile_pipeline(path, &source, &settings)?;

    let output_path = resolve_output_path(path, output);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory {}", parent.display()))?;
    }

    let cached_binary = match &stage.result {
        CompilationResult::CacheHit(entry) => &entry.binary_path,
        CompilationResult::Compiled { artifact, .. } => &artifact.binary,
    };

    fs::copy(cached_binary, &output_path).with_context(|| {
        format!(
            "failed to copy cached binary {} to {}",
            cached_binary.display(),
            output_path.display()
        )
    })?;

    println!("built {}", output_path.display());

    match &stage.result {
        CompilationResult::Compiled { artifact, metadata } => {
            if settings.dump_ir {
                if let Some(ir) = &artifact.ir {
                    println!("\n== LLVM IR ==");
                    println!("{ir}");
                }
            }
            if settings.profile {
                print_profile(metadata);
            }
        }
        CompilationResult::CacheHit(entry) => {
            if settings.profile {
                print_profile(&entry.metadata);
            }
        }
    }

    if settings.time {
        print_timings(&stage);
    }

    Ok(())
}

fn compile_pipeline(
    path: &Path,
    source: &str,
    settings: &CompilationSettings,
) -> Result<CompilationStage> {
    let mut cache_manager = CacheManager::new();
    let cache_options = settings.cache_build_options();
    let mut profiler = Profiler::new();
    let source_id = path.display().to_string();
    let source_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();

    // Try to find stdlib directory
    let stdlib_dir = find_stdlib_dir().ok();

    // Initial inputs without module dependencies (will be updated after parsing)
    let mut inputs = CompilationInputs::new(path.to_path_buf(), Vec::new());

    // Generate initial cache key for quick lookup check
    let initial_cache_key = profiler.record_phase("Fingerprint", || {
        cache_manager.fingerprint(&inputs, &cache_options, VERSION)
    });

    if settings.allow_cache() {
        if let Some(entry) =
            profiler.record_phase("Cache lookup", || cache_manager.lookup(&initial_cache_key))
        {
            debug!(cache_hit = %entry.binary_path.display());
            profiler.push_phase("Compile skipped", Duration::from_millis(0));
            return Ok(CompilationStage {
                profiler,
                result: CompilationResult::CacheHit(entry),
            });
        }
    }

    let tokens = match profiler.record_phase("Lexing", || tokenize(source)) {
        Ok(tokens) => tokens,
        Err(errors) => {
            emit_lexer_errors(&source_id, source, &errors);
            bail!("lexing failed");
        }
    };

    if settings.dump_tokens {
        println!("\n== Tokens ==");
        for token in &tokens {
            println!("  {:?} @ {:?}", token.kind, token.span);
        }
    }

    let program = match profiler.record_phase("Parsing", || parse(&tokens)) {
        Ok(program) => {
            if settings.debug {
                println!("Parsed successfully");
            }
            program
        }
        Err(errors) => {
            emit_parser_errors(&source_id, source, &errors);
            bail!("parsing failed");
        }
    };

    if settings.dump_ast {
        println!("\n== AST ==");
        println!("{:#?}", program);
    }

    // Process module imports
    let mut module_processor = ModuleProcessor::new(source_dir.clone(), stdlib_dir.clone());
    let module_deps = profiler.record_phase("Module Resolution", || {
        module_processor.process_imports(&program)
    })?;

    // Resolve re-exports after all modules are loaded
    profiler.record_phase("Re-export Resolution", || {
        module_processor.resolve_all_re_exports()
    })?;

    // Type check the program
    let mut type_checker =
        TypeChecker::with_language_features(settings.language_features().clone())
            .with_registry(crate::runtime::symbol_registry::SymbolRegistry::global());

    for module in module_processor.modules() {
        type_checker.register_module_definitions(&module.program);
    }
    let type_check_result =
        profiler.record_phase("Type Checking", || type_checker.check_program(&program));

    if let Err(err) = type_check_result {
        println!("\n== Type Errors ==");
        for error in type_checker.errors() {
            println!("  {}", error);
        }
        return Err(err).with_context(|| "type checking failed");
    }

    let expr_types = type_checker.into_expr_type_map();

    // Update inputs with module dependencies for accurate cache fingerprinting
    inputs.imports = module_deps
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    let cache_key = profiler.record_phase("Fingerprint (with modules)", || {
        cache_manager.fingerprint(&inputs, &cache_options, VERSION)
    });

    // Check cache again with module dependencies included
    if settings.allow_cache() {
        if let Some(entry) = profiler.record_phase("Cache lookup (with modules)", || {
            cache_manager.lookup(&cache_key)
        }) {
            debug!(cache_hit = %entry.binary_path.display());
            profiler.push_phase("Compile skipped", Duration::from_millis(0));
            return Ok(CompilationStage {
                profiler,
                result: CompilationResult::CacheHit(entry),
            });
        }
    }

    let codegen_options = settings.codegen_options();
    let binary_path = cache_manager
        .binary_path(&cache_key)
        .unwrap_or_else(|| PathBuf::from("./target/tmp_binary"));

    let artifact = profiler.record_phase("LLVM Codegen", || {
        build_executable(&program, &expr_types, &binary_path, &codegen_options)
    })?;

    let build_duration_ms = profiler
        .phases()
        .last()
        .map(|phase| phase.duration.as_millis())
        .unwrap_or_default();

    let binary_size = std::fs::metadata(&artifact.binary)?.len();

    let metadata = CacheMetadata::new(
        cache_key.clone(),
        VERSION,
        codegen::current_llvm_version(),
        canonical_or(path),
        inputs.dependencies.clone(),
        artifact.binary.clone(),
        binary_size,
        build_duration_ms as u64,
        PathBuf::from("./cache"), // cache_path
        inputs.imports.clone(),
    );

    if let Err(e) = cache_manager.store(&metadata) {
        warn!("Failed to store cache entry: {}", e);
    }

    info!(compiled = %artifact.binary.display(), size = binary_size);

    Ok(CompilationStage {
        profiler,
        result: CompilationResult::Compiled { artifact, metadata },
    })
}

struct CompilationStage {
    profiler: Profiler,
    result: CompilationResult,
}

enum CompilationResult {
    CacheHit(CacheEntry),
    Compiled {
        artifact: BuildArtifact,
        metadata: CacheMetadata,
    },
}

impl CompilationStage {
    fn timings(&self) -> &[PhaseTiming] {
        self.profiler.phases()
    }
}

#[derive(Clone)]
struct CompilationSettings {
    dump_tokens: bool,
    dump_ast: bool,
    dump_ir: bool,
    time: bool,
    profile: bool,
    release: bool,
    tasks: bool,
    tasks_debug: bool,
    tasks_trace: bool,
    debug: bool,
    target: Option<String>,
    no_cache: bool,
    enable_cache: bool,
    cache_dir: PathBuf,
    max_cache_size: usize,
    language_features: LanguageFeatureFlags,
}

impl CompilationSettings {
    fn from_cli(cli: &OtterCli) -> Self {
        let language_features = resolve_language_features(cli);
        Self {
            dump_tokens: cli.dump_tokens,
            dump_ast: cli.dump_ast,
            dump_ir: cli.dump_ir,
            time: cli.time,
            profile: cli.profile,
            release: cli.release,
            tasks: cli.tasks,
            tasks_debug: cli.tasks_debug,
            tasks_trace: cli.tasks_trace,
            debug: cli.debug,
            target: cli.target.clone(),
            no_cache: cli.no_cache,
            enable_cache: !cli.no_cache,
            cache_dir: PathBuf::from("./cache"),
            max_cache_size: 1024 * 1024 * 1024, // 1GB default
            language_features,
        }
    }

    fn allow_cache(&self) -> bool {
        !(self.dump_tokens || self.dump_ast || self.dump_ir || self.no_cache)
    }

    fn cache_build_options(&self) -> CacheBuildOptions {
        CacheBuildOptions {
            enable_cache: self.enable_cache,
            cache_dir: self.cache_dir.clone(),
            max_cache_size: self.max_cache_size,
            release: self.release,
            lto: self.release,
            emit_ir: self.dump_ir,
        }
    }

    fn codegen_options(&self) -> CodegenOptions {
        let target = self.target.as_ref().and_then(|t| {
            TargetTriple::parse(t)
                .map_err(|e| {
                    eprintln!("Warning: Invalid target triple '{}': {}", t, e);
                })
                .ok()
        });

        CodegenOptions {
            emit_ir: self.dump_ir,
            opt_level: if self.release {
                CodegenOptLevel::Aggressive
            } else {
                CodegenOptLevel::Default
            },
            enable_lto: self.release,
            enable_pgo: false,
            pgo_profile_file: None,
            inline_threshold: None,
            target,
        }
    }

    fn language_features(&self) -> &LanguageFeatureFlags {
        &self.language_features
    }
}

fn resolve_language_features(cli: &OtterCli) -> LanguageFeatureFlags {
    let mut flags = LanguageFeatureFlags::default();

    if let Ok(env_value) = std::env::var("OTTER_FEATURES") {
        apply_feature_list(&env_value, &mut flags, "OTTER_FEATURES");
    }

    if let Some(cli_value) = cli.features.as_deref() {
        apply_feature_list(cli_value, &mut flags, "--features");
    }

    if flags.any_enabled() {
        let enabled = collect_enabled_feature_names(&flags).join(", ");
        info!("language features enabled: {}", enabled);
    }

    flags
}

fn apply_feature_list(source: &str, flags: &mut LanguageFeatureFlags, label: &str) {
    for raw in source.split(',') {
        for token in raw.split_whitespace() {
            let feature = token.trim();
            if feature.is_empty() {
                continue;
            }
            if !flags.enable(feature) {
                warn!("unknown language feature '{}' from {}", feature, label);
            }
        }
    }
}

fn collect_enabled_feature_names(flags: &LanguageFeatureFlags) -> Vec<&'static str> {
    let mut names = Vec::new();
    if flags.result_option_core {
        names.push(LanguageFeatureFlags::RESULT_OPTION_CORE);
    }
    if flags.match_exhaustiveness {
        names.push(LanguageFeatureFlags::MATCH_EXHAUSTIVENESS);
    }
    if flags.newtype_aliases {
        names.push(LanguageFeatureFlags::NEWTYPE_ALIASES);
    }
    names
}

fn read_source(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
}

fn resolve_output_path(path: &Path, output: Option<PathBuf>) -> PathBuf {
    output.unwrap_or_else(|| {
        let mut candidate = path.with_extension("");
        if candidate.file_name().is_none() {
            candidate = PathBuf::from("otter.out");
        }

        #[cfg(target_os = "windows")]
        {
            candidate.set_extension("exe");
        }

        candidate
    })
}

fn canonical_or(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn find_stdlib_dir() -> Result<PathBuf> {
    // Try environment variable first
    if let Ok(dir) = std::env::var("OTTER_STDLIB_DIR") {
        let path = PathBuf::from(dir);
        if path.exists() {
            return Ok(path);
        }
    }

    // Try relative to executable (for development)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let stdlib = exe_dir
                .parent()
                .unwrap_or(exe_dir)
                .join("stdlib")
                .join("otter");
            if stdlib.exists() {
                return Ok(stdlib);
            }
        }
    }

    // Try relative to current directory (for development)
    let stdlib = PathBuf::from("stdlib").join("otter");
    if stdlib.exists() {
        return Ok(stdlib);
    }

    bail!("stdlib directory not found. Set OTTER_STDLIB_DIR environment variable or ensure stdlib/otter exists")
}

fn execute_binary(path: &Path, settings: &CompilationSettings) -> Result<()> {
    if settings.debug {
        println!("Running program: {}", path.display());
    }

    let mut command = ProcessCommand::new(path);

    if settings.tasks {
        command.env("OTTER_TASKS_DIAGNOSTICS", "1");
    }
    if settings.tasks_debug {
        command.env("OTTER_TASKS_DEBUG", "1");
    }
    if settings.tasks_trace {
        command.env("OTTER_TASKS_TRACE", "1");
    }
    if settings.debug {
        command.env("RUST_BACKTRACE", "1");
        command.env("OTTER_DEBUG", "1");
    }

    let status = command
        .status()
        .with_context(|| format!("failed to execute {}", path.display()))?;

    if !status.success() {
        if settings.debug {
            eprintln!("\nStack trace:");
            eprintln!("  Exit status: {}", status);
        }
        bail!("program exited with status {status}");
    }

    Ok(())
}

fn print_timings(stage: &CompilationStage) {
    println!("\nTimings:");
    let mut total = Duration::ZERO;
    for PhaseTiming { name: _, duration } in stage.timings() {
        total += *duration;
    }
    for PhaseTiming { name, duration } in stage.timings() {
        let pct = if total.as_secs_f64() > 0.0 {
            (duration.as_secs_f64() / total.as_secs_f64()) * 100.0
        } else {
            0.0
        };
        println!(
            "  {:20} {:8.2}ms ({:5.1}%)",
            name,
            duration.as_secs_f64() * 1000.0,
            pct
        );
    }
    println!(
        "  {:20} {:8.2}ms",
        "Total",
        total.as_secs_f64() * 1000.0
    );
}

fn handle_fmt(paths: &[PathBuf]) -> Result<()> {
    use fmt::Formatter;
    use glob::glob;
    use lexer::tokenize;
    use parser::parse;

    println!("Formatting OtterLang files...");

    let formatter = Formatter::new();
    let mut formatted_count = 0;

    // Collect all .ot files
    let mut files = Vec::new();
    if paths.is_empty() || (paths.len() == 1 && paths[0].to_str() == Some(".")) {
        // Default: format all .ot files in current directory recursively
        for entry in glob("**/*.ot")? {
            if let Ok(path) = entry {
                files.push(path);
            }
        }
    } else {
        for path in paths {
            if path.is_dir() {
                for entry in glob(&format!("{}/**/*.ot", path.display()))? {
                    if let Ok(p) = entry {
                        files.push(p);
                    }
                }
            } else if path.extension().map_or(false, |ext| ext == "ot") {
                files.push(path.clone());
            }
        }
    }

    for file_path in files {
        let source = fs::read_to_string(&file_path)
            .with_context(|| format!("failed to read {}", file_path.display()))?;

        let tokens = tokenize(&source)
            .map_err(|_| anyhow::anyhow!("failed to tokenize {}", file_path.display()))?;

        let program = parse(&tokens)
            .map_err(|_| anyhow::anyhow!("failed to parse {}", file_path.display()))?;

        let formatted = formatter.format_program(&program);

        if formatted != source {
            fs::write(&file_path, formatted)
                .with_context(|| format!("failed to write {}", file_path.display()))?;
            println!("  {}", file_path.display());
            formatted_count += 1;
        }
    }

    if formatted_count == 0 {
        println!("All files are already formatted");
    } else {
        println!("\nFormatted {} file(s)", formatted_count);
    }

    Ok(())
}

fn handle_repl() -> Result<()> {
    use crate::repl::{ReplEngine, Tui};
    
    let engine = ReplEngine::new();
    match Tui::new(engine) {
        Ok(mut tui) => {
            if let Err(e) = tui.run() {
                eprintln!("TUI error: {}", e);
                eprintln!("Error chain: {:?}", e);
                return Err(e).with_context(|| "TUI runtime error");
            }
            Ok(())
        }
        Err(e) => {
            eprintln!("Failed to initialize TUI: {}", e);
            eprintln!("Error chain: {:?}", e);
            Err(e).with_context(|| "Failed to initialize TUI. Make sure you're running in a terminal.")
        }
    }
}

fn print_profile(metadata: &CacheMetadata) {
    println!("\nProfile:");
    println!("  Binary: {}", metadata.binary_path.display());
    println!("  Size:   {} bytes", metadata.binary_size);
    println!("  Build:  {} ms", metadata.build_time_ms);
    if let Some(version) = &metadata.llvm_version {
        println!("  LLVM:   {}", version);
    }
}

fn emit_lexer_errors(source_id: &str, source: &str, errors: &[LexerError]) {
    println!("\nLexical errors:");
    let diagnostics: Vec<Diagnostic> = errors
        .iter()
        .map(|err| err.to_diagnostic(source_id))
        .collect();
    emit_diagnostics(&diagnostics, source);
}

fn emit_parser_errors(source_id: &str, source: &str, errors: &[ParserError]) {
    println!("\nParsing errors:");
    let diagnostics: Vec<Diagnostic> = errors
        .iter()
        .map(|err| err.to_diagnostic(source_id))
        .collect();
    emit_diagnostics(&diagnostics, source);
}
