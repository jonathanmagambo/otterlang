use abi_stable::StableAbi;
use abi_stable::std_types::{RString, RVec};
use anyhow::{Context, Result};
use libloading::Library;

use otterc_symbol::registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

#[repr(C)]
#[derive(Clone, StableAbi)]
pub struct StableFunction {
    pub name: RString,
    pub symbol: RString,
    pub params: RVec<FfiType>,
    pub result: FfiType,
}

#[repr(C)]
#[derive(Clone, StableAbi)]
pub struct StableExportSet {
    pub functions: RVec<StableFunction>,
}

pub type ExportFn = extern "C" fn() -> StableExportSet;

pub fn register_dynamic_exports(library: &Library, registry: &SymbolRegistry) -> Result<()> {
    unsafe {
        let exports = library
            .get::<ExportFn>(b"otterlang_exports")
            .context("ffi module missing otterlang_exports symbol")?;
        let set = exports();
        for function in set.functions.into_iter() {
            registry.register(FfiFunction {
                name: function.name.into_string(),
                symbol: function.symbol.into_string(),
                signature: FfiSignature::new(function.params.into_vec(), function.result),
            });
        }
    }

    Ok(())
}
