use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

use once_cell::sync::Lazy;
use parking_lot::RwLock;

use otterc_symbol::registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

static NEXT_ENUM_HANDLE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
enum EnumFieldValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Ptr(u64),
}

#[derive(Debug, Clone)]
struct EnumObject {
    tag: i64,
    fields: Vec<EnumFieldValue>,
}

static ENUM_OBJECTS: Lazy<RwLock<std::collections::HashMap<u64, EnumObject>>> =
    Lazy::new(|| RwLock::new(std::collections::HashMap::new()));

fn next_enum_handle() -> u64 {
    NEXT_ENUM_HANDLE.fetch_add(1, Ordering::SeqCst)
}

fn with_enum_object<R>(handle: u64, f: impl FnOnce(&EnumObject) -> R) -> Option<R> {
    ENUM_OBJECTS.read().get(&handle).map(f)
}

fn with_enum_object_mut<R>(handle: u64, f: impl FnOnce(&mut EnumObject) -> R) -> Option<R> {
    ENUM_OBJECTS.write().get_mut(&handle).map(f)
}

fn set_field(handle: u64, index: usize, value: EnumFieldValue) -> bool {
    with_enum_object_mut(handle, |object| {
        if index >= object.fields.len() {
            return false;
        }
        object.fields[index] = value;
        true
    })
    .unwrap_or(false)
}

fn ptr_to_u64(ptr: *mut c_void) -> u64 {
    ptr as usize as u64
}

fn u64_to_ptr(value: u64) -> *mut c_void {
    value as usize as *mut c_void
}

/// # Safety
/// The caller must ensure `field_count` is non-negative and the returned handle is managed via
/// the enum runtime helpers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_enum_create(tag: i64, field_count: i64) -> u64 {
    if field_count < 0 {
        return 0;
    }

    let handle = next_enum_handle();
    let fields = vec![EnumFieldValue::Int(0); field_count as usize];
    ENUM_OBJECTS
        .write()
        .insert(handle, EnumObject { tag, fields });
    handle
}

/// # Safety
/// `handle` must be a valid enum handle obtained from `otter_enum_create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_enum_get_tag(handle: u64) -> i64 {
    with_enum_object(handle, |object| object.tag).unwrap_or(-1)
}

/// # Safety
/// `handle` must be a valid enum handle obtained from `otter_enum_create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_enum_get_field_count(handle: u64) -> i64 {
    with_enum_object(handle, |object| object.fields.len() as i64).unwrap_or(0)
}

/// # Safety
/// `handle` must refer to a valid enum and `index` must target an existing field for that variant.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_enum_set_i64(handle: u64, index: i64, value: i64) -> bool {
    if index < 0 {
        return false;
    }
    set_field(handle, index as usize, EnumFieldValue::Int(value))
}

/// # Safety
/// `handle` must refer to a valid enum and `index` must target an existing field for that variant.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_enum_set_f64(handle: u64, index: i64, value: f64) -> bool {
    if index < 0 {
        return false;
    }
    set_field(handle, index as usize, EnumFieldValue::Float(value))
}

/// # Safety
/// `handle` must refer to a valid enum and `index` must target an existing field for that variant.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_enum_set_bool(handle: u64, index: i64, value: bool) -> bool {
    if index < 0 {
        return false;
    }
    set_field(handle, index as usize, EnumFieldValue::Bool(value))
}

/// # Safety
/// `handle` must refer to a valid enum and `index` must target an existing field for that variant.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_enum_set_ptr(handle: u64, index: i64, value: *mut c_void) -> bool {
    if index < 0 {
        return false;
    }
    set_field(
        handle,
        index as usize,
        EnumFieldValue::Ptr(ptr_to_u64(value)),
    )
}

/// # Safety
/// `handle` must refer to a valid enum and `index` must target an existing field for that variant.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_enum_get_i64(handle: u64, index: i64) -> i64 {
    if index < 0 {
        return 0;
    }
    with_enum_object(handle, |object| match object.fields.get(index as usize) {
        Some(EnumFieldValue::Int(value)) => *value,
        Some(EnumFieldValue::Bool(value)) => *value as i64,
        Some(EnumFieldValue::Ptr(value)) => *value as i64,
        _ => 0,
    })
    .unwrap_or(0)
}

/// # Safety
/// `handle` must refer to a valid enum and `index` must target an existing field for that variant.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_enum_get_f64(handle: u64, index: i64) -> f64 {
    if index < 0 {
        return 0.0;
    }
    with_enum_object(handle, |object| match object.fields.get(index as usize) {
        Some(EnumFieldValue::Float(value)) => *value,
        Some(EnumFieldValue::Int(value)) => *value as f64,
        _ => 0.0,
    })
    .unwrap_or(0.0)
}

/// # Safety
/// `handle` must refer to a valid enum and `index` must target an existing field for that variant.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_enum_get_bool(handle: u64, index: i64) -> bool {
    if index < 0 {
        return false;
    }
    with_enum_object(handle, |object| match object.fields.get(index as usize) {
        Some(EnumFieldValue::Bool(value)) => *value,
        Some(EnumFieldValue::Int(value)) => *value != 0,
        _ => false,
    })
    .unwrap_or(false)
}

/// # Safety
/// `handle` must refer to a valid enum and `index` must target an existing field for that variant.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_enum_get_ptr(handle: u64, index: i64) -> *mut c_void {
    if index < 0 {
        return std::ptr::null_mut();
    }
    with_enum_object(handle, |object| match object.fields.get(index as usize) {
        Some(EnumFieldValue::Ptr(value)) => u64_to_ptr(*value),
        Some(EnumFieldValue::Int(value)) => u64_to_ptr(*value as u64),
        _ => std::ptr::null_mut(),
    })
    .unwrap_or(std::ptr::null_mut())
}

fn register_enum_functions(registry: &SymbolRegistry) {
    registry.register_many([
        FfiFunction {
            name: "runtime.enum.create".into(),
            symbol: "otter_enum_create".into(),
            signature: FfiSignature::new(vec![FfiType::I64, FfiType::I64], FfiType::I64),
        },
        FfiFunction {
            name: "runtime.enum.tag".into(),
            symbol: "otter_enum_get_tag".into(),
            signature: FfiSignature::new(vec![FfiType::I64], FfiType::I64),
        },
        FfiFunction {
            name: "runtime.enum.get_tag".into(),
            symbol: "otter_enum_get_tag".into(),
            signature: FfiSignature::new(vec![FfiType::I64], FfiType::I64),
        },
        FfiFunction {
            name: "runtime.enum.field_count".into(),
            symbol: "otter_enum_get_field_count".into(),
            signature: FfiSignature::new(vec![FfiType::I64], FfiType::I64),
        },
        FfiFunction {
            name: "runtime.enum.set_i64".into(),
            symbol: "otter_enum_set_i64".into(),
            signature: FfiSignature::new(
                vec![FfiType::I64, FfiType::I64, FfiType::I64],
                FfiType::Bool,
            ),
        },
        FfiFunction {
            name: "runtime.enum.set_f64".into(),
            symbol: "otter_enum_set_f64".into(),
            signature: FfiSignature::new(
                vec![FfiType::I64, FfiType::I64, FfiType::F64],
                FfiType::Bool,
            ),
        },
        FfiFunction {
            name: "runtime.enum.set_bool".into(),
            symbol: "otter_enum_set_bool".into(),
            signature: FfiSignature::new(
                vec![FfiType::I64, FfiType::I64, FfiType::Bool],
                FfiType::Bool,
            ),
        },
        FfiFunction {
            name: "runtime.enum.set_ptr".into(),
            symbol: "otter_enum_set_ptr".into(),
            signature: FfiSignature::new(
                vec![FfiType::I64, FfiType::I64, FfiType::I64],
                FfiType::Bool,
            ),
        },
        FfiFunction {
            name: "runtime.enum.get_i64".into(),
            symbol: "otter_enum_get_i64".into(),
            signature: FfiSignature::new(vec![FfiType::I64, FfiType::I64], FfiType::I64),
        },
        FfiFunction {
            name: "runtime.enum.get_f64".into(),
            symbol: "otter_enum_get_f64".into(),
            signature: FfiSignature::new(vec![FfiType::I64, FfiType::I64], FfiType::F64),
        },
        FfiFunction {
            name: "runtime.enum.get_bool".into(),
            symbol: "otter_enum_get_bool".into(),
            signature: FfiSignature::new(vec![FfiType::I64, FfiType::I64], FfiType::Bool),
        },
        FfiFunction {
            name: "runtime.enum.get_ptr".into(),
            symbol: "otter_enum_get_ptr".into(),
            signature: FfiSignature::new(vec![FfiType::I64, FfiType::I64], FfiType::I64),
        },
    ]);
}

inventory::submit! {
    otterc_ffi::SymbolProvider {
        namespace: "runtime.enum",
        autoload: true,
        register: register_enum_functions,
    }
}
