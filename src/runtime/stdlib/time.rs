use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[cfg(feature = "task-runtime")]
use parking_lot::{Condvar, Mutex};
#[cfg(feature = "task-runtime")]
use std::sync::Arc;
#[cfg(feature = "task-runtime")]
use std::task::Waker;

use once_cell::sync::Lazy;
use parking_lot::RwLock;

use crate::runtime::symbol_registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

#[cfg(feature = "task-runtime")]
use crate::runtime::task::runtime;

// ============================================================================
// Time and Duration Structures
// ============================================================================

type HandleId = u64;
static NEXT_HANDLE_ID: AtomicU64 = AtomicU64::new(1);

fn next_handle_id() -> HandleId {
    NEXT_HANDLE_ID.fetch_add(1, Ordering::SeqCst)
}

struct Time {
    epoch_ms: i64,
}

struct DurationHandle {
    ms: i64,
}

static TIMES: Lazy<RwLock<std::collections::HashMap<HandleId, Time>>> =
    Lazy::new(|| RwLock::new(std::collections::HashMap::new()));

static DURATIONS: Lazy<RwLock<std::collections::HashMap<HandleId, DurationHandle>>> =
    Lazy::new(|| RwLock::new(std::collections::HashMap::new()));

#[no_mangle]
pub extern "C" fn otter_std_time_now() -> u64 {
    let id = next_handle_id();
    let now = chrono::Utc::now().timestamp_millis();
    let time = Time { epoch_ms: now };
    TIMES.write().insert(id, time);
    id
}

#[no_mangle]
pub extern "C" fn otter_std_time_now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[no_mangle]
pub extern "C" fn otter_std_time_sleep_ms(milliseconds: i64) {
    if milliseconds <= 0 {
        return;
    }

    #[cfg(feature = "task-runtime")]
    {
        // Use timer wheel for non-blocking sleep
        let timer_wheel = runtime().scheduler().timer_wheel();
        let condvar_pair = Arc::new((Mutex::new(false), Condvar::new()));

        // Create a waker that will notify the condvar
        let pair = Arc::clone(&condvar_pair);
        let waker = create_condvar_waker(pair.clone());

        // Schedule wakeup
        timer_wheel.schedule_wakeup(Duration::from_millis(milliseconds as u64), waker);

        // Block until woken
        let mut ready = condvar_pair.0.lock();
        while !*ready {
            condvar_pair.1.wait(&mut ready);
        }
        return;
    }

    #[cfg(not(feature = "task-runtime"))]
    {
        std::thread::sleep(Duration::from_millis(milliseconds as u64));
    }
}

#[cfg(feature = "task-runtime")]
fn create_condvar_waker(pair: Arc<(Mutex<bool>, Condvar)>) -> Waker {
    use std::task::{RawWaker, RawWakerVTable};

    unsafe fn clone(data: *const ()) -> RawWaker {
        RawWaker::new(data, &VTABLE)
    }

    unsafe fn wake(data: *const ()) {
        let pair = Arc::from_raw(data as *const Arc<(Mutex<bool>, Condvar)>);
        *pair.0.lock() = true;
        pair.1.notify_all();
    }

    unsafe fn wake_by_ref(data: *const ()) {
        // For wake_by_ref, we need to use the Arc without consuming it
        // So we'll clone it first
        let pair = Arc::from_raw(data as *const Arc<(Mutex<bool>, Condvar)>);
        let pair_clone = Arc::clone(&pair);
        std::mem::forget(pair); // Don't drop the original
        *pair_clone.0.lock() = true;
        pair_clone.1.notify_all();
    }

    unsafe fn drop(data: *const ()) {
        let _ = Arc::from_raw(data as *const Arc<(Mutex<bool>, Condvar)>);
    }

    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop);

    let pair_ptr = Arc::into_raw(pair);
    unsafe { Waker::from_raw(RawWaker::new(pair_ptr as *const (), &VTABLE)) }
}

#[no_mangle]
pub extern "C" fn otter_std_time_since(t: u64) -> u64 {
    let times = TIMES.read();
    if let Some(start_time) = times.get(&t) {
        let now = chrono::Utc::now().timestamp_millis();
        let duration_ms = now - start_time.epoch_ms;

        let id = next_handle_id();
        let duration = DurationHandle { ms: duration_ms };
        drop(times);
        DURATIONS.write().insert(id, duration);
        id
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn otter_std_time_format(t: u64, fmt: *const c_char) -> *mut c_char {
    if fmt.is_null() {
        return std::ptr::null_mut();
    }

    let format_str = unsafe {
        CStr::from_ptr(fmt)
            .to_str()
            .unwrap_or("%Y-%m-%d %H:%M:%S")
            .to_string()
    };

    let times = TIMES.read();
    if let Some(time) = times.get(&t) {
        let dt = chrono::DateTime::from_timestamp_millis(time.epoch_ms);
        if let Some(dt) = dt {
            let formatted = dt.format(&format_str).to_string();
            CString::new(formatted)
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

#[no_mangle]
pub extern "C" fn otter_std_time_parse(fmt: *const c_char, text: *const c_char) -> u64 {
    if fmt.is_null() || text.is_null() {
        return 0;
    }

    let format_str = unsafe {
        CStr::from_ptr(fmt)
            .to_str()
            .unwrap_or("%Y-%m-%d %H:%M:%S")
            .to_string()
    };

    let text_str = unsafe { CStr::from_ptr(text).to_str().unwrap_or("").to_string() };

    // Try to parse using chrono
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&text_str, &format_str) {
        let epoch_ms = dt.and_utc().timestamp_millis();
        let id = next_handle_id();
        let time = Time { epoch_ms };
        TIMES.write().insert(id, time);
        id
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn otter_std_time_tick(ms: i64) -> u64 {
    let id = next_handle_id();
    let time = Time { epoch_ms: ms };
    TIMES.write().insert(id, time);
    id
}

#[no_mangle]
pub extern "C" fn otter_std_time_after(ms: i64) -> u64 {
    let id = next_handle_id();
    let now = chrono::Utc::now().timestamp_millis();
    let time = Time { epoch_ms: now + ms };
    TIMES.write().insert(id, time);
    id
}

#[no_mangle]
pub extern "C" fn otter_std_time_epoch_ms(t: u64) -> i64 {
    let times = TIMES.read();
    if let Some(time) = times.get(&t) {
        time.epoch_ms
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn otter_std_duration_ms(d: u64) -> i64 {
    let durations = DURATIONS.read();
    if let Some(duration) = durations.get(&d) {
        duration.ms
    } else {
        0
    }
}

fn register_std_time_symbols(registry: &SymbolRegistry) {
    registry.register(FfiFunction {
        name: "time.now".into(),
        symbol: "otter_std_time_now".into(),
        signature: FfiSignature::new(vec![], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "std.time.now".into(),
        symbol: "otter_std_time_now_ms".into(),
        signature: FfiSignature::new(vec![], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "time.sleep".into(),
        symbol: "otter_std_time_sleep_ms".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "time.since".into(),
        symbol: "otter_std_time_since".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "time.format".into(),
        symbol: "otter_std_time_format".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque, FfiType::Str], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "time.parse".into(),
        symbol: "otter_std_time_parse".into(),
        signature: FfiSignature::new(vec![FfiType::Str, FfiType::Str], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "time.tick".into(),
        symbol: "otter_std_time_tick".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "time.after".into(),
        symbol: "otter_std_time_after".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "time.epoch_ms".into(),
        symbol: "otter_std_time_epoch_ms".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "duration.ms".into(),
        symbol: "otter_std_duration_ms".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::I64),
    });

    // Convenience aliases
    registry.register(FfiFunction {
        name: "time.now_ms".into(),
        symbol: "otter_std_time_now_ms".into(),
        signature: FfiSignature::new(vec![], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "time.now_us".into(),
        symbol: "otter_std_time_now_us".into(),
        signature: FfiSignature::new(vec![], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "time.now_ns".into(),
        symbol: "otter_std_time_now_ns".into(),
        signature: FfiSignature::new(vec![], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "time.now_sec".into(),
        symbol: "otter_std_time_now_sec".into(),
        signature: FfiSignature::new(vec![], FfiType::I64),
    });
}

inventory::submit! {
    crate::runtime::ffi::SymbolProvider {
        register: register_std_time_symbols,
    }
}
