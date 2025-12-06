use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use sysinfo::System;

use crate::runtime::symbol_registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_sys_cores() -> i64 {
    let mut system = System::new_all();
    system.refresh_cpu();
    system.cpus().len() as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_sys_total_memory_bytes() -> i64 {
    let mut system = System::new_all();
    system.refresh_memory();
    (system.total_memory() * 1024) as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_sys_available_memory_bytes() -> i64 {
    let mut system = System::new_all();
    system.refresh_memory();
    (system.available_memory() * 1024) as i64
}

/// get the env variable `name`
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_std_sys_getenv(name: *const c_char) -> *mut c_char {
    if name.is_null() {
        return std::ptr::null_mut();
    }

    let name_str = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .to_string();

    if let Ok(value) = std::env::var(&name_str) {
        CString::new(value)
            .ok()
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut())
    } else {
        std::ptr::null_mut()
    }
}

#[unsafe(no_mangle)]
#[expect(clippy::exit, reason = "TODO: Use a more graceful shutdown mechanism")]
pub extern "C" fn otter_std_sys_exit(code: i32) {
    std::process::exit(code);
}

fn register_std_sys_symbols(registry: &SymbolRegistry) {
    registry.register(FfiFunction {
        name: "std.sys.cores".into(),
        symbol: "otter_std_sys_cores".into(),
        signature: FfiSignature::new(vec![], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "std.sys.total_memory".into(),
        symbol: "otter_std_sys_total_memory_bytes".into(),
        signature: FfiSignature::new(vec![], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "std.sys.available_memory".into(),
        symbol: "otter_std_sys_available_memory_bytes".into(),
        signature: FfiSignature::new(vec![], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "sys.getenv".into(),
        symbol: "otter_std_sys_getenv".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "sys.exit".into(),
        symbol: "otter_std_sys_exit".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::Unit),
    });
}

inventory::submit! {
    crate::runtime::ffi::SymbolProvider {
        namespace: "sys",
        autoload: false,
        register: register_std_sys_symbols,
    }
}
