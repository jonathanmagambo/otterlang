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
pub enum Value {
    Unit,
    Bool(bool),
    I64(i64),
    F64(f64),
    String(String),
    List(HandleId),
    Map(HandleId),
}

// Runtime Value Handle Registry
// Instead of encoding type info into bits, we store full-precision values
// in a registry and return handles with embedded type information

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ValueKind {
    Unit = 0,
    Bool = 1,
    I64 = 2,
    F64 = 3,
    String = 4,
    List = 5,
    Map = 6,
}

struct RuntimeValue {
    value: Value,
}

static RUNTIME_VALUES: Lazy<RwLock<std::collections::HashMap<HandleId, RuntimeValue>>> =
    Lazy::new(|| RwLock::new(std::collections::HashMap::new()));

// Encode value handle with type tag in upper 8 bits, handle ID in lower 56 bits
// This gives us full precision for all types
const TAG_SHIFT: u32 = 56;
const HANDLE_MASK: u64 = (1u64 << TAG_SHIFT) - 1;

pub(crate) fn encode_runtime_value(value: &Value) -> u64 {
    // For simple scalar values that fit in 56 bits without loss, encode directly
    match value {
        Value::Unit => (ValueKind::Unit as u64) << TAG_SHIFT,
        Value::Bool(b) => {
            let val = if *b { 1u64 } else { 0u64 };
            ((ValueKind::Bool as u64) << TAG_SHIFT) | val
        }
        Value::I64(_i) => {
            // For I64, we need full 64-bit precision, so use handle
            let handle_id = next_handle_id();
            RUNTIME_VALUES.write().insert(
                handle_id,
                RuntimeValue {
                    value: value.clone(),
                },
            );
            ((ValueKind::I64 as u64) << TAG_SHIFT) | (handle_id & HANDLE_MASK)
        }
        Value::F64(_f) => {
            // For F64, we need full 64-bit precision, so use handle
            let handle_id = next_handle_id();
            RUNTIME_VALUES.write().insert(
                handle_id,
                RuntimeValue {
                    value: value.clone(),
                },
            );
            ((ValueKind::F64 as u64) << TAG_SHIFT) | (handle_id & HANDLE_MASK)
        }
        Value::String(s) => {
            // For strings, store pointer directly with tag
            let ptr = CString::new(s.clone()).unwrap().into_raw() as u64;
            ((ValueKind::String as u64) << TAG_SHIFT) | (ptr & HANDLE_MASK)
        }
        Value::List(h) | Value::Map(h) => {
            // Lists and maps already use handles
            let tag = if matches!(value, Value::List(_)) {
                ValueKind::List
            } else {
                ValueKind::Map
            };
            ((tag as u64) << TAG_SHIFT) | (*h & HANDLE_MASK)
        }
    }
}

pub fn decode_value_kind(encoded: u64) -> ValueKind {
    let tag = (encoded >> TAG_SHIFT) as u8;
    match tag {
        1 => ValueKind::Bool,
        2 => ValueKind::I64,
        3 => ValueKind::F64,
        4 => ValueKind::String,
        5 => ValueKind::List,
        6 => ValueKind::Map,
        _ => ValueKind::Unit,
    }
}

pub fn decode_value_handle(encoded: u64) -> u64 {
    encoded & HANDLE_MASK
}

pub struct List {
    pub items: Vec<Value>,
}

pub static LISTS: Lazy<RwLock<std::collections::HashMap<HandleId, List>>> =
    Lazy::new(|| RwLock::new(std::collections::HashMap::new()));

struct Map {
    items: std::collections::HashMap<String, Value>,
}

static MAPS: Lazy<RwLock<std::collections::HashMap<HandleId, Map>>> =
    Lazy::new(|| RwLock::new(std::collections::HashMap::new()));

struct ArrayIterator {
    handle: HandleId,
    index: usize,
}

static ARRAY_ITERATORS: Lazy<RwLock<std::collections::HashMap<HandleId, ArrayIterator>>> =
    Lazy::new(|| RwLock::new(std::collections::HashMap::new()));

struct StringIterator {
    string: String,
    index: usize,
}

static STRING_ITERATORS: Lazy<RwLock<std::collections::HashMap<HandleId, StringIterator>>> =
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

#[unsafe(no_mangle)]
pub extern "C" fn otter_runtime_list_length(handle: u64) -> i64 {
    otter_builtin_len_list(handle)
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_runtime_list_get(handle: u64, index: i64) -> u64 {
    list_value(handle, index)
        .map(|value| encode_runtime_value(&value))
        .unwrap_or(0)
}

fn map_value(handle: HandleId, key: &str) -> Option<Value> {
    let maps = MAPS.read();
    maps.get(&handle)
        .and_then(|map| map.items.get(key).cloned())
}

fn stringify_list_handle(handle: HandleId) -> String {
    let lists = LISTS.read();
    if let Some(list) = lists.get(&handle) {
        let items = list.items.iter().map(value_to_string).collect::<Vec<_>>();
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

/// get the length of the given string
///
/// # Safety
///
/// this function dereferences a raw pointer
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

/// get the capacity of the given string
///
/// # Safety
///
/// this function dereferences a raw pointer
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

/// see if a string `s` contains substring `substring`
///
/// # Safety
///
/// this function dereferences a raw pointer
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

/// appends a string to the end of a list
///
/// # Safety
///
/// this function dereferences a raw pointer
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

/// deletes a key `key` from a map
///
/// # Safety
///
/// this function dereferences a raw pointer
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

/// get the length of the given string
///
/// # Safety
///
/// this function dereferences a raw pointer
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

/// insert a string key-value pair into a map
///
/// # Safety
///
/// this function dereferences a raw pointer
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

/// retrieves a key `key` from the map pointed to by `handle` and attempts a cast to an i64
///
/// # Safety
///
/// this function dereferences a raw pointer
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

/// retrieves a key `key` from the map pointed to by `handle` and attempts a cast to an f64
///
/// # Safety
///
/// this function dereferences a raw pointer
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

/// retrieves a key `key` from the map pointed to by `handle` and attempts a cast to a bool
///
/// # Safety
///
/// this function dereferences a raw pointer
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

/// retrieves a key `key` from the map pointed to by `handle` and attempts to
/// convert the value to a handle to a list
///
/// # Safety
///
/// this function dereferences a raw pointer
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

/// retrieves a key `key` from the map pointed to by `handle` and attempts to
/// convert the value to a handle to a map
///
/// # Safety
///
/// this function dereferences a raw pointer
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

/// retrieves a key `key` from the map pointed to by `handle` and sets the
/// current value to the new value `value`
///
/// # Safety
///
/// this function dereferences a raw pointer
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

/// retrieves a key `key` from the map pointed to by `handle` and sets the
/// current value to the new value `value`
///
/// # Safety
///
/// this function dereferences a raw pointer
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

/// retrieves a key `key` from the map pointed to by `handle` and sets the
/// current value to the new value `value`
///
/// # Safety
///
/// this function dereferences a raw pointer
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

/// retrieves a key `key` from the map pointed to by `handle` and sets the
/// current value to the new value `value_handle`
///
/// # Safety
///
/// this function dereferences a raw pointer
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

/// retrieves a key `key` from the map pointed to by `handle` and sets the
/// current value to the new value `value_handle`
///
/// # Safety
///
/// this function dereferences a raw pointer
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

// ============================================================================
// Array Iterator
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_iter_array(handle: u64) -> u64 {
    let id = next_handle_id();
    let iter = ArrayIterator { handle, index: 0 };
    ARRAY_ITERATORS.write().insert(id, iter);
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_iter_has_next_array(iter_handle: u64) -> bool {
    let iterators = ARRAY_ITERATORS.read();
    if let Some(iter) = iterators.get(&iter_handle) {
        let lists = LISTS.read();
        if let Some(list) = lists.get(&iter.handle) {
            iter.index < list.items.len()
        } else {
            false
        }
    } else {
        false
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_iter_next_array(iter_handle: u64) -> u64 {
    let mut iterators = ARRAY_ITERATORS.write();
    if let Some(iter) = iterators.get_mut(&iter_handle) {
        if let Some(val) = list_value(iter.handle, iter.index as i64) {
            iter.index += 1;
            encode_runtime_value(&val)
        } else {
            0
        }
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_iter_free_array(iter_handle: u64) {
    ARRAY_ITERATORS.write().remove(&iter_handle);
}

/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer.
/// The caller must ensure that `ptr` points to a valid null-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_builtin_iter_string(ptr: *const c_char) -> u64 {
    let s = unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() };
    let id = next_handle_id();
    let iter = StringIterator {
        string: s,
        index: 0,
    };
    STRING_ITERATORS.write().insert(id, iter);
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_iter_has_next_string(iter_handle: u64) -> bool {
    let iterators = STRING_ITERATORS.read();
    if let Some(iter) = iterators.get(&iter_handle) {
        iter.index < iter.string.len()
    } else {
        false
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_iter_next_string(iter_handle: u64) -> u64 {
    let mut iterators = STRING_ITERATORS.write();
    if let Some(iter) = iterators.get_mut(&iter_handle) {
        if iter.index < iter.string.len() {
            let mut chars = iter.string[iter.index..].chars();
            if let Some(c) = chars.next() {
                let char_len = c.len_utf8();
                iter.index += char_len;
                let s = c.to_string();
                // Return encoded value instead of raw pointer
                encode_runtime_value(&Value::String(s))
            } else {
                0
            }
        } else {
            0
        }
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_builtin_iter_free_string(iter_handle: u64) {
    STRING_ITERATORS.write().remove(&iter_handle);
}

/// otter-lang's builtin panic function
///
/// # Safety
///
/// this function dereferences a raw pointer
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

    #[expect(
        clippy::panic,
        reason = "TODO: Use a more robust panic handling mechanism"
    )]
    {
        panic!("{}", message);
    }
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

/// Converts an instance of `Cstr` into `CString`
///
/// # Safety
///
/// this function dereferences a raw pointer
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
        let items: Vec<String> = list.items.iter().map(value_to_string).collect();
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

/// Iterates through all cases pointed to by `cases`, if the case is send, we
/// send data to it, otherwise, try to receive data from it
///
/// # Safety
///
/// this function dereferences a raw pointer
/// caller must guarantee that `num_cases` is valid within the span `cases`
// Async/await runtime functions using tokio
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

#[derive(Clone)]
struct TaskHandle {
    handle: Arc<Mutex<Option<JoinHandle<i64>>>>,
}

impl TaskHandle {
    fn new() -> Self {
        Self {
            handle: Arc::new(Mutex::new(None)),
        }
    }
}

static TASK_HANDLES: Mutex<Vec<TaskHandle>> = Mutex::const_new(vec![]);

pub extern "C" fn otter_builtin_spawn_async(value: i64) -> u64 {
    // Spawn a new async task that just returns the value
    // In a real implementation, this would execute a closure or function
    let task_handle = TaskHandle::new();

    let handle_clone = task_handle.handle.clone();
    let value_clone = value;
    let tokio_handle = tokio::spawn(async move {
        // For now, just return the value after a brief delay to simulate async work
        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        value_clone
    });

    // Store the handle
    let mut guard = TASK_HANDLES.blocking_lock();
    let handle_id = guard.len() as u64;
    guard.push(task_handle);

    // Set the handle in a separate task to avoid blocking
    tokio::spawn(async move {
        let mut handle_guard = handle_clone.lock().await;
        *handle_guard = Some(tokio_handle);
    });

    handle_id
}

pub extern "C" fn otter_builtin_await_task(task_handle: u64) -> i64 {
    // Wait for the task to complete and return its result
    let guard = TASK_HANDLES.blocking_lock();
    if let Some(task) = guard.get(task_handle as usize) {
        let handle_clone = task.handle.clone();

        // Block on the async operation
        tokio::runtime::Handle::current().block_on(async {
            let mut handle_guard = handle_clone.lock().await;
            if let Some(join_handle) = handle_guard.take() {
                join_handle.await.unwrap_or(0)
            } else {
                0
            }
        })
    } else {
        0
    }
}

pub extern "C" fn otter_builtin_task_ready(task_handle: u64) -> bool {
    // Check if the task is ready (completed)
    let guard = TASK_HANDLES.blocking_lock();
    if let Some(task) = guard.get(task_handle as usize) {
        let handle_clone = task.handle.clone();

        tokio::runtime::Handle::current().block_on(async {
            let handle_guard = handle_clone.lock().await;
            if let Some(ref join_handle) = *handle_guard {
                join_handle.is_finished()
            } else {
                false
            }
        })
    } else {
        false
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
        name: "runtime.list.length".into(),
        symbol: "otter_runtime_list_length".into(),
        signature: FfiSignature::new(vec![FfiType::List], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "runtime.list.get".into(),
        symbol: "otter_runtime_list_get".into(),
        signature: FfiSignature::new(vec![FfiType::List, FfiType::I64], FfiType::I64),
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

    // Async/await functions
    registry.register(FfiFunction {
        name: "spawn_async".into(),
        symbol: "otter_builtin_spawn_async".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "await_task".into(),
        symbol: "otter_builtin_await_task".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "task_ready".into(),
        symbol: "otter_builtin_task_ready".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Bool),
    });
    // GC functions
    registry.register(FfiFunction {
        name: "gc.alloc".into(),
        symbol: "otter_alloc".into(),
        signature: FfiSignature {
            params: vec![FfiType::I64], // size
            result: FfiType::Opaque,    // ptr
        },
    });

    registry.register(FfiFunction {
        name: "gc.add_root".into(),
        symbol: "otter_gc_add_root".into(),
        signature: FfiSignature {
            params: vec![FfiType::Opaque], // ptr
            result: FfiType::Unit,
        },
    });

    registry.register(FfiFunction {
        name: "gc.remove_root".into(),
        symbol: "otter_gc_remove_root".into(),
        signature: FfiSignature {
            params: vec![FfiType::Opaque], // ptr
            result: FfiType::Unit,
        },
    });

    registry.register(FfiFunction {
        name: "gc.enable".into(),
        symbol: "otter_gc_enable".into(),
        signature: FfiSignature::new(vec![], FfiType::Bool),
    });

    registry.register(FfiFunction {
        name: "gc.disable".into(),
        symbol: "otter_gc_disable".into(),
        signature: FfiSignature::new(vec![], FfiType::Bool),
    });

    registry.register(FfiFunction {
        name: "gc.is_enabled".into(),
        symbol: "otter_gc_is_enabled".into(),
        signature: FfiSignature::new(vec![], FfiType::Bool),
    });

    registry.register(FfiFunction {
        name: "arena.create".into(),
        symbol: "otter_arena_create".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "arena.destroy".into(),
        symbol: "otter_arena_destroy".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::Bool),
    });

    registry.register(FfiFunction {
        name: "arena.reset".into(),
        symbol: "otter_arena_reset".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::Bool),
    });

    registry.register(FfiFunction {
        name: "arena.alloc".into(),
        symbol: "otter_arena_alloc".into(),
        signature: FfiSignature::new(
            vec![FfiType::I64, FfiType::I64, FfiType::I64],
            FfiType::Opaque,
        ),
    });

    // Array Iterator functions
    registry.register(FfiFunction {
        name: "__otter_iter_array".into(),
        symbol: "otter_builtin_iter_array".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "__otter_iter_has_next_array".into(),
        symbol: "otter_builtin_iter_has_next_array".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Bool),
    });

    registry.register(FfiFunction {
        name: "__otter_iter_next_array".into(),
        symbol: "otter_builtin_iter_next_array".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "__otter_iter_free_array".into(),
        symbol: "otter_builtin_iter_free_array".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Unit),
    });

    // String Iterator functions
    registry.register(FfiFunction {
        name: "__otter_iter_string".into(),
        symbol: "otter_builtin_iter_string".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "__otter_iter_has_next_string".into(),
        symbol: "otter_builtin_iter_has_next_string".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Bool),
    });

    registry.register(FfiFunction {
        name: "__otter_iter_next_string".into(),
        symbol: "otter_builtin_iter_next_string".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "__otter_iter_free_string".into(),
        symbol: "otter_builtin_iter_free_string".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Unit),
    });

    // Runtime value decoding functions with full precision
    registry.register(FfiFunction {
        name: "__otter_decode_value_kind".into(),
        symbol: "otter_decode_value_kind".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "__otter_decode_value_handle".into(),
        symbol: "otter_decode_value_handle".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "__otter_decode_value_as_bool".into(),
        symbol: "otter_decode_value_as_bool".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::Bool),
    });

    registry.register(FfiFunction {
        name: "__otter_decode_value_as_i64".into(),
        symbol: "otter_decode_value_as_i64".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "__otter_decode_value_as_f64".into(),
        symbol: "otter_decode_value_as_f64".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::F64),
    });

    registry.register(FfiFunction {
        name: "__otter_decode_value_as_string".into(),
        symbol: "otter_decode_value_as_string".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "__otter_decode_value_as_handle".into(),
        symbol: "otter_decode_value_as_handle".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "__otter_free_runtime_value".into(),
        symbol: "otter_free_runtime_value".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::Unit),
    });
}

// Export decode functions for FFI
#[unsafe(no_mangle)]
pub extern "C" fn otter_decode_value_kind(encoded: u64) -> u64 {
    decode_value_kind(encoded) as u64
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_decode_value_handle(encoded: u64) -> u64 {
    decode_value_handle(encoded)
}

// Type-specific decode functions for full precision
#[unsafe(no_mangle)]
pub extern "C" fn otter_decode_value_as_bool(encoded: u64) -> bool {
    let kind = decode_value_kind(encoded);
    let handle = decode_value_handle(encoded);

    match kind {
        ValueKind::Bool => handle != 0,
        _ => false,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_decode_value_as_i64(encoded: u64) -> i64 {
    let kind = decode_value_kind(encoded);
    let handle = decode_value_handle(encoded);

    match kind {
        ValueKind::I64 => {
            // Look up in registry for full precision
            let values = RUNTIME_VALUES.read();
            if let Some(Value::I64(i)) = values.get(&handle).map(|rv| rv.value.clone()) {
                return i;
            }
            0
        }
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_decode_value_as_f64(encoded: u64) -> f64 {
    let kind = decode_value_kind(encoded);
    let handle = decode_value_handle(encoded);

    match kind {
        ValueKind::F64 => {
            // Look up in registry for full precision
            let values = RUNTIME_VALUES.read();
            if let Some(Value::F64(f)) = values.get(&handle).map(|rv| rv.value.clone()) {
                return f;
            }
            0.0
        }
        _ => 0.0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_decode_value_as_string(encoded: u64) -> *mut c_char {
    let kind = decode_value_kind(encoded);
    let handle = decode_value_handle(encoded);

    match kind {
        ValueKind::String => {
            // Handle is actually a pointer
            if handle == 0 {
                CString::new("")
                    .ok()
                    .map(CString::into_raw)
                    .unwrap_or(std::ptr::null_mut())
            } else {
                unsafe {
                    let ptr = handle as *const c_char;
                    // Copy the string since we need to return a new owned pointer
                    if let Ok(cstr) = CStr::from_ptr(ptr).to_str() {
                        CString::new(cstr)
                            .ok()
                            .map(CString::into_raw)
                            .unwrap_or(std::ptr::null_mut())
                    } else {
                        std::ptr::null_mut()
                    }
                }
            }
        }
        _ => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_decode_value_as_handle(encoded: u64) -> u64 {
    decode_value_handle(encoded)
}

// Cleanup function to free runtime value handles
#[unsafe(no_mangle)]
pub extern "C" fn otter_free_runtime_value(encoded: u64) {
    let kind = decode_value_kind(encoded);
    let handle = decode_value_handle(encoded);

    // Only free registry entries for I64 and F64 which use handles
    if matches!(kind, ValueKind::I64 | ValueKind::F64) {
        RUNTIME_VALUES.write().remove(&handle);
    }
}

#[cfg(feature = "ffi-main")]
unsafe extern "C" {
    fn otter_entry();
}

// This main function is only for FFI-compiled programs, not for the binaries
#[cfg(feature = "ffi-main")]
#[unsafe(no_mangle)]
pub extern "C" fn main(_argc: i32, _argv: *const *const c_char) -> i32 {
    unsafe {
        otter_entry();
    }
    0
}

inventory::submit! {
    crate::runtime::ffi::SymbolProvider {
        namespace: "builtins",
        autoload: true,
        register: register_builtin_symbols,
    }
}
