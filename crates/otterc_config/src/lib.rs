pub mod target;

pub use crate::target::TargetTriple;
use inkwell::OptimizationLevel;
use std::path::PathBuf;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LanguageFeatureFlags {
    pub result_option_core: bool,
    pub match_exhaustiveness: bool,
    pub newtype_aliases: bool,
}

impl LanguageFeatureFlags {
    pub const RESULT_OPTION_CORE: &'static str = "result_option_core";
    pub const MATCH_EXHAUSTIVENESS: &'static str = "match_exhaustiveness";
    pub const NEWTYPE_ALIASES: &'static str = "newtype_aliases";

    pub fn enable(&mut self, feature: &str) -> bool {
        match feature {
            Self::RESULT_OPTION_CORE => {
                self.result_option_core = true;
                true
            }
            Self::MATCH_EXHAUSTIVENESS => {
                self.match_exhaustiveness = true;
                true
            }
            Self::NEWTYPE_ALIASES => {
                self.newtype_aliases = true;
                true
            }
            _ => false,
        }
    }

    pub fn any_enabled(&self) -> bool {
        self.result_option_core || self.match_exhaustiveness || self.newtype_aliases
    }
}

/// Codegen optimization level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodegenOptLevel {
    None,
    Default,
    Aggressive,
}

/// Codegen options
#[derive(Debug, Clone)]
pub struct CodegenOptions {
    pub emit_ir: bool,
    pub opt_level: CodegenOptLevel,
    pub enable_lto: bool,
    pub enable_pgo: bool,
    pub pgo_profile_file: Option<PathBuf>,
    pub inline_threshold: Option<u32>,
    /// Target triple for cross-compilation (defaults to native)
    pub target: Option<TargetTriple>,
}

impl Default for CodegenOptions {
    fn default() -> Self {
        Self {
            emit_ir: false,
            opt_level: CodegenOptLevel::Default,
            enable_lto: false,
            enable_pgo: false,
            pgo_profile_file: None,
            inline_threshold: None,
            target: None,
        }
    }
}

impl From<CodegenOptLevel> for OptimizationLevel {
    fn from(value: CodegenOptLevel) -> Self {
        match value {
            CodegenOptLevel::None => OptimizationLevel::None,
            CodegenOptLevel::Default => OptimizationLevel::Default,
            CodegenOptLevel::Aggressive => OptimizationLevel::Aggressive,
        }
    }
}
