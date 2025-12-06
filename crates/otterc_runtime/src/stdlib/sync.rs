use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Once};
use std::thread;

use once_cell::sync::Lazy;
use parking_lot::{Mutex, RwLock};

use otterc_symbol::registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};

thread_local! {
    static THREAD_LOCKS: RefCell<HashSet<u64>> = RefCell::new(HashSet::new());
}

/// Handle IDs for opaque types
type HandleId = u64;

static NEXT_HANDLE_ID: AtomicU64 = AtomicU64::new(1);

fn next_handle_id() -> HandleId {
    NEXT_HANDLE_ID.fetch_add(1, Ordering::SeqCst)
}

struct MutexHandle {
    _id: HandleId,
    inner: Arc<Mutex<()>>,
}

struct WaitGroup {
    _id: HandleId,
    count: Arc<AtomicUsize>,
}

struct AtomicInt {
    _id: HandleId,
    value: Arc<AtomicU64>,
}

struct OnceHandle {
    _id: HandleId,
    inner: Arc<Once>,
}

static MUTEXES: Lazy<RwLock<HashMap<HandleId, MutexHandle>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));
static WAIT_GROUPS: Lazy<RwLock<HashMap<HandleId, WaitGroup>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));
static ATOMICS: Lazy<RwLock<HashMap<HandleId, AtomicInt>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));
static ONCE_HANDLES: Lazy<RwLock<HashMap<HandleId, OnceHandle>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

// ============================================================================
// Mutex Operations
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn otter_sync_mutex() -> u64 {
    let id = next_handle_id();
    let mutex = MutexHandle {
        _id: id,
        inner: Arc::new(Mutex::new(())),
    };

    MUTEXES.write().insert(id, mutex);
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_sync_lock(handle: u64) {
    let mutexes = MUTEXES.read();
    if let Some(mutex) = mutexes.get(&handle) {
        let _guard = mutex.inner.lock();
        THREAD_LOCKS.with(|locks| {
            locks.borrow_mut().insert(handle);
        });
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_sync_unlock(handle: u64) {
    THREAD_LOCKS.with(|locks| {
        locks.borrow_mut().remove(&handle);
    });
}

// ============================================================================
// WaitGroup Operations
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn otter_sync_waitgroup() -> u64 {
    let id = next_handle_id();
    let wg = WaitGroup {
        _id: id,
        count: Arc::new(AtomicUsize::new(0)),
    };

    WAIT_GROUPS.write().insert(id, wg);
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_sync_waitgroup_add(handle: u64, delta: i64) {
    let wait_groups = WAIT_GROUPS.read();
    if let Some(wg) = wait_groups.get(&handle) {
        if delta > 0 {
            wg.count.fetch_add(delta as usize, Ordering::SeqCst);
        } else if delta < 0 {
            wg.count.fetch_sub((-delta) as usize, Ordering::SeqCst);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_sync_waitgroup_done(handle: u64) {
    let wait_groups = WAIT_GROUPS.read();
    if let Some(wg) = wait_groups.get(&handle) {
        let prev = wg.count.fetch_sub(1, Ordering::SeqCst);
        if prev == 1 {
            // No-op for now; future versions may notify waiters explicitly.
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_sync_waitgroup_wait(handle: u64) {
    let wait_groups = WAIT_GROUPS.read();
    if let Some(wg) = wait_groups.get(&handle) {
        while wg.count.load(Ordering::SeqCst) > 0 {
            thread::yield_now();
        }
    }
}

// ============================================================================
// Atomic Operations
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn otter_sync_atomic_int(initial: i64) -> u64 {
    let id = next_handle_id();
    let atomic = AtomicInt {
        _id: id,
        value: Arc::new(AtomicU64::new(initial as u64)),
    };

    ATOMICS.write().insert(id, atomic);
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_sync_atomic_add(handle: u64, delta: i64) -> i64 {
    let atomics = ATOMICS.read();
    if let Some(atomic) = atomics.get(&handle) {
        atomic.value.fetch_add(delta as u64, Ordering::SeqCst) as i64
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_sync_atomic_get(handle: u64) -> i64 {
    let atomics = ATOMICS.read();
    if let Some(atomic) = atomics.get(&handle) {
        atomic.value.load(Ordering::SeqCst) as i64
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_sync_atomic_set(handle: u64, value: i64) {
    let atomics = ATOMICS.read();
    if let Some(atomic) = atomics.get(&handle) {
        atomic.value.store(value as u64, Ordering::SeqCst);
    }
}

// ============================================================================
// Once Operations
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn otter_sync_once() -> u64 {
    let id = next_handle_id();
    let once_handle = OnceHandle {
        _id: id,
        inner: Arc::new(Once::new()),
    };

    ONCE_HANDLES.write().insert(id, once_handle);
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn otter_sync_once_call(handle: u64, callback: extern "C" fn()) {
    let once_handles = ONCE_HANDLES.read();
    if let Some(once_handle) = once_handles.get(&handle) {
        once_handle.inner.call_once(|| {
            callback();
        });
    }
}

// ============================================================================
// Symbol Registration
// ============================================================================

fn register_std_sync_symbols(registry: &SymbolRegistry) {
    registry.register(FfiFunction {
        name: "sync.mutex".into(),
        symbol: "otter_sync_mutex".into(),
        signature: FfiSignature::new(vec![], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "sync.lock".into(),
        symbol: "otter_sync_lock".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "sync.unlock".into(),
        symbol: "otter_sync_unlock".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "sync.waitgroup".into(),
        symbol: "otter_sync_waitgroup".into(),
        signature: FfiSignature::new(vec![], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "sync.waitgroup_add".into(),
        symbol: "otter_sync_waitgroup_add".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque, FfiType::I64], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "sync.waitgroup_done".into(),
        symbol: "otter_sync_waitgroup_done".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "sync.waitgroup_wait".into(),
        symbol: "otter_sync_waitgroup_wait".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "sync.atomic_int".into(),
        symbol: "otter_sync_atomic_int".into(),
        signature: FfiSignature::new(vec![FfiType::I64], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "sync.atomic_add".into(),
        symbol: "otter_sync_atomic_add".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque, FfiType::I64], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "sync.atomic_get".into(),
        symbol: "otter_sync_atomic_get".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "sync.atomic_set".into(),
        symbol: "otter_sync_atomic_set".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque, FfiType::I64], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "sync.once".into(),
        symbol: "otter_sync_once".into(),
        signature: FfiSignature::new(vec![], FfiType::Opaque),
    });

    registry.register(FfiFunction {
        name: "sync.once_call".into(),
        symbol: "otter_sync_once_call".into(),
        signature: FfiSignature::new(vec![FfiType::Opaque, FfiType::Opaque], FfiType::Unit),
    });
}

inventory::submit! {
    otterc_ffi::SymbolProvider {
        namespace: "sync",
        autoload: false,
        register: register_std_sync_symbols,
    }
}
