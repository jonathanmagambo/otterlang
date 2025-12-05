use std::backtrace::Backtrace;
use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

// Import builtins for list/map access
use crate::stdlib::builtins::{self, LISTS, Value};

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

fn make_c_string(value: &str) -> *mut c_char {
    CString::new(value)
        .unwrap_or_else(|_| CString::new("invalid utf-8 in exception").unwrap())
        .into_raw()
}

fn dispose_exception(exception: OtterException) {
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

fn set_current_exception(exception: OtterException) {
    CURRENT_EXCEPTION.with(|exc| {
        let mut slot = exc.borrow_mut();
        if let Some(prev) = slot.take() {
            dispose_exception(prev);
        }
        *slot = Some(exception);
    });
}

fn store_exception(message: String, exception_type: String, stack_trace: Option<String>) {
    let stack_trace = stack_trace.unwrap_or_else(capture_stack_trace);
    let exception = OtterException {
        message: make_c_string(&message),
        exception_type: make_c_string(&exception_type),
        stack_trace: make_c_string(&stack_trace),
    };
    set_current_exception(exception);
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
    store_exception(msg, "Exception".to_string(), None);
}

/// Throw an exception with an explicit type label.
///
/// # Safety
/// All pointers must reference valid UTF-8 strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_throw_typed_exception(
    message: *const c_char,
    exception_type: *const c_char,
) {
    if message.is_null() {
        return;
    }

    let msg = unsafe { CStr::from_ptr(message) }
        .to_string_lossy()
        .into_owned();
    let exception_type = if exception_type.is_null() {
        "Exception".to_string()
    } else {
        unsafe { CStr::from_ptr(exception_type) }
            .to_string_lossy()
            .into_owned()
    };

    store_exception(msg, exception_type, None);
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

/// Get the current exception type label
#[unsafe(no_mangle)]
pub extern "C" fn otter_get_exception_type() -> *mut c_char {
    CURRENT_EXCEPTION.with(|exc| {
        if let Some(ref exception) = *exc.borrow() {
            exception.exception_type
        } else {
            std::ptr::null_mut()
        }
    })
}

/// Get the captured stack trace for the current exception
#[unsafe(no_mangle)]
pub extern "C" fn otter_get_exception_stack_trace() -> *mut c_char {
    CURRENT_EXCEPTION.with(|exc| {
        if let Some(ref exception) = *exc.borrow() {
            exception.stack_trace
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
            dispose_exception(exception);
        }
    });
}

fn capture_stack_trace() -> String {
    Backtrace::force_capture().to_string()
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
    values: Vec<Value>,
    index: usize,
}

#[derive(Debug)]
pub struct OtterStringIterator {
    chars: Vec<String>,
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
    let values = lists
        .get(&handle_id)
        .map(|list| list.items.clone())
        .unwrap_or_default();

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

/// Get the next element from an array iterator as a tagged runtime value handle.
///
/// # Safety
/// `iter` must be valid and not null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_iter_next_array(iter: *mut OtterArrayIterator) -> u64 {
    if iter.is_null() {
        return 0;
    }
    let iter_ref = unsafe { &mut *iter };
    if iter_ref.index < iter_ref.values.len() {
        let value = iter_ref.values[iter_ref.index].clone();
        iter_ref.index += 1;
        builtins::encode_runtime_value(&value)
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

    // Convert string into owned UTF-8 scalar values so we preserve multi-byte characters.
    let chars: Vec<String> = rust_str.chars().map(|c| c.to_string()).collect();

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

/// Retrieve the next character as a tagged runtime value.
///
/// # Safety
/// `iter` must be valid and not null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_iter_next_string(iter: *mut OtterStringIterator) -> u64 {
    if iter.is_null() {
        return 0;
    }
    let iter_ref = unsafe { &mut *iter };
    if iter_ref.index < iter_ref.chars.len() {
        let char_value = iter_ref.chars[iter_ref.index].clone();
        iter_ref.index += 1;
        builtins::encode_runtime_value(&Value::String(char_value))
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

use otterc_symbol::registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

fn register_exception_runtime_symbols(registry: &SymbolRegistry) {
    // Exception handling
    registry.register(FfiFunction {
        name: "__otter_throw".into(),
        symbol: "otter_throw_exception".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "__otter_throw_typed".into(),
        symbol: "otter_throw_typed_exception".into(),
        signature: FfiSignature::new(vec![FfiType::Str, FfiType::Str], FfiType::Unit),
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
        name: "__otter_get_exception_type".into(),
        symbol: "otter_get_exception_type".into(),
        signature: FfiSignature::new(vec![], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "__otter_get_exception_stack".into(),
        symbol: "otter_get_exception_stack_trace".into(),
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
}

inventory::submit! {
    otterc_ffi::SymbolProvider {
        namespace: "exceptions",
        autoload: true,
        register: register_exception_runtime_symbols,
    }
}
