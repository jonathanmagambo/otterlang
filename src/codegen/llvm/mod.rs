pub mod bridges;
pub mod build;
pub mod compiler;
pub mod config;

pub use build::{build_executable, build_shared_library, current_llvm_version};
pub use config::{BuildArtifact, CodegenOptLevel, CodegenOptions};

#[cfg(feature = "cranelift-backend")]
pub(crate) use bridges::prepare_rust_bridges;
#[cfg(feature = "cranelift-backend")]
pub(crate) use config::preferred_target_flag;
