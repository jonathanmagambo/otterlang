use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use serde_json::Value as JsonValue;
use serde_yaml::Value as YamlValue;

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

fn normalize_yaml(text: &str) -> Option<String> {
    serde_yaml::from_str::<YamlValue>(text)
        .ok()
        .and_then(|value| serde_yaml::to_string(&value).ok())
}

fn yaml_to_json(text: &str) -> Option<String> {
    serde_yaml::from_str::<YamlValue>(text)
        .ok()
        .and_then(|value| serde_json::to_string(&value).ok())
}

fn json_to_yaml(text: &str) -> Option<String> {
    serde_json::from_str::<JsonValue>(text)
        .ok()
        .and_then(|value| serde_yaml::to_string(&value).ok())
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_yaml_normalize(input: *const c_char) -> *mut c_char {
    read_c_string(input)
        .and_then(|text| normalize_yaml(&text))
        .map_or(std::ptr::null_mut(), into_c_string)
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_yaml_validate(input: *const c_char) -> bool {
    read_c_string(input)
        .map(|text| serde_yaml::from_str::<YamlValue>(&text).is_ok())
        .unwrap_or(false)
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_yaml_to_json(input: *const c_char) -> *mut c_char {
    read_c_string(input)
        .and_then(|text| yaml_to_json(&text))
        .map_or(std::ptr::null_mut(), into_c_string)
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_yaml_from_json(input: *const c_char) -> *mut c_char {
    read_c_string(input)
        .and_then(|text| json_to_yaml(&text))
        .map_or(std::ptr::null_mut(), into_c_string)
}

fn register_std_yaml_symbols(registry: &SymbolRegistry) {
    let normalize_sig = FfiSignature::new(vec![FfiType::Str], FfiType::Str);
    let validate_sig = FfiSignature::new(vec![FfiType::Str], FfiType::Bool);
    let convert_sig = FfiSignature::new(vec![FfiType::Str], FfiType::Str);

    for name in [
        "std.yaml.normalize",
        "yaml.normalize",
        "std_yaml_normalize",
        "yaml_normalize",
    ] {
        registry.register(FfiFunction {
            name: name.into(),
            symbol: "otter_std_yaml_normalize".into(),
            signature: normalize_sig.clone(),
        });
    }

    for name in [
        "std.yaml.validate",
        "yaml.validate",
        "std_yaml_validate",
        "yaml_validate",
    ] {
        registry.register(FfiFunction {
            name: name.into(),
            symbol: "otter_std_yaml_validate".into(),
            signature: validate_sig.clone(),
        });
    }

    for name in [
        "std.yaml.to_json",
        "yaml.to_json",
        "std_yaml_to_json",
        "yaml_to_json",
    ] {
        registry.register(FfiFunction {
            name: name.into(),
            symbol: "otter_std_yaml_to_json".into(),
            signature: convert_sig.clone(),
        });
    }

    for name in [
        "std.yaml.from_json",
        "yaml.from_json",
        "std_yaml_from_json",
        "yaml_from_json",
    ] {
        registry.register(FfiFunction {
            name: name.into(),
            symbol: "otter_std_yaml_from_json".into(),
            signature: convert_sig.clone(),
        });
    }
}

inventory::submit! {
    otterc_ffi::SymbolProvider {
        namespace: "yaml",
        autoload: false,
        register: register_std_yaml_symbols,
    }
}
