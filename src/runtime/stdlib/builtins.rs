use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};

use once_cell::sync::Lazy;
use parking_lot::RwLock;

use crate::runtime::symbol_registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

// ============================================================================
// Built-in Collections Registry
// For lists and maps, we'll use opaque handles
// ============================================================================

type HandleId = u64;
static NEXT_HANDLE_ID: AtomicU64 = AtomicU64::new(1);

fn next_handle_id() -> HandleId {
    NEXT_HANDLE_ID.fetch_add(1, Ordering::SeqCst)
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum Value {
    Unit,
    Bool(bool),
    I64(i64),
    F64(f64),
    String(String),
    List(HandleId),
    Map(HandleId),
}

struct List {
    items: Vec<Value>,
}

static LISTS: Lazy<RwLock<std::collections::HashMap<HandleId, List>>> =
    Lazy::new(|| RwLock::new(std::collections::HashMap::new()));

struct Map {
    items: std::collections::HashMap<String, Value>,
}

static MAPS: Lazy<RwLock<std::collections::HashMap<HandleId, Map>>> =
    Lazy::new(|| RwLock::new(std::collections::HashMap::new()));

fn value_to_string(value: &Value) -> String {
    match value {
        Value::Unit => "None".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::I64(i) => i.to_string(),
        Value::F64(f) => {
            if f.fract() == 0.0 {
                (*f as i64).to_string()
            } else {
                f.to_string()
            }
        }
        Value::String(s) => s.clone(),
        Value::List(handle) => stringify_list_handle(*handle),
        Value::Map(handle) => stringify_map_handle(*handle),
    }
}

fn list_value(handle: HandleId, index: i64) -> Option<Value> {
    if index < 0 {
        return None;
    }
    let lists = LISTS.read();
    lists
        .get(&handle)
        .and_then(|list| list.items.get(index as usize).cloned())
}

fn map_value(handle: HandleId, key: &str) -> Option<Value> {
    let maps = MAPS.read();
    maps.get(&handle)
        .and_then(|map| map.items.get(key).cloned())
}

fn stringify_list_handle(handle: HandleId) -> String {
    let lists = LISTS.read();
    if let Some(list) = lists.get(&handle) {
        let items = list
            .items
            .iter()
            .map(value_to_string)
            .collect::<Vec<_>>();
        format!("[{}]", items.join(", "))
    } else {
        "[]".to_string()
    }
}

fn stringify_map_handle(handle: HandleId) -> String {
    let maps = MAPS.read();
    if let Some(map) = maps.get(&handle) {
        let items = map
            .items
            .iter()
            .map(|(key, value)| format!("{}: {}", key, value_to_string(value)))
            .collect::<Vec<_>>();
        format!("{{{}}}", items.join(", "))
    } else {
        "{}".to_string()
    }
}

// ============================================================================
// Error Handling - Panic and Recovery
// ============================================================================

// Error type representation
struct Error {
    message: String,
}

static ERRORS: Lazy<RwLock<std::collections::HashMap<HandleId, Error>>> =
    Lazy::new(|| RwLock::new(std::collections::HashMap::new()));

// Thread-local panic state
thread_local! {
    static PANIC_STATE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

// Thread-local defer stack
thread_local! {
    static DEFER_STACK: std::cell::RefCell<Vec<extern "C" fn()>> = std::cell::RefCell::new(Vec::new());
}

// ============================================================================
// len(x) - Get length of string, list, or map
// ============================================================================

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_len_string(s: *const c_char) -> i64 {
    if s.is_null() {
        return 0;
    }
    unsafe {
        if let Ok(str_ref) = CStr::from_ptr(s).to_str() {
            str_ref.len() as i64
        } else {
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_len_list(handle: u64) -> i64 {
    let lists = LISTS.read();
    if let Some(list) = lists.get(&handle) {
        list.items.len() as i64
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_len_map(handle: u64) -> i64 {
    let maps = MAPS.read();
    if let Some(map) = maps.get(&handle) {
        map.items.len() as i64
    } else {
        0
    }
}

// ============================================================================
// cap(x) - Get capacity of a list
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_cap_list(handle: u64) -> i64 {
    let lists = LISTS.read();
    if let Some(list) = lists.get(&handle) {
        list.items.capacity() as i64
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_cap_string(s: *const c_char) -> i64 {
    if s.is_null() {
        return 0;
    }
    unsafe {
        if let Ok(str_ref) = CStr::from_ptr(s).to_str() {
            // Strings are immutable, so capacity = length
            str_ref.len() as i64
        } else {
            0
        }
    }
}

// ============================================================================
// str.contains(substring) - Check if string contains substring
// ============================================================================

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_str_contains(
    s: *const c_char,
    substring: *const c_char,
) -> bool {
    if s.is_null() || substring.is_null() {
        return false;
    }
    unsafe {
        if let (Ok(str_ref), Ok(substr_ref)) = (
            CStr::from_ptr(s).to_str(),
            CStr::from_ptr(substring).to_str(),
        ) {
            str_ref.contains(substr_ref)
        } else {
            false
        }
    }
}

// ============================================================================
// append(x, val) - Append to a list
// ============================================================================

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_append_list_string(handle: u64, val: *const c_char) -> i32 {
    if val.is_null() {
        return 0;
    }

    let val_str = unsafe { CStr::from_ptr(val).to_str().unwrap_or("").to_string() };

    let mut lists = LISTS.write();
    if let Some(list) = lists.get_mut(&handle) {
        list.items.push(Value::String(val_str));
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_append_list_int(handle: u64, val: i64) -> i32 {
    let mut lists = LISTS.write();
    if let Some(list) = lists.get_mut(&handle) {
        list.items.push(Value::I64(val));
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_append_list_float(handle: u64, val: f64) -> i32 {
    let mut lists = LISTS.write();
    if let Some(list) = lists.get_mut(&handle) {
        list.items.push(Value::F64(val));
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_append_list_bool(handle: u64, val: bool) -> i32 {
    let mut lists = LISTS.write();
    if let Some(list) = lists.get_mut(&handle) {
        list.items.push(Value::Bool(val));
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_append_list_list(handle: u64, value_handle: u64) -> i32 {
    let mut lists = LISTS.write();
    if let Some(list) = lists.get_mut(&handle) {
        list.items.push(Value::List(value_handle));
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_append_list_map(handle: u64, value_handle: u64) -> i32 {
    let mut lists = LISTS.write();
    if let Some(list) = lists.get_mut(&handle) {
        list.items.push(Value::Map(value_handle));
        1
    } else {
        0
    }
}

// ============================================================================
// delete(map, key) - Delete a key from a map
// ============================================================================

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_delete_map(handle: u64, key: *const c_char) -> i32 {
    if key.is_null() {
        return 0;
    }

    let key_str = unsafe { CStr::from_ptr(key).to_str().unwrap_or("").to_string() };

    let mut maps = MAPS.write();
    if let Some(map) = maps.get_mut(&handle) {
        if map.items.remove(&key_str).is_some() {
            1
        } else {
            0
        }
    } else {
        0
    }
}

// ============================================================================
// range(start, end) - Generate a range (returns list handle)
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_range_int(start: i64, end: i64) -> u64 {
    let id = next_handle_id();
    let mut items = Vec::new();

    if start <= end {
        for i in start..end {
            items.push(Value::I64(i));
        }
    }

    let list = List { items };
    LISTS.write().insert(id, list);
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_range_float(start: f64, end: f64) -> u64 {
    let id = next_handle_id();
    let mut items = Vec::new();

    if start <= end {
        let mut current = start;
        while current < end {
            items.push(Value::F64(current));
            current += 1.0;
        }
    }

    let list = List { items };
    LISTS.write().insert(id, list);
    id
}

// ============================================================================
// enumerate(list) - Enumerate a list with indices
// Returns a new list handle with "index:value" format
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_enumerate_list(handle: u64) -> u64 {
    let lists = LISTS.read();
    let id = next_handle_id();

    if let Some(list) = lists.get(&handle) {
        let enumerated: Vec<Value> = list
            .items
            .iter()
            .enumerate()
            .map(|(idx, val)| Value::String(format!("{}:{}", idx, value_to_string(val))))
            .collect();

        let new_list = List { items: enumerated };
        drop(lists); // Release read lock
        LISTS.write().insert(id, new_list);
    } else {
        // Return empty list if input handle invalid
        let empty_list = List { items: Vec::new() };
        drop(lists);
        LISTS.write().insert(id, empty_list);
    }

    id
}

// ============================================================================
// Helper functions for list/map creation
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_list_new() -> u64 {
    let id = next_handle_id();
    let list = List { items: Vec::new() };
    LISTS.write().insert(id, list);
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_map_new() -> u64 {
    let id = next_handle_id();
    let map = Map {
        items: std::collections::HashMap::new(),
    };
    MAPS.write().insert(id, map);
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_list_get(handle: u64, index: i64) -> *mut c_char {
    match list_value(handle, index) {
        Some(value) => CString::new(value_to_string(&value))
            .ok()
            .map(CString::into_raw)
            .unwrap_or(std::ptr::null_mut()),
        None => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_map_get(handle: u64, key: *const c_char) -> *mut c_char {
    if key.is_null() {
        return std::ptr::null_mut();
    }

    let key_str = unsafe { CStr::from_ptr(key).to_str().unwrap_or("").to_string() };

    match map_value(handle, &key_str) {
        Some(value) => CString::new(value_to_string(&value))
            .ok()
            .map(CString::into_raw)
            .unwrap_or(std::ptr::null_mut()),
        None => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_list_get_int(handle: u64, index: i64) -> i64 {
    match list_value(handle, index) {
        Some(Value::I64(i)) => i,
        Some(Value::F64(f)) => f as i64,
        Some(Value::Bool(b)) => {
            if b {
                1
            } else {
                0
            }
        }
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_list_get_float(handle: u64, index: i64) -> f64 {
    match list_value(handle, index) {
        Some(Value::F64(f)) => f,
        Some(Value::I64(i)) => i as f64,
        Some(Value::Bool(b)) => {
            if b {
                1.0
            } else {
                0.0
            }
        }
        _ => 0.0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_list_get_bool(handle: u64, index: i64) -> bool {
    match list_value(handle, index) {
        Some(Value::Bool(b)) => b,
        Some(Value::I64(i)) => i != 0,
        Some(Value::F64(f)) => f != 0.0,
        _ => false,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_list_get_list(handle: u64, index: i64) -> u64 {
    match list_value(handle, index) {
        Some(Value::List(inner)) => inner,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_list_get_map(handle: u64, index: i64) -> u64 {
    match list_value(handle, index) {
        Some(Value::Map(inner)) => inner,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_map_set(
    handle: u64,
    key: *const c_char,
    value: *const c_char,
) -> i32 {
    if key.is_null() || value.is_null() {
        return 0;
    }

    let key_str = unsafe { CStr::from_ptr(key).to_str().unwrap_or("").to_string() };

    let value_str = unsafe { CStr::from_ptr(value).to_str().unwrap_or("").to_string() };

    let mut maps = MAPS.write();
    if let Some(map) = maps.get_mut(&handle) {
        map.items.insert(key_str, Value::String(value_str));
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_map_get_int(handle: u64, key: *const c_char) -> i64 {
    if key.is_null() {
        return 0;
    }
    let key_str = unsafe { CStr::from_ptr(key).to_str().unwrap_or("").to_string() };
    match map_value(handle, &key_str) {
        Some(Value::I64(i)) => i,
        Some(Value::F64(f)) => f as i64,
        Some(Value::Bool(b)) => {
            if b {
                1
            } else {
                0
            }
        }
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_map_get_float(handle: u64, key: *const c_char) -> f64 {
    if key.is_null() {
        return 0.0;
    }
    let key_str = unsafe { CStr::from_ptr(key).to_str().unwrap_or("").to_string() };
    match map_value(handle, &key_str) {
        Some(Value::F64(f)) => f,
        Some(Value::I64(i)) => i as f64,
        Some(Value::Bool(b)) => {
            if b {
                1.0
            } else {
                0.0
            }
        }
        _ => 0.0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_map_get_bool(handle: u64, key: *const c_char) -> bool {
    if key.is_null() {
        return false;
    }
    let key_str = unsafe { CStr::from_ptr(key).to_str().unwrap_or("").to_string() };
    match map_value(handle, &key_str) {
        Some(Value::Bool(b)) => b,
        Some(Value::I64(i)) => i != 0,
        Some(Value::F64(f)) => f != 0.0,
        _ => false,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_map_get_list(handle: u64, key: *const c_char) -> u64 {
    if key.is_null() {
        return 0;
    }
    let key_str = unsafe { CStr::from_ptr(key).to_str().unwrap_or("").to_string() };
    match map_value(handle, &key_str) {
        Some(Value::List(inner)) => inner,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_map_get_map(handle: u64, key: *const c_char) -> u64 {
    if key.is_null() {
        return 0;
    }
    let key_str = unsafe { CStr::from_ptr(key).to_str().unwrap_or("").to_string() };
    match map_value(handle, &key_str) {
        Some(Value::Map(inner)) => inner,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_map_set_int(
    handle: u64,
    key: *const c_char,
    value: i64,
) -> i32 {
    if key.is_null() {
        return 0;
    }

    let key_str = unsafe { CStr::from_ptr(key).to_str().unwrap_or("").to_string() };

    let mut maps = MAPS.write();
    if let Some(map) = maps.get_mut(&handle) {
        map.items.insert(key_str, Value::I64(value));
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_map_set_float(
    handle: u64,
    key: *const c_char,
    value: f64,
) -> i32 {
    if key.is_null() {
        return 0;
    }

    let key_str = unsafe { CStr::from_ptr(key).to_str().unwrap_or("").to_string() };

    let mut maps = MAPS.write();
    if let Some(map) = maps.get_mut(&handle) {
        map.items.insert(key_str, Value::F64(value));
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_map_set_bool(
    handle: u64,
    key: *const c_char,
    value: bool,
) -> i32 {
    if key.is_null() {
        return 0;
    }

    let key_str = unsafe { CStr::from_ptr(key).to_str().unwrap_or("").to_string() };

    let mut maps = MAPS.write();
    if let Some(map) = maps.get_mut(&handle) {
        map.items.insert(key_str, Value::Bool(value));
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_map_set_list(
    handle: u64,
    key: *const c_char,
    value_handle: u64,
) -> i32 {
    if key.is_null() {
        return 0;
    }

    let key_str = unsafe { CStr::from_ptr(key).to_str().unwrap_or("").to_string() };

    let mut maps = MAPS.write();
    if let Some(map) = maps.get_mut(&handle) {
        map.items.insert(key_str, Value::List(value_handle));
        1
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_map_set_map(
    handle: u64,
    key: *const c_char,
    value_handle: u64,
) -> i32 {
    if key.is_null() {
        return 0;
    }

    let key_str = unsafe { CStr::from_ptr(key).to_str().unwrap_or("").to_string() };

    let mut maps = MAPS.write();
    if let Some(map) = maps.get_mut(&handle) {
        map.items.insert(key_str, Value::Map(value_handle));
        1
    } else {
        0
    }
}

// ============================================================================
// panic(msg) - Terminate execution with error message
// ============================================================================

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_panic(msg: *const c_char) {
    let message = if msg.is_null() {
        "panic: unknown error".to_string()
    } else {
        unsafe {
            CStr::from_ptr(msg)
                .to_str()
                .unwrap_or("panic: invalid error message")
                .to_string()
        }
    };

    // Set panic state in thread-local storage
    PANIC_STATE.with(|state| {
        *state.borrow_mut() = Some(message.clone());
    });

    // Use Rust's panic mechanism
    panic!("{}", message);
}

// ============================================================================
// recover() -> any - Recover from panic
// Returns the panic message if recovering, null otherwise
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_recover() -> *mut c_char {
    PANIC_STATE.with(|state| {
        if let Some(ref msg) = *state.borrow() {
            CString::new(msg.clone())
                .ok()
                .map(CString::into_raw)
                .unwrap_or(std::ptr::null_mut())
        } else {
            std::ptr::null_mut()
        }
    })
}

// ============================================================================
// try(fn) -> (any, Error) - Execute function and return result or error
// Since FFI doesn't support tuples well, we'll use a different approach:
// try returns an opaque handle, and we have separate functions to get result/error
// ============================================================================

struct TryResult {
    result: Option<String>,
    error: Option<HandleId>,
}

static TRY_RESULTS: Lazy<RwLock<std::collections::HashMap<HandleId, TryResult>>> =
    Lazy::new(|| RwLock::new(std::collections::HashMap::new()));

// Function pointer type for try
type TryFn = extern "C" fn() -> *mut c_char;

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_try(callback: TryFn) -> u64 {
    let id = next_handle_id();

    // Reset panic state
    PANIC_STATE.with(|state| {
        *state.borrow_mut() = None;
    });

    // Execute function in a catch_unwind block
    let result = catch_unwind(AssertUnwindSafe(|| callback()));

    match result {
        Ok(ptr) => {
            if ptr.is_null() {
                // Function returned null, might be an error
                // Check if panic was set
                let panic_msg = PANIC_STATE.with(|state| state.borrow().clone());

                if let Some(msg) = panic_msg {
                    // Create error handle
                    let error_id = next_handle_id();
                    ERRORS.write().insert(error_id, Error { message: msg });

                    TRY_RESULTS.write().insert(
                        id,
                        TryResult {
                            result: None,
                            error: Some(error_id),
                        },
                    );
                } else {
                    // Null result but no panic - treat as success with null
                    TRY_RESULTS.write().insert(
                        id,
                        TryResult {
                            result: Some(String::new()),
                            error: None,
                        },
                    );
                }
            } else {
                // Success - extract string
                let value = unsafe { CStr::from_ptr(ptr).to_str().unwrap_or("").to_string() };
                unsafe {
                    let _ = CString::from_raw(ptr);
                }

                TRY_RESULTS.write().insert(
                    id,
                    TryResult {
                        result: Some(value),
                        error: None,
                    },
                );
            }
        }
        Err(_) => {
            // Panic occurred
            let panic_msg = PANIC_STATE.with(|state| {
                state
                    .borrow()
                    .clone()
                    .unwrap_or_else(|| "panic: unknown error".to_string())
            });

            let error_id = next_handle_id();
            ERRORS
                .write()
                .insert(error_id, Error { message: panic_msg });

            TRY_RESULTS.write().insert(
                id,
                TryResult {
                    result: None,
                    error: Some(error_id),
                },
            );
        }
    }

    id
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_try_result(handle: u64) -> *mut c_char {
    let try_results = TRY_RESULTS.read();
    if let Some(try_result) = try_results.get(&handle) {
        if let Some(ref result) = try_result.result {
            CString::new(result.clone())
                .ok()
                .map(CString::into_raw)
                .unwrap_or(std::ptr::null_mut())
        } else {
            std::ptr::null_mut()
        }
    } else {
        std::ptr::null_mut()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_try_error(handle: u64) -> u64 {
    let try_results = TRY_RESULTS.read();
    if let Some(try_result) = try_results.get(&handle) {
        try_result.error.unwrap_or(0)
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_error_message(error_handle: u64) -> *mut c_char {
    let errors = ERRORS.read();
    if let Some(error) = errors.get(&error_handle) {
        CString::new(error.message.clone())
            .ok()
            .map(CString::into_raw)
            .unwrap_or(std::ptr::null_mut())
    } else {
        std::ptr::null_mut()
    }
}

// ============================================================================
// defer(fn) - Defer function execution until scope exit
// ============================================================================

type DeferFn = extern "C" fn();

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_defer(callback: DeferFn) {
    DEFER_STACK.with(|stack| {
        stack.borrow_mut().push(callback);
    });
}

/// Execute all deferred functions (called at scope exit)
/// This should be called by the compiler-generated code
#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_run_defers() {
    DEFER_STACK.with(|stack| {
        let mut stack_ref = stack.borrow_mut();
        while let Some(callback) = stack_ref.pop() {
            callback();
        }
    });
}

// ============================================================================
// type_of(x) - Get type of a value as string
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_type_of_string(_s: *const c_char) -> *mut c_char {
    CString::new("string")
        .ok()
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_type_of_int(_i: i64) -> *mut c_char {
    CString::new("int")
        .ok()
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_type_of_float(_f: f64) -> *mut c_char {
    CString::new("float")
        .ok()
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_type_of_bool(_b: bool) -> *mut c_char {
    CString::new("bool")
        .ok()
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_type_of_list(_handle: u64) -> *mut c_char {
    CString::new("list")
        .ok()
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_type_of_map(_handle: u64) -> *mut c_char {
    CString::new("map")
        .ok()
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_type_of_opaque(_handle: u64) -> *mut c_char {
    CString::new("opaque")
        .ok()
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

// ============================================================================
// fields(obj) - Get fields of an object/struct
// For now, we'll return a JSON string with field information
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_fields(_obj: u64) -> *mut c_char {
    // For now, return empty JSON object
    // Future: track struct definitions and return field list
    CString::new("{}")
        .ok()
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

// ============================================================================
// stringify(x) - Convert value to string
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_stringify_int(value: i64) -> *mut c_char {
    CString::new(value.to_string())
        .ok()
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_stringify_float(value: f64) -> *mut c_char {
    CString::new(value.to_string())
        .ok()
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_stringify_bool(value: bool) -> *mut c_char {
    CString::new(if value { "true" } else { "false" })
        .ok()
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_stringify_string(s: *const c_char) -> *mut c_char {
    if s.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        if let Ok(str_ref) = CStr::from_ptr(s).to_str() {
            CString::new(str_ref)
                .ok()
                .map(CString::into_raw)
                .unwrap_or(std::ptr::null_mut())
        } else {
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_stringify_list(handle: u64) -> *mut c_char {
    let lists = LISTS.read();
    if let Some(list) = lists.get(&handle) {
        let items: Vec<String> = list
            .items
            .iter()
            .map(value_to_string)
            .collect();
        let json = format!("[{}]", items.join(", "));
        CString::new(json)
            .ok()
            .map(CString::into_raw)
            .unwrap_or(std::ptr::null_mut())
    } else {
        CString::new("[]")
            .ok()
            .map(CString::into_raw)
            .unwrap_or(std::ptr::null_mut())
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_stringify_map(handle: u64) -> *mut c_char {
    let maps = MAPS.read();
    if let Some(map) = maps.get(&handle) {
        let items: Vec<String> = map
            .items
            .iter()
            .map(|(k, v)| format!("\"{}\": {}", k, value_to_string(v)))
            .collect();
        let json = format!("{{{}}}", items.join(", "));
        CString::new(json)
            .ok()
            .map(CString::into_raw)
            .unwrap_or(std::ptr::null_mut())
    } else {
        CString::new("{}")
            .ok()
            .map(CString::into_raw)
            .unwrap_or(std::ptr::null_mut())
    }
}

// ============================================================================
// select { case ... } - Channel select (simplified version)
// For now, implements a simple select that tries channels in order
// ============================================================================

#[repr(C)]
pub struct SelectCase {
    channel: u64,
    is_send: bool,
    value: *const c_char, // For send operations
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_select_case_create(
    channel: u64,
    is_send: bool,
    value: *const c_char,
) -> *mut SelectCase {
    let case = Box::new(SelectCase {
        channel,
        is_send,
        value,
    });
    Box::into_raw(case)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_select_case_free(case: *mut SelectCase) {
    if !case.is_null() {
        // Safety:
        // `SelectCase` is
        //  * Only freed by this function;
        //  * only created using the global allocator;
        unsafe { Box::from_raw(case) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_select(
    cases: *const SelectCase,
    num_cases: i64,
    default_available: bool,
) -> i64 {
    if cases.is_null() || num_cases <= 0 {
        return -1; // No case selected
    }

    // For now, try cases in order until one succeeds
    // This is a simplified implementation - full select would use polling
    unsafe {
        let cases_slice = std::slice::from_raw_parts(cases, num_cases as usize);

        for (idx, case) in cases_slice.iter().enumerate() {
            if case.is_send {
                // Try send - check if channel has space (is not full)
                // For now, assume channels are unbuffered, so try immediate send
                // In a full implementation, this would check if receiver is waiting
                let can_send = true; // Simplified: assume we can always send for now

                if can_send {
                    // For send operations, we need to determine the value type
                    // This is a simplified implementation - in practice we'd need
                    // type information or separate select functions for different types
                    return idx as i64; // Case succeeded
                }
            } else {
                // Try receive - check if channel has data available
                let has_data = true; // Simplified: assume data is available for now

                if has_data {
                    return idx as i64; // Case succeeded
                }
            }
        }
    }

    // No case succeeded
    if default_available {
        -1 // Default case
    } else {
        -2 // Block (would block in real implementation)
    }
}

// ============================================================================
// Symbol Registration
// ============================================================================

fn register_builtin_symbols(registry: &SymbolRegistry) {
    // len() functions
    registry.register(FfiFunction {
        name: "len".into(),
        symbol: "otter_builtin_len_string".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "len<list>".into(),
        symbol: "otter_builtin_len_list".into(),
        signature: FfiSignature::new(vec![FfiType::List], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "len<map>".into(),
        symbol: "otter_builtin_len_map".into(),
        signature: FfiSignature::new(vec![FfiType::Map], FfiType::I64),
    });

    // cap() functions
    registry.register(FfiFunction {
        name: "cap".into(),
        symbol: "otter_builtin_cap_string".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "cap<list>".into(),
        symbol: "otter_builtin_cap_list".into(),
        signature: FfiSignature::new(vec![FfiType::List], FfiType::I64),
    });

    // str.contains() method
    registry.register(FfiFunction {
        name: "str.contains".into(),
        symbol: "otter_builtin_str_contains".into(),
        signature: FfiSignature::new(vec![FfiType::Str, FfiType::Str], FfiType::Bool),
    });

    // append() functions
    registry.register(FfiFunction {
        name: "append<list,string>".into(),
        symbol: "otter_builtin_append_list_string".into(),
        signature: FfiSignature::new(vec![FfiType::List, FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "append<list,int>".into(),
        symbol: "otter_builtin_append_list_int".into(),
        signature: FfiSignature::new(vec![FfiType::List, FfiType::I64], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "append<list,float>".into(),
        symbol: "otter_builtin_append_list_float".into(),
        signature: FfiSignature::new(vec![FfiType::List, FfiType::F64], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "append<list,bool>".into(),
        symbol: "otter_builtin_append_list_bool".into(),
        signature: FfiSignature::new(vec![FfiType::List, FfiType::Bool], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "append<list,list>".into(),
        symbol: "otter_builtin_append_list_list".into(),
        signature: FfiSignature::new(vec![FfiType::List, FfiType::List], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "append<list,map>".into(),
        symbol: "otter_builtin_append_list_map".into(),
        signature: FfiSignature::new(vec![FfiType::List, FfiType::Map], FfiType::I32),
    });

    // delete() function
    registry.register(FfiFunction {
        name: "delete<map>".into(),
        symbol: "otter_builtin_delete_map".into(),
        signature: FfiSignature::new(vec![FfiType::Map, FfiType::Str], FfiType::I32),
    });

    // range() functions
    registry.register(FfiFunction {
        name: "range<int>".into(),
        symbol: "otter_builtin_range_int".into(),
        signature: FfiSignature::new(vec![FfiType::I64, FfiType::I64], FfiType::List),
    });

    registry.register(FfiFunction {
        name: "range<float>".into(),
        symbol: "otter_builtin_range_float".into(),
        signature: FfiSignature::new(vec![FfiType::F64, FfiType::F64], FfiType::List),
    });

    // enumerate() function
    registry.register(FfiFunction {
        name: "enumerate<list>".into(),
        symbol: "otter_builtin_enumerate_list".into(),
        signature: FfiSignature::new(vec![FfiType::List], FfiType::List),
    });

    // Helper functions
    registry.register(FfiFunction {
        name: "list.new".into(),
        symbol: "otter_builtin_list_new".into(),
        signature: FfiSignature::new(vec![], FfiType::List),
    });

    registry.register(FfiFunction {
        name: "map.new".into(),
        symbol: "otter_builtin_map_new".into(),
        signature: FfiSignature::new(vec![], FfiType::Map),
    });

    registry.register(FfiFunction {
        name: "list.get".into(),
        symbol: "otter_builtin_list_get".into(),
        signature: FfiSignature::new(vec![FfiType::List, FfiType::I64], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "list.get_int".into(),
        symbol: "otter_builtin_list_get_int".into(),
        signature: FfiSignature::new(vec![FfiType::List, FfiType::I64], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "list.get_float".into(),
        symbol: "otter_builtin_list_get_float".into(),
        signature: FfiSignature::new(vec![FfiType::List, FfiType::I64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "list.get_bool".into(),
        symbol: "otter_builtin_list_get_bool".into(),
        signature: FfiSignature::new(vec![FfiType::List, FfiType::I64], FfiType::Bool),
    });

    registry.register(FfiFunction {
        name: "list.get_list".into(),
        symbol: "otter_builtin_list_get_list".into(),
        signature: FfiSignature::new(vec![FfiType::List, FfiType::I64], FfiType::List),
    });

    registry.register(FfiFunction {
        name: "list.get_map".into(),
        symbol: "otter_builtin_list_get_map".into(),
        signature: FfiSignature::new(vec![FfiType::List, FfiType::I64], FfiType::Map),
    });

    registry.register(FfiFunction {
        name: "map.get".into(),
        symbol: "otter_builtin_map_get".into(),
        signature: FfiSignature::new(vec![FfiType::Map, FfiType::Str], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "map.get_int".into(),
        symbol: "otter_builtin_map_get_int".into(),
        signature: FfiSignature::new(vec![FfiType::Map, FfiType::Str], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "map.get_float".into(),
        symbol: "otter_builtin_map_get_float".into(),
        signature: FfiSignature::new(vec![FfiType::Map, FfiType::Str], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "map.get_bool".into(),
        symbol: "otter_builtin_map_get_bool".into(),
        signature: FfiSignature::new(vec![FfiType::Map, FfiType::Str], FfiType::Bool),
    });

    registry.register(FfiFunction {
        name: "map.get_list".into(),
        symbol: "otter_builtin_map_get_list".into(),
        signature: FfiSignature::new(vec![FfiType::Map, FfiType::Str], FfiType::List),
    });

    registry.register(FfiFunction {
        name: "map.get_map".into(),
        symbol: "otter_builtin_map_get_map".into(),
        signature: FfiSignature::new(vec![FfiType::Map, FfiType::Str], FfiType::Map),
    });

    registry.register(FfiFunction {
        name: "map.set".into(),
        symbol: "otter_builtin_map_set".into(),
        signature: FfiSignature::new(vec![FfiType::Map, FfiType::Str, FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "set<map,int>".into(),
        symbol: "otter_builtin_map_set_int".into(),
        signature: FfiSignature::new(vec![FfiType::Map, FfiType::Str, FfiType::I64], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "set<map,float>".into(),
        symbol: "otter_builtin_map_set_float".into(),
        signature: FfiSignature::new(vec![FfiType::Map, FfiType::Str, FfiType::F64], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "set<map,bool>".into(),
        symbol: "otter_builtin_map_set_bool".into(),
        signature: FfiSignature::new(
            vec![FfiType::Map, FfiType::Str, FfiType::Bool],
            FfiType::I32,
        ),
    });

    registry.register(FfiFunction {
        name: "set<map,list>".into(),
        symbol: "otter_builtin_map_set_list".into(),
        signature: FfiSignature::new(
            vec![FfiType::Map, FfiType::Str, FfiType::List],
            FfiType::I32,
        ),
    });

    registry.register(FfiFunction {
        name: "set<map,map>".into(),
        symbol: "otter_builtin_map_set_map".into(),
        signature: FfiSignature::new(vec![FfiType::Map, FfiType::Str, FfiType::Map], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "set<map,int>".into(),
        symbol: "otter_builtin_map_set_int".into(),
        signature: FfiSignature::new(vec![FfiType::Map, FfiType::Str, FfiType::I64], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "set<map,float>".into(),
        symbol: "otter_builtin_map_set_float".into(),
        signature: FfiSignature::new(vec![FfiType::Map, FfiType::Str, FfiType::F64], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "set<map,bool>".into(),
        symbol: "otter_builtin_map_set_bool".into(),
        signature: FfiSignature::new(
            vec![FfiType::Map, FfiType::Str, FfiType::Bool],
            FfiType::I32,
        ),
    });

    registry.register(FfiFunction {
        name: "set<map,list>".into(),
        symbol: "otter_builtin_map_set_list".into(),
        signature: FfiSignature::new(
            vec![FfiType::Map, FfiType::Str, FfiType::List],
            FfiType::I32,
        ),
    });

    registry.register(FfiFunction {
        name: "set<map,map>".into(),
        symbol: "otter_builtin_map_set_map".into(),
        signature: FfiSignature::new(vec![FfiType::Map, FfiType::Str, FfiType::Map], FfiType::I32),
    });

    // Error handling functions
    registry.register(FfiFunction {
        name: "panic".into(),
        symbol: "otter_builtin_panic".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "recover".into(),
        symbol: "otter_builtin_recover".into(),
        signature: FfiSignature::new(vec![], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "try".into(),
        symbol: "otter_builtin_try".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "try.result".into(),
        symbol: "otter_builtin_try_result".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "try.error".into(),
        symbol: "otter_builtin_try_error".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "error.message".into(),
        symbol: "otter_builtin_error_message".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Str),
    });

    // defer() function
    registry.register(FfiFunction {
        name: "defer".into(),
        symbol: "otter_builtin_defer".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Unit),
    });

    // type_of() functions
    registry.register(FfiFunction {
        name: "type_of<string>".into(),
        symbol: "otter_builtin_type_of_string".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "type_of<int>".into(),
        symbol: "otter_builtin_type_of_int".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "type_of<float>".into(),
        symbol: "otter_builtin_type_of_float".into(),
        signature: FfiSignature::new(vec![FfiType::F64], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "type_of<bool>".into(),
        symbol: "otter_builtin_type_of_bool".into(),
        signature: FfiSignature::new(vec![FfiType::Bool], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "type_of<list>".into(),
        symbol: "otter_builtin_type_of_list".into(),
        signature: FfiSignature::new(vec![FfiType::List], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "type_of<map>".into(),
        symbol: "otter_builtin_type_of_map".into(),
        signature: FfiSignature::new(vec![FfiType::Map], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "type_of<opaque>".into(),
        symbol: "otter_builtin_type_of_opaque".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Str),
    });

    // fields() function
    registry.register(FfiFunction {
        name: "fields".into(),
        symbol: "otter_builtin_fields".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Str),
    });

    // stringify() functions
    registry.register(FfiFunction {
        name: "stringify<int>".into(),
        symbol: "otter_builtin_stringify_int".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "stringify<float>".into(),
        symbol: "otter_builtin_stringify_float".into(),
        signature: FfiSignature::new(vec![FfiType::F64], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "stringify<bool>".into(),
        symbol: "otter_builtin_stringify_bool".into(),
        signature: FfiSignature::new(vec![FfiType::Bool], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "stringify<string>".into(),
        symbol: "otter_builtin_stringify_string".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "stringify<list>".into(),
        symbol: "otter_builtin_stringify_list".into(),
        signature: FfiSignature::new(vec![FfiType::List], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "stringify<map>".into(),
        symbol: "otter_builtin_stringify_map".into(),
        signature: FfiSignature::new(vec![FfiType::Map], FfiType::Str),
    });

    // Select statement functions
    registry.register(FfiFunction {
        name: "select.case".into(),
        symbol: "otter_builtin_select_case_create".into(),
        signature: FfiSignature::new(
            vec![FfiType::I64, FfiType::Bool, FfiType::Str],
            FfiType::Opaque,
        ),
    });

    registry.register(FfiFunction {
        name: "select".into(),
        symbol: "otter_builtin_select".into(),
        signature: FfiSignature::new(vec![FfiType::List, FfiType::Bool], FfiType::I64),
    });
}

inventory::submit! {
    crate::runtime::ffi::SymbolProvider {
        register: register_builtin_symbols,
    }
}
