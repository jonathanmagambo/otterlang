use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use crate::runtime::memory::gc::{ObjectKind, get_gc};
use crate::runtime::symbol_registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

/// Format a float value to string
#[unsafe(no_mangle)]
pub extern "C" fn otter_format_float(value: f64) -> *mut c_char {
    let formatted = format!("{:.9}", value)
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string();
    let s = CString::new(formatted)
        .map(CString::into_raw)
        .unwrap_or_else(|_| std::ptr::null_mut());

    if !s.is_null() {
        unsafe {
            let len = std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len();
            get_gc().register_object(s as usize, len, ObjectKind::CString);
        }
    }
    s
}

/// Format an integer value to string
#[unsafe(no_mangle)]
pub extern "C" fn otter_format_int(value: i64) -> *mut c_char {
    let formatted = format!("{}", value);
    let s = CString::new(formatted)
        .map(CString::into_raw)
        .unwrap_or_else(|_| std::ptr::null_mut());

    if !s.is_null() {
        unsafe {
            let len = std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len();
            get_gc().register_object(s as usize, len, ObjectKind::CString);
        }
    }
    s
}

/// Format a boolean value to string
#[unsafe(no_mangle)]
pub extern "C" fn otter_format_bool(value: bool) -> *mut c_char {
    let formatted = if value { "true" } else { "false" };
    let s = CString::new(formatted)
        .map(CString::into_raw)
        .unwrap_or_else(|_| std::ptr::null_mut());

    if !s.is_null() {
        unsafe {
            let len = std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len();
            get_gc().register_object(s as usize, len, ObjectKind::CString);
        }
    }
    s
}

/// Concatenate two strings.
///
/// # Safety
///
/// The input pointers must be valid UTF-8 strings allocated by the host or
/// runtime. Returned strings must be released with `otter_free_string`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_str_concat(s1: *const c_char, s2: *const c_char) -> *mut c_char {
    if s1.is_null() || s2.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let Ok(str1) = CStr::from_ptr(s1).to_str() else {
            return std::ptr::null_mut();
        };
        let Ok(str2) = CStr::from_ptr(s2).to_str() else {
            return std::ptr::null_mut();
        };

        let result = format!("{}{}", str1, str2);
        let s = CString::new(result)
            .map(CString::into_raw)
            .unwrap_or_else(|_| std::ptr::null_mut());

        if !s.is_null() {
            let len = CStr::from_ptr(s).to_bytes_with_nul().len();
            get_gc().register_object(s as usize, len, ObjectKind::CString);
        }
        s
    }
}

/// Free a string allocated by Otter runtime
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_free_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let _ = CString::from_raw(ptr);
    }
}

/// Validate UTF-8 string (returns 1 if valid, 0 if invalid)
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_validate_utf8(ptr: *const c_char) -> i32 {
    if ptr.is_null() {
        return 0;
    }

    unsafe {
        match CStr::from_ptr(ptr).to_str() {
            Ok(_) => 1,
            Err(_) => 0,
        }
    }
}

/// Create a string from a string literal (makes a copy)
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_string_from_literal(ptr: *const c_char) -> *mut c_char {
    if ptr.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        match CStr::from_ptr(ptr).to_str() {
            Ok(s) => {
                let new_s = CString::new(s)
                    .map(CString::into_raw)
                    .unwrap_or_else(|_| std::ptr::null_mut());

                if !new_s.is_null() {
                    let len = CStr::from_ptr(new_s).to_bytes_with_nul().len();
                    get_gc().register_object(new_s as usize, len, ObjectKind::CString);
                }
                new_s
            }
            Err(_) => std::ptr::null_mut(),
        }
    }
}

/// Compare two strings for equality (returns 1 if equal, 0 if not equal)
///
/// # Safety
///
/// this function dereferences raw pointers
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_string_equal(s1: *const c_char, s2: *const c_char) -> i32 {
    if s1.is_null() || s2.is_null() {
        return 0;
    }

    unsafe {
        let Ok(str1) = CStr::from_ptr(s1).to_str() else {
            return 0;
        };
        let Ok(str2) = CStr::from_ptr(s2).to_str() else {
            return 0;
        };

        if str1 == str2 { 1 } else { 0 }
    }
}

fn register_string_functions(registry: &SymbolRegistry) {
    registry.register(FfiFunction {
        name: "std.strings.format_float".into(),
        symbol: "otter_format_float".into(),
        signature: FfiSignature::new(vec![FfiType::F64], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "std.strings.format_int".into(),
        symbol: "otter_format_int".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "std.strings.format_bool".into(),
        symbol: "otter_format_bool".into(),
        signature: FfiSignature::new(vec![FfiType::Bool], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "std.strings.concat".into(),
        symbol: "otter_str_concat".into(),
        signature: FfiSignature::new(vec![FfiType::Str, FfiType::Str], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "std.strings.free".into(),
        symbol: "otter_free_string".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "std.strings.validate_utf8".into(),
        symbol: "otter_validate_utf8".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "std.strings.from_literal".into(),
        symbol: "otter_string_from_literal".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "std.strings.equal".into(),
        symbol: "otter_string_equal".into(),
        signature: FfiSignature::new(vec![FfiType::Str, FfiType::Str], FfiType::I32),
    });
}

inventory::submit! {
    crate::runtime::ffi::SymbolProvider {
        namespace: "strings",
        autoload: true,
        register: register_string_functions,
    }
}

#[cfg(test)]
mod tests {
    use std::f64;

    use super::*;

    #[test]
    fn test_format_float() {
        let result = otter_format_float(f64::consts::PI);
        assert!(!result.is_null());
        unsafe {
            let s = CStr::from_ptr(result).to_str().unwrap();
            assert!(s.starts_with("3.14"));
            otter_free_string(result);
        }
    }

    #[test]
    fn test_format_int() {
        let result = otter_format_int(42);
        assert!(!result.is_null());
        unsafe {
            let s = CStr::from_ptr(result).to_str().unwrap();
            assert_eq!(s, "42");
            otter_free_string(result);
        }
    }

    #[test]
    fn test_concat_strings() {
        let s1 = CString::new("Hello ").unwrap();
        let s2 = CString::new("World").unwrap();
        let result = unsafe { otter_str_concat(s1.as_ptr(), s2.as_ptr()) };
        assert!(!result.is_null());
        unsafe {
            let s = CStr::from_ptr(result).to_str().unwrap();
            assert_eq!(s, "Hello World");
            otter_free_string(result);
        }
    }

    #[test]
    fn test_validate_utf8() {
        let valid = CString::new("Hello 🦦").unwrap();
        assert_eq!(unsafe { otter_validate_utf8(valid.as_ptr()) }, 1);
    }
}
