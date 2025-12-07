use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use crate::runtime::symbol_registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

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

fn http_get(url: &str) -> Option<String> {
    match ureq::get(url).call() {
        Ok(response) => response.into_string().ok(),
        Err(_) => None,
    }
}

fn http_post(url: &str, body: &str, content_type: &str) -> Option<String> {
    let request = ureq::post(url).set("Content-Type", content_type);
    match request.send_string(body) {
        Ok(response) => response.into_string().ok(),
        Err(_) => None,
    }
}

fn http_status(url: &str) -> Option<i64> {
    ureq::request("HEAD", url)
        .call()
        .map(|resp| resp.status() as i64)
        .ok()
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_http_get(url: *const c_char) -> *mut c_char {
    read_c_string(url)
        .and_then(|u| http_get(&u))
        .map_or(std::ptr::null_mut(), into_c_string)
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_http_post(
    url: *const c_char,
    body: *const c_char,
    content_type: *const c_char,
) -> *mut c_char {
    let Some(url) = read_c_string(url) else {
        return std::ptr::null_mut();
    };
    let body = read_c_string(body).unwrap_or_default();
    let content_type =
        read_c_string(content_type).unwrap_or_else(|| "application/json".to_string());
    http_post(&url, &body, &content_type).map_or(std::ptr::null_mut(), into_c_string)
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_std_http_head(url: *const c_char) -> i64 {
    read_c_string(url)
        .and_then(|u| http_status(&u))
        .unwrap_or(-1)
}

fn register_http_symbols(registry: &SymbolRegistry) {
    let get_sig = FfiSignature::new(vec![FfiType::Str], FfiType::Str);
    let post_sig = FfiSignature::new(vec![FfiType::Str, FfiType::Str, FfiType::Str], FfiType::Str);
    let head_sig = FfiSignature::new(vec![FfiType::Str], FfiType::I64);

    for name in ["std.http.get", "http.get", "std_http_get", "http_get"] {
        registry.register(FfiFunction {
            name: name.into(),
            symbol: "otter_std_http_get".into(),
            signature: get_sig.clone(),
        });
    }

    for name in ["std.http.post", "http.post", "std_http_post", "http_post"] {
        registry.register(FfiFunction {
            name: name.into(),
            symbol: "otter_std_http_post".into(),
            signature: post_sig.clone(),
        });
    }

    for name in ["std.http.head", "http.head", "std_http_head", "http_head"] {
        registry.register(FfiFunction {
            name: name.into(),
            symbol: "otter_std_http_head".into(),
            signature: head_sig.clone(),
        });
    }
}

inventory::submit! {
    crate::runtime::ffi::SymbolProvider {
        namespace: "http",
        autoload: false,
        register: register_http_symbols,
    }
}
