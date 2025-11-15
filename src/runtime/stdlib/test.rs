use std::ffi::CStr;
use std::os::raw::c_char;

use crate::runtime::symbol_registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_test_assert(condition: i64, message: *const c_char) -> i32 {
    if condition != 0 {
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
    std::process::exit(1);
}

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
    std::process::exit(1);
}

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
    std::process::exit(1);
}

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
    std::process::exit(1);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_test_assert_true(condition: i64, message: *const c_char) -> i32 {
    unsafe { otter_test_assert(condition, message) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_test_assert_false(condition: i64, message: *const c_char) -> i32 {
    unsafe { otter_test_assert(if condition == 0 { 1 } else { 0 }, message) }
}

use once_cell::sync::Lazy;
use std::sync::Mutex;

static SNAPSHOT_STORAGE: Lazy<Mutex<std::collections::HashMap<String, String>>> =
    Lazy::new(|| Mutex::new(std::collections::HashMap::new()));

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
                0// Match
            } else {
                eprintln!("Snapshot mismatch for '{}':", name_str);
                eprintln!("  Expected: {}", expected);
                eprintln!("  Got:      {}", value_str);
                std::process::exit(1);
            }
        }
        None => {
            eprintln!(
                "Snapshot '{}' not found. Run with --update-snapshots to create it.",
                name_str
            );
            eprintln!("  Value: {}", value_str);
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
        signature: FfiSignature::new(vec![FfiType::I64, FfiType::Str], FfiType::I32),
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
        signature: FfiSignature::new(vec![FfiType::I64, FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "test.assert_false".into(),
        symbol: "otter_test_assert_false".into(),
        signature: FfiSignature::new(vec![FfiType::I64, FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "test.snapshot".into(),
        symbol: "otter_test_snapshot".into(),
        signature: FfiSignature::new(vec![FfiType::Str, FfiType::Str], FfiType::I32),
    });
}

inventory::submit! {
    crate::runtime::ffi::SymbolProvider {
        register: register_std_test_symbols,
    }
}
