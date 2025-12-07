#![expect(
    clippy::print_stderr,
    reason = "We want to print to stderr with eprintln in test assertions"
)]

use std::ffi::CStr;
use std::os::raw::c_char;

use crate::runtime::symbol_registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TestPoint {
    pub x: i64,
    pub y: i64,
}

/// asserts that a condition is truthy; panic-ing with `message` otherwise
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_test_assert(condition: bool, message: *const c_char) -> i32 {
    if condition {
        return 0; // Success
    }

    let msg = if message.is_null() {
        "Assertion failed".to_string()
    } else {
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .to_string()
    };

    eprintln!("Assertion failed: {}", msg);
    #[expect(clippy::exit, reason = "TODO: Use a more robust panic mechanism here")]
    std::process::exit(1);
}

/// asserts that two strings are equal; panic-ing with `message` otherwise
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_test_assert_eq(
    left: *const c_char,
    right: *const c_char,
    message: *const c_char,
) -> i32 {
    let left_str = unsafe { CStr::from_ptr(left) }
        .to_string_lossy()
        .to_string();
    let right_str = unsafe { CStr::from_ptr(right) }
        .to_string_lossy()
        .to_string();

    if left_str == right_str {
        return 0; // Success
    }

    let msg = if message.is_null() {
        format!(
            "Assertion failed: expected '{}', got '{}'",
            right_str, left_str
        )
    } else {
        let custom_msg = unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .to_string();
        format!(
            "{}: expected '{}', got '{}'",
            custom_msg, right_str, left_str
        )
    };

    eprintln!("{}", msg);
    #[expect(clippy::exit, reason = "TODO: Use a more robust panic mechanism here")]
    std::process::exit(1);
}

/// asserts that two strings are not equal; panic-ing with `message` otherwise
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_test_assert_ne(
    left: *const c_char,
    right: *const c_char,
    message: *const c_char,
) -> i32 {
    let left_str = unsafe { CStr::from_ptr(left) }
        .to_string_lossy()
        .to_string();
    let right_str = unsafe { CStr::from_ptr(right) }
        .to_string_lossy()
        .to_string();

    if left_str != right_str {
        return 0; // Success
    }

    let msg = if message.is_null() {
        format!(
            "Assertion failed: values should not be equal, but both are '{}'",
            left_str
        )
    } else {
        let custom_msg = unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .to_string();
        format!(
            "{}: values should not be equal, but both are '{}'",
            custom_msg, left_str
        )
    };

    eprintln!("{}", msg);
    #[expect(clippy::exit, reason = "TODO: Use a more robust panic mechanism here")]
    std::process::exit(1);
}

/// asserts that two floats are approximately equal within `epsilon`; panic-ing
/// with `message` otherwise
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_test_assert_approx_eq(
    left: f64,
    right: f64,
    epsilon: f64,
    message: *const c_char,
) -> i32 {
    let diff = (left - right).abs();
    if diff <= epsilon {
        return 0; // Success
    }

    let msg = if message.is_null() {
        format!(
            "Assertion failed: expected approximately {}, got {} (diff: {}, epsilon: {})",
            right, left, diff, epsilon
        )
    } else {
        let custom_msg = unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .to_string();
        format!(
            "{}: expected approximately {}, got {} (diff: {}, epsilon: {})",
            custom_msg, right, left, diff, epsilon
        )
    };

    eprintln!("{}", msg);
    #[expect(clippy::exit, reason = "TODO: Use a more robust panic mechanism here")]
    std::process::exit(1);
}

/// asserts that `condition` is truthy; panic-ing with `message` otherwise
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_test_assert_true(condition: bool, message: *const c_char) -> i32 {
    unsafe { otter_test_assert(condition, message) }
}

/// asserts that `condition` is falsy; panic-ing with `message` otherwise
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_test_assert_false(condition: bool, message: *const c_char) -> i32 {
    unsafe { otter_test_assert(!condition, message) }
}

/// Creates a new Point struct from x, y values and returns an opaque handle
///
/// # Safety
///
/// this function crosses the FFI boundary
#[unsafe(no_mangle)]
pub extern "C" fn otter_test_point_new(x: i64, y: i64) -> i64 {
    let point = Box::new(TestPoint { x, y });
    Box::into_raw(point) as i64
}

/// Gets the x field from a Point handle
///
/// # Safety
///
/// handle must be a valid Point pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_test_point_get_x(handle: i64) -> i64 {
    unsafe {
        let point = handle as *const TestPoint;
        if point.is_null() {
            return 0;
        }
        (*point).x
    }
}

/// Gets the y field from a Point handle
///
/// # Safety
///
/// handle must be a valid Point pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_test_point_get_y(handle: i64) -> i64 {
    unsafe {
        let point = handle as *const TestPoint;
        if point.is_null() {
            return 0;
        }
        (*point).y
    }
}

/// Returns a copy of the input point (identity function via handles)
///
/// # Safety
///
/// handle must be a valid Point pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_test_struct_identity(handle: i64) -> i64 {
    unsafe {
        let point = handle as *const TestPoint;
        if point.is_null() {
            return 0;
        }
        // Clone the point and return new handle
        let copy = Box::new(TestPoint {
            x: (*point).x,
            y: (*point).y,
        });
        Box::into_raw(copy) as i64
    }
}

/// Frees a Point handle
///
/// # Safety
///
/// handle must be a valid Point pointer or 0
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_test_point_free(handle: i64) {
    unsafe {
        if handle != 0 {
            let _ = Box::from_raw(handle as *mut TestPoint);
        }
    }
}

use once_cell::sync::Lazy;
use std::sync::Mutex;

static SNAPSHOT_STORAGE: Lazy<Mutex<std::collections::HashMap<String, String>>> =
    Lazy::new(|| Mutex::new(std::collections::HashMap::new()));

/// performs snapshot testing for the given value
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_test_snapshot(name: *const c_char, value: *const c_char) -> i32 {
    let name_str = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .to_string();
    let value_str = unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .to_string();

    let update_mode = std::env::var("OTTER_UPDATE_SNAPSHOTS").is_ok();

    let mut storage = SNAPSHOT_STORAGE.lock().unwrap();

    if update_mode {
        storage.insert(name_str.clone(), value_str.clone());
        return 0; // Success in update mode
    }

    match storage.get(&name_str) {
        Some(expected) => {
            if expected == &value_str {
                0 // Match
            } else {
                eprintln!("Snapshot mismatch for '{}':", name_str);
                eprintln!("  Expected: {}", expected);
                eprintln!("  Got:      {}", value_str);
                #[expect(clippy::exit, reason = "TODO: Use a more robust panic mechanism here")]
                std::process::exit(1);
            }
        }
        None => {
            eprintln!(
                "Snapshot '{}' not found. Run with --update-snapshots to create it.",
                name_str
            );
            eprintln!("  Value: {}", value_str);
            #[expect(clippy::exit, reason = "TODO: Use a more robust panic mechanism here")]
            std::process::exit(1);
        }
    }
}

// ============================================================================
// Symbol Registration
// ============================================================================

fn register_std_test_symbols(registry: &SymbolRegistry) {
    registry.register(FfiFunction {
        name: "test.assert".into(),
        symbol: "otter_test_assert".into(),
        signature: FfiSignature::new(vec![FfiType::Bool, FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "test.assert_eq".into(),
        symbol: "otter_test_assert_eq".into(),
        signature: FfiSignature::new(vec![FfiType::Str, FfiType::Str, FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "test.assert_ne".into(),
        symbol: "otter_test_assert_ne".into(),
        signature: FfiSignature::new(vec![FfiType::Str, FfiType::Str, FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "test.assert_approx_eq".into(),
        symbol: "otter_test_assert_approx_eq".into(),
        signature: FfiSignature::new(
            vec![FfiType::F64, FfiType::F64, FfiType::F64, FfiType::Str],
            FfiType::I32,
        ),
    });

    registry.register(FfiFunction {
        name: "test.assert_true".into(),
        symbol: "otter_test_assert_true".into(),
        signature: FfiSignature::new(vec![FfiType::Bool, FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "test.assert_false".into(),
        symbol: "otter_test_assert_false".into(),
        signature: FfiSignature::new(vec![FfiType::Bool, FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "test.snapshot".into(),
        symbol: "otter_test_snapshot".into(),
        signature: FfiSignature::new(vec![FfiType::Str, FfiType::Str], FfiType::I32),
    });

    // Point struct operations using opaque handles (cross-platform ABI)
    registry.register(FfiFunction {
        name: "test.point_new".into(),
        symbol: "otter_test_point_new".into(),
        signature: FfiSignature::new(vec![FfiType::I64, FfiType::I64], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "test.point_get_x".into(),
        symbol: "otter_test_point_get_x".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "test.point_get_y".into(),
        symbol: "otter_test_point_get_y".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "test.struct_identity".into(),
        symbol: "otter_test_struct_identity".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "test.point_free".into(),
        symbol: "otter_test_point_free".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Unit),
    });
}

inventory::submit! {
    crate::runtime::ffi::SymbolProvider {
        namespace: "test",
        autoload: false,
        register: register_std_test_symbols,
    }
}
