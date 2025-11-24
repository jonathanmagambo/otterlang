use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

// Import builtins for list/map access
use crate::runtime::stdlib::builtins::{LISTS, Value};

// ============================================================================
// Exception Runtime Support
// ============================================================================

#[repr(C)]
pub struct OtterException {
    pub message: *mut c_char,
    pub exception_type: *mut c_char,
    pub stack_trace: *mut c_char,
}

// Thread-local exception storage
thread_local! {
    static CURRENT_EXCEPTION: RefCell<Option<OtterException>> = const { RefCell::new(None) };
}

/// Throw an exception with a message
///
/// # Safety
/// The caller must ensure `message` points to a valid NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_throw_exception(message: *const c_char) {
    if message.is_null() {
        return;
    }

    let msg = unsafe { CStr::from_ptr(message) }
        .to_string_lossy()
        .into_owned();
    let exception = OtterException {
        message: CString::new(msg.clone()).unwrap().into_raw(),
        exception_type: CString::new("Exception").unwrap().into_raw(),
        stack_trace: CString::new(capture_stack_trace()).unwrap().into_raw(),
    };

    CURRENT_EXCEPTION.with(|exc| {
        *exc.borrow_mut() = Some(exception);
    });
}

/// Check if there's a current exception
#[unsafe(no_mangle)]
pub extern "C" fn otter_has_exception() -> bool {
    CURRENT_EXCEPTION.with(|exc| exc.borrow().is_some())
}

/// Get the current exception message
#[unsafe(no_mangle)]
pub extern "C" fn otter_get_exception_message() -> *mut c_char {
    CURRENT_EXCEPTION.with(|exc| {
        if let Some(ref exception) = *exc.borrow() {
            exception.message
        } else {
            std::ptr::null_mut()
        }
    })
}

/// Clear the current exception
#[unsafe(no_mangle)]
pub extern "C" fn otter_clear_exception() {
    CURRENT_EXCEPTION.with(|exc| {
        if let Some(exception) = exc.borrow_mut().take() {
            unsafe {
                if !exception.message.is_null() {
                    drop(CString::from_raw(exception.message));
                }
                if !exception.exception_type.is_null() {
                    drop(CString::from_raw(exception.exception_type));
                }
                if !exception.stack_trace.is_null() {
                    drop(CString::from_raw(exception.stack_trace));
                }
            }
        }
    });
}

fn capture_stack_trace() -> String {
    // Simple stack trace - in production would use backtrace crate
    "  at <unknown>:0:0".to_string()
}

// ============================================================================
// Iterator Protocol Support
// ============================================================================

#[repr(C)]
pub struct OtterIterator {
    pub current: i64,
    pub end: i64,
    pub step: i64,
}

/// Create a range iterator
#[unsafe(no_mangle)]
pub extern "C" fn otter_iter_range(start: i64, end: i64) -> *mut OtterIterator {
    Box::into_raw(Box::new(OtterIterator {
        current: start,
        end,
        step: 1,
    }))
}

/// Create a range iterator with step
#[unsafe(no_mangle)]
pub extern "C" fn otter_iter_range_step(start: i64, end: i64, step: i64) -> *mut OtterIterator {
    Box::into_raw(Box::new(OtterIterator {
        current: start,
        end,
        step,
    }))
}

/// Check if iterator has next element
///
/// # Safety
/// The caller must pass a valid iterator pointer returned by the runtime.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_iter_has_next(iter: *mut OtterIterator) -> bool {
    if iter.is_null() {
        return false;
    }
    let it = unsafe { &*iter };
    if it.step > 0 {
        it.current < it.end
    } else {
        it.current > it.end
    }
}

/// Get next element from iterator
///
/// # Safety
/// The caller must pass a valid iterator pointer returned by the runtime.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_iter_next(iter: *mut OtterIterator) -> i64 {
    if iter.is_null() {
        return 0;
    }
    let it = unsafe { &mut *iter };
    let current = it.current;
    it.current += it.step;
    current
}

/// Free an iterator
///
/// # Safety
/// The caller must pass a valid iterator pointer and must not use it after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_iter_free(iter: *mut OtterIterator) {
    if !iter.is_null() {
        unsafe {
            drop(Box::from_raw(iter));
        }
    }
}

// Float iterators
#[repr(C)]
pub struct OtterFloatIterator {
    pub current: f64,
    pub end: f64,
    pub step: f64,
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_iter_range_f64(start: f64, end: f64) -> *mut OtterFloatIterator {
    Box::into_raw(Box::new(OtterFloatIterator {
        current: start,
        end,
        step: 1.0,
    }))
}

/// Check if a float iterator has another element.
///
/// # Safety
/// `iter` must be a valid pointer returned by the runtime.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_iter_has_next_f64(iter: *mut OtterFloatIterator) -> bool {
    if iter.is_null() {
        return false;
    }
    let it = unsafe { &*iter };
    if it.step > 0.0 {
        it.current < it.end
    } else {
        it.current > it.end
    }
}

/// Fetch the next value from a float iterator.
///
/// # Safety
/// `iter` must be valid and previously obtained from the runtime.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_iter_next_f64(iter: *mut OtterFloatIterator) -> f64 {
    if iter.is_null() {
        return 0.0;
    }
    let it = unsafe { &mut *iter };
    let current = it.current;
    it.current += it.step;
    current
}

/// Release a float iterator.
///
/// # Safety
/// `iter` must be valid and not used after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_iter_free_f64(iter: *mut OtterFloatIterator) {
    if !iter.is_null() {
        unsafe {
            drop(Box::from_raw(iter));
        }
    }
}

// ============================================================================
// Array/String Iterator Implementations
// ============================================================================

#[derive(Debug)]
pub struct OtterArrayIterator {
    values: Vec<i64>, // Store i64 values for iteration
    index: usize,
}

#[derive(Debug)]
pub struct OtterStringIterator {
    chars: Vec<i64>, // Store character codes as i64
    index: usize,
}

/// Create an iterator over a runtime list handle.
///
/// # Safety
/// `array_handle_ptr` must come from Otter's runtime and remain valid for the
/// iterator lifetime.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_iter_array(
    array_handle_ptr: *mut std::ffi::c_void,
) -> *mut OtterArrayIterator {
    if array_handle_ptr.is_null() {
        return std::ptr::null_mut();
    }

    // Cast the void pointer to a handle (u64)
    let handle = array_handle_ptr as *const u64;
    if handle.is_null() {
        return std::ptr::null_mut();
    }

    let handle_id = unsafe { *handle };

    // Look up the list in the global lists map
    let lists = LISTS.read();
    let values = if let Some(list) = lists.get(&handle_id) {
        // Convert the Vec<Value> to Vec<i64> for iteration
        // For now, we'll extract numeric values and convert others to 0
        list.items
            .iter()
            .map(|value| match value {
                Value::I64(i) => *i,
                Value::F64(f) => *f as i64,
                Value::Bool(b) => {
                    if *b {
                        1
                    } else {
                        0
                    }
                }
                _ => 0, // Other types not supported for iteration yet
            })
            .collect()
    } else {
        Vec::new()
    };

    let iter = Box::new(OtterArrayIterator { values, index: 0 });
    Box::into_raw(iter)
}

/// Check if an array iterator has remaining elements.
///
/// # Safety
/// `iter` must be a valid pointer returned by [`otter_iter_array`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_iter_has_next_array(iter: *mut OtterArrayIterator) -> bool {
    if iter.is_null() {
        return false;
    }
    let iter_ref = unsafe { &*iter };
    iter_ref.index < iter_ref.values.len()
}

/// Get the next element from an array iterator.
///
/// # Safety
/// `iter` must be valid and not null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_iter_next_array(iter: *mut OtterArrayIterator) -> i64 {
    if iter.is_null() {
        return 0;
    }
    let iter_ref = unsafe { &mut *iter };
    if iter_ref.index < iter_ref.values.len() {
        let value = iter_ref.values[iter_ref.index];
        iter_ref.index += 1;
        value
    } else {
        0
    }
}

/// Release an array iterator.
///
/// # Safety
/// `iter` must be valid and must not be used after calling this function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_iter_free_array(iter: *mut OtterArrayIterator) {
    if !iter.is_null() {
        unsafe {
            drop(Box::from_raw(iter));
        }
    }
}

/// Create an iterator over the characters of a runtime string handle.
///
/// # Safety
/// `str_ptr` must be a valid C string pointer supplied by the runtime.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_iter_string(
    str_ptr: *mut std::ffi::c_void,
) -> *mut OtterStringIterator {
    if str_ptr.is_null() {
        return std::ptr::null_mut();
    }

    // Cast to a C string pointer and convert to Rust string
    let c_str = unsafe { CStr::from_ptr(str_ptr as *const c_char) };
    let rust_str = c_str.to_str().unwrap_or_default();

    // Convert string to vector of character codes (i64)
    let chars: Vec<i64> = rust_str.chars().map(|c| c as i64).collect();

    let iter = Box::new(OtterStringIterator { chars, index: 0 });
    Box::into_raw(iter)
}

/// Check whether a string iterator has another character.
///
/// # Safety
/// `iter` must be a valid pointer from [`otter_iter_string`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_iter_has_next_string(iter: *mut OtterStringIterator) -> bool {
    if iter.is_null() {
        return false;
    }
    let iter_ref = unsafe { &*iter };
    iter_ref.index < iter_ref.chars.len()
}

/// Retrieve the next character code from a string iterator.
///
/// # Safety
/// `iter` must be valid and not null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_iter_next_string(iter: *mut OtterStringIterator) -> i64 {
    if iter.is_null() {
        return 0;
    }
    let iter_ref = unsafe { &mut *iter };
    if iter_ref.index < iter_ref.chars.len() {
        let char_code = iter_ref.chars[iter_ref.index];
        iter_ref.index += 1;
        char_code
    } else {
        0
    }
}

/// Release a string iterator.
///
/// # Safety
/// `iter` must be valid and should not be reused after freeing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_iter_free_string(iter: *mut OtterStringIterator) {
    if !iter.is_null() {
        unsafe {
            drop(Box::from_raw(iter));
        }
    }
}

// ============================================================================
// Symbol Registration
// ============================================================================

use crate::runtime::symbol_registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

fn register_exception_runtime_symbols(registry: &SymbolRegistry) {
    // Exception handling
    registry.register(FfiFunction {
        name: "__otter_throw".into(),
        symbol: "otter_throw_exception".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "__otter_has_exception".into(),
        symbol: "otter_has_exception".into(),
        signature: FfiSignature::new(vec![], FfiType::Bool),
    });

    registry.register(FfiFunction {
        name: "__otter_get_exception_message".into(),
        symbol: "otter_get_exception_message".into(),
        signature: FfiSignature::new(vec![], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "__otter_clear_exception".into(),
        symbol: "otter_clear_exception".into(),
        signature: FfiSignature::new(vec![], FfiType::Unit),
    });

    // Iterator protocol
    registry.register(FfiFunction {
        name: "__otter_iter_range".into(),
        symbol: "otter_iter_range".into(),
        signature: FfiSignature::new(vec![FfiType::I64, FfiType::I64], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "__otter_iter_has_next".into(),
        symbol: "otter_iter_has_next".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Bool),
    });

    registry.register(FfiFunction {
        name: "__otter_iter_next".into(),
        symbol: "otter_iter_next".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "__otter_iter_free".into(),
        symbol: "otter_iter_free".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Unit),
    });

    // Float iterators
    registry.register(FfiFunction {
        name: "__otter_iter_range_f64".into(),
        symbol: "otter_iter_range_f64".into(),
        signature: FfiSignature::new(vec![FfiType::F64, FfiType::F64], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "__otter_iter_has_next_f64".into(),
        symbol: "otter_iter_has_next_f64".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Bool),
    });

    registry.register(FfiFunction {
        name: "__otter_iter_next_f64".into(),
        symbol: "otter_iter_next_f64".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "__otter_iter_free_f64".into(),
        symbol: "otter_iter_free_f64".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Unit),
    });

    // Array/list iterators
    registry.register(FfiFunction {
        name: "__otter_iter_array".into(),
        symbol: "otter_iter_array".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "__otter_iter_has_next_array".into(),
        symbol: "otter_iter_has_next_array".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Bool),
    });

    registry.register(FfiFunction {
        name: "__otter_iter_next_array".into(),
        symbol: "otter_iter_next_array".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "__otter_iter_free_array".into(),
        symbol: "otter_iter_free_array".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Unit),
    });

    // String iterators (character iteration)
    registry.register(FfiFunction {
        name: "__otter_iter_string".into(),
        symbol: "otter_iter_string".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "__otter_iter_has_next_string".into(),
        symbol: "otter_iter_has_next_string".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Bool),
    });

    registry.register(FfiFunction {
        name: "__otter_iter_next_string".into(),
        symbol: "otter_iter_next_string".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "__otter_iter_free_string".into(),
        symbol: "otter_iter_free_string".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Unit),
    });
}

inventory::submit! {
    crate::runtime::ffi::SymbolProvider {
        register: register_exception_runtime_symbols,
    }
}
