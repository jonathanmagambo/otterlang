use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use serde_json::Value;

use otterc_symbol::registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

fn read_c_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }

    unsafe { CStr::from_ptr(ptr).to_str().ok().map(|s| s.to_string()) }
}

fn into_c_string<S: Into<String>>(value: S) -> *mut c_char {
    CString::new(value.into())
        .ok()
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

fn normalize_json(text: &str) -> Option<String> {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| serde_json::to_string(&value).ok())
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_json_encode(obj: *const c_char) -> *mut c_char {
    if let Some(text) = read_c_string(obj) {
        normalize_json(&text).map_or(std::ptr::null_mut(), into_c_string)
    } else {
        std::ptr::null_mut()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_json_decode(json_str: *const c_char) -> *mut c_char {
    if let Some(text) = read_c_string(json_str) {
        normalize_json(&text).map_or(std::ptr::null_mut(), into_c_string)
    } else {
        std::ptr::null_mut()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_json_pretty(json_str: *const c_char) -> *mut c_char {
    read_c_string(json_str)
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .map_or(std::ptr::null_mut(), into_c_string)
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_json_validate(json_str: *const c_char) -> bool {
    read_c_string(json_str)
        .map(|text| serde_json::from_str::<Value>(&text).is_ok())
        .unwrap_or(false)
}

fn register_std_json_symbols(registry: &SymbolRegistry) {
    registry.register(FfiFunction {
        name: "std.json.encode".into(),
        symbol: "otter_std_json_encode".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "std.json.decode".into(),
        symbol: "otter_std_json_decode".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "std.json.pretty".into(),
        symbol: "otter_std_json_pretty".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "std.json.validate".into(),
        symbol: "otter_std_json_validate".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Bool),
    });
}

inventory::submit! {
    otterc_ffi::SymbolProvider {
        namespace: "json",
        autoload: false,
        register: register_std_json_symbols,
    }
}
