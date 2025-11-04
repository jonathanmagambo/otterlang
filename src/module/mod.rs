//! Module system for OtterLang
//!
//! Handles module resolution, loading, and dependency tracking for .otter files

pub mod loader;
pub mod processor;
pub mod resolver;

pub use loader::{Module, ModuleExports, ModuleLoader};
pub use processor::ModuleProcessor;
pub use resolver::{DependencyGraph, ModulePath, ModuleResolver};
