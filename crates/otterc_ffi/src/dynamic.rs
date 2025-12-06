use std::collections::HashMap;
use std::ffi::CString;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use cargo_metadata::{Metadata, MetadataCommand, Package};
use libloading::Library;
use sha1::{Digest, Sha1};
use tracing::debug;

use super::{FfiBackend, bootstrap_stdlib, register_dynamic_exports};
use crate::api;
use otterc_cache::path::cache_root;
use otterc_symbol::registry::SymbolRegistry;

type JsonDispatcher = unsafe extern "C" fn(
    *const std::os::raw::c_char,
    *const std::os::raw::c_char,
) -> *mut std::os::raw::c_char;

pub struct DynamicLibraryBackend {
    registry: &'static SymbolRegistry,
    libraries: HashMap<String, Library>,
    json_dispatchers: HashMap<String, JsonDispatcher>,
    ffi_root: PathBuf,
}

impl DynamicLibraryBackend {
    pub fn new() -> Result<Self> {
        let registry = bootstrap_stdlib();

        let ffi_root = match cache_root() {
            Ok(path) => path.join("ffi"),
            Err(_) => return Err(anyhow!("Failed to get cache root")),
        };
        fs::create_dir_all(&ffi_root).context("failed to create ffi cache directory")?;

        Ok(Self {
            registry,
            libraries: HashMap::new(),
            json_dispatchers: HashMap::new(),
            ffi_root,
        })
    }

    fn ensure_loaded(&mut self, crate_name: &str) -> Result<()> {
        if self.libraries.contains_key(crate_name) {
            return Ok(());
        }

        let (metadata, package) = resolve_package(crate_name)?;

        let fingerprint = fingerprint_package(&package)?;
        let lib_filename = libloading::library_filename(&package.name);

        let cache_dir = self.ffi_root.join(&package.name).join(&fingerprint);
        fs::create_dir_all(&cache_dir)
            .with_context(|| format!("failed to create ffi cache path {}", cache_dir.display()))?;
        let cached_lib = cache_dir.join(&lib_filename);

        if !cached_lib.exists() {
            build_crate(&metadata, &package)?;
            let built_path = metadata
                .target_directory
                .as_std_path()
                .join("release")
                .join(&lib_filename);
            fs::copy(&built_path, &cached_lib).with_context(|| {
                format!(
                    "failed to cache built library {} to {}",
                    built_path.display(),
                    cached_lib.display()
                )
            })?;
        }

        let library = unsafe { Library::new(&cached_lib) }
            .with_context(|| format!("failed to load ffi library {}", cached_lib.display()))?;

        if let Err(err) = register_dynamic_exports(&library, self.registry) {
            debug!(error = %err, "ffi register failed");
        }

        if let Ok(dispatcher) = unsafe { library.get::<JsonDispatcher>(b"otter_call_json") } {
            self.json_dispatchers
                .insert(crate_name.to_string(), *dispatcher);
        }

        self.libraries.insert(crate_name.to_string(), library);

        Ok(())
    }

    fn call_json_inner(&mut self, crate_name: &str, func: &str, args_json: &str) -> Result<String> {
        self.ensure_loaded(crate_name)?;
        let dispatcher = self
            .json_dispatchers
            .get(crate_name)
            .ok_or_else(|| anyhow!("crate `{crate_name}` does not expose otter_call_json"))?;

        let func_cstr = CString::new(func)
            .with_context(|| format!("failed to convert function name `{func}` to CString"))?;
        let args_cstr = CString::new(args_json)
            .with_context(|| format!("failed to convert args JSON for `{func}`"))?;

        let raw = unsafe { dispatcher(func_cstr.as_ptr(), args_cstr.as_ptr()) };
        if raw.is_null() {
            bail!("json dispatcher returned null for `{crate_name}:{func}`");
        }

        let result = unsafe { api::cstring_to_otter(raw) }?;
        Ok(result)
    }
}

impl FfiBackend for DynamicLibraryBackend {
    fn symbols(&self) -> &SymbolRegistry {
        self.registry
    }

    fn load_crate(&mut self, crate_name: &str) -> Result<()> {
        self.ensure_loaded(crate_name)
    }

    fn call_json(&mut self, crate_name: &str, func: &str, args_json: &str) -> Result<String> {
        self.call_json_inner(crate_name, func, args_json)
    }
}

fn build_crate(metadata: &Metadata, package: &Package) -> Result<()> {
    let manifest_path = package.manifest_path.as_std_path();
    let status = duct::cmd!(
        "cargo",
        "build",
        "--release",
        "--manifest-path",
        manifest_path
    )
    .dir(
        manifest_path
            .parent()
            .unwrap_or_else(|| metadata.workspace_root.as_std_path()),
    )
    .run()
    .context("failed to build ffi crate")?;

    if !status.status.success() {
        bail!("cargo build failed for crate {}", package.name);
    }

    Ok(())
}

fn fingerprint_package(package: &Package) -> Result<String> {
    let manifest = fs::read(&package.manifest_path).with_context(|| {
        format!(
            "failed to read manifest {}",
            package.manifest_path.as_std_path().display()
        )
    })?;
    let mut hasher = Sha1::new();
    hasher.update(manifest);
    hasher.update(package.version.to_string().as_bytes());
    Ok(format!("{:x}", hasher.finalize()))
}

fn resolve_package(crate_name: &str) -> Result<(Metadata, Package)> {
    let metadata = MetadataCommand::new()
        .exec()
        .context("failed to execute cargo metadata")?;

    if let Some(package) = metadata
        .packages
        .iter()
        .find(|pkg| pkg.name == crate_name)
        .cloned()
    {
        return Ok((metadata, package));
    }

    let workspace_root = metadata.workspace_root.as_std_path();
    let alt_manifest = workspace_root
        .join("ffi")
        .join(crate_name)
        .join("Cargo.toml");

    if alt_manifest.exists() {
        let alt_metadata = MetadataCommand::new()
            .manifest_path(&alt_manifest)
            .exec()
            .with_context(|| {
                format!("failed to execute cargo metadata for crate `{crate_name}`")
            })?;
        let package = alt_metadata
            .packages
            .iter()
            .find(|pkg| pkg.name == crate_name)
            .cloned()
            .ok_or_else(|| anyhow!("crate `{crate_name}` not found in metadata"))?;
        return Ok((alt_metadata, package));
    }

    Err(anyhow!("crate `{crate_name}` not found"))
}
