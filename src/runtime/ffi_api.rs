use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use anyhow::{Context, Result};

/// Converts an OtterLang UTF-8 string into a raw C string suitable for FFI calls.
pub fn otter_to_cstring(input: &str) -> Result<*mut c_char> {
    Ok(CString::new(input)
        .with_context(|| format!("failed to create CString from `{}`", input))?
        .into_raw())
}

/// Reconstructs a Rust `String` from an FFI-owned pointer and frees it using the
/// standard `CString::from_raw` workflow.
///
/// # Safety
/// The caller must ensure `ptr` originated from a compatible `CString::into_raw`
/// allocation or an equivalent allocator contract.
pub unsafe fn cstring_to_otter(ptr: *mut c_char) -> Result<String> { unsafe {
    if ptr.is_null() {
        return Ok(String::new());
    }

    let value = CStr::from_ptr(ptr)
        .to_str()
        .context("failed to decode UTF-8 from ffi pointer")?
        .to_owned();
    drop(CString::from_raw(ptr));
    Ok(value)
}}

/// Convenience helper that frees a pointer returned from the bridge without
/// converting it back into a `String`.
///
/// # Safety
/// Pointer must be valid for the `CString::from_raw` contract.
pub unsafe fn free_cstring(ptr: *mut c_char) { unsafe {
    if ptr.is_null() {
        return;
    }
    drop(CString::from_raw(ptr));
}}
