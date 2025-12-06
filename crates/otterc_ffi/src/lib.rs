//! OtterLang Rust FFI bridge modules.
//!
//! This module hosts the scaffolding for the cargo bridge pipeline that turns
//! `use rust:crate` imports into dynamically loaded shared libraries.

pub mod api;
pub mod cargo_bridge;
pub mod dynamic;
pub mod dynamic_loader;
pub mod exports;
pub mod metadata;
pub mod providers;
pub mod rust_stubgen;
pub mod rustdoc_extractor;
pub mod symbol_registry;
pub mod types;

pub use cargo_bridge::{BridgeArtifacts, CargoBridge};
pub use dynamic::DynamicLibraryBackend;
pub use dynamic_loader::{DynamicLibrary, DynamicLibraryLoader};
pub use exports::{ExportFn, StableExportSet, StableFunction, register_dynamic_exports};
pub use metadata::load_bridge_functions;
pub use providers::{SymbolProvider, bootstrap_stdlib};

use otterc_symbol::registry::SymbolRegistry;

use anyhow::Result;
pub use rust_stubgen::RustStubGenerator;
pub use rustdoc_extractor::{
    extract_crate_spec, extract_crate_spec_from_json, generate_rustdoc_json,
};
pub use symbol_registry::{BridgeFunction, BridgeSymbolRegistry};
pub use types::{
    BridgeMetadata, CallTemplate, CrateSpec, DependencyConfig, EnumVariant, EnumVariantKind, FnSig,
    FunctionSpec, PublicItem, RustPath, RustTypeRef, StructField, StubSource, TraitMethod,
    TypeSpec,
};

pub trait FfiBackend {
    fn symbols(&self) -> &SymbolRegistry;
    fn load_crate(&mut self, crate_name: &str) -> Result<()>;
    fn call_json(&mut self, crate_name: &str, func: &str, args_json: &str) -> Result<String>;
}

/// Returns a boxed backend that uses dynamic libraries for FFI.
pub fn new_backend() -> Result<Box<dyn FfiBackend>> {
    Ok(Box::new(DynamicLibraryBackend::new()?))
}
