use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "task-runtime")]
use std::sync::Arc;

use once_cell::sync::Lazy;
use parking_lot::RwLock;
use sysinfo::System;

use crate::runtime::memory::config::GcStrategy;
use crate::runtime::memory::gc::get_gc;
use crate::runtime::memory::profiler::get_profiler;
use crate::runtime::symbol_registry::{FfiFunction, FfiSignature, FfiType, SymbolRegistry};
use crate::version::VERSION;

#[cfg(feature = "task-runtime")]
use crate::runtime::task::{TaskMetricsSnapshot, TaskRuntimeMetrics, WorkerState};

// ============================================================================
// Runtime Statistics Tracking
// ============================================================================

static ACTIVE_GOROUTINES: AtomicU64 = AtomicU64::new(0);

// Track runtime statistics
#[derive(Default)]
struct RuntimeStats {
    #[expect(dead_code, reason = "Work in progress")]
    total_tasks: u64,
    #[expect(dead_code, reason = "Work in progress")]
    completed_tasks: u64,
    #[expect(dead_code, reason = "Work in progress")]
    active_threads: usize,
    heap_bytes: usize,
    cpu_count: usize,
}

static RUNTIME_STATS: Lazy<RwLock<RuntimeStats>> =
    Lazy::new(|| RwLock::new(RuntimeStats::default()));

#[cfg(feature = "task-runtime")]
static TASK_METRICS: Lazy<RwLock<Option<Arc<TaskRuntimeMetrics>>>> =
    Lazy::new(|| RwLock::new(None));

// ============================================================================
// Runtime Functions
// ============================================================================

/// Get the number of active tasks/threads
#[unsafe(no_mangle)]
pub extern "C" fn otter_runtime_gos() -> i64 {
    ACTIVE_GOROUTINES.load(Ordering::SeqCst) as i64
}

/// Get the number of CPU cores
#[unsafe(no_mangle)]
pub extern "C" fn otter_runtime_cpu_count() -> i64 {
    let mut system = System::new_all();
    system.refresh_cpu();
    let count = system.cpus().len();

    // Update stats
    RUNTIME_STATS.write().cpu_count = count;

    count as i64
}

/// Get current heap memory usage in bytes
/// Note: In Rust, we don't have direct heap access, so we approximate
/// using process memory from sysinfo
#[unsafe(no_mangle)]
pub extern "C" fn otter_runtime_memory() -> i64 {
    let mut system = System::new_all();
    system.refresh_memory();

    // Get current process memory (approximation of heap)
    let process_id = std::process::id();
    if let Some(process) = system.process(sysinfo::Pid::from(process_id as usize)) {
        let memory_bytes = process.memory() * 1024; // sysinfo returns KB
        RUNTIME_STATS.write().heap_bytes = memory_bytes as usize;
        memory_bytes as i64
    } else {
        // Fallback: use system memory info
        let used_memory = system.used_memory() * 1024;
        RUNTIME_STATS.write().heap_bytes = used_memory as usize;
        used_memory as i64
    }
}

/// Trigger garbage collection
/// Returns the number of bytes freed
#[unsafe(no_mangle)]
pub extern "C" fn otter_runtime_collect_garbage() -> i64 {
    let gc = get_gc();
    let stats = gc.collect();
    stats.bytes_freed as i64
}

/// Start memory profiling
#[unsafe(no_mangle)]
pub extern "C" fn otter_runtime_memory_profiler_start() {
    let profiler = get_profiler();
    profiler.start();
}

/// Stop memory profiling
#[unsafe(no_mangle)]
pub extern "C" fn otter_runtime_memory_profiler_stop() {
    let profiler = get_profiler();
    profiler.stop();
}

/// Get memory profiling statistics as JSON
#[unsafe(no_mangle)]
pub extern "C" fn otter_runtime_memory_profiler_stats() -> *mut c_char {
    let profiler = get_profiler();
    let stats = profiler.get_stats();

    // Convert to JSON using serde_json if available, otherwise manual formatting
    let json = serde_json::to_string(&stats).unwrap_or_else(|_| {
        format!(
            r#"{{"enabled":{},"total_allocated":{},"total_freed":{},"current_memory":{},"peak_memory":{},"active_allocations":{},"duration_seconds":{}}}"#,
            stats.enabled,
            stats.total_allocated,
            stats.total_freed,
            stats.current_memory,
            stats.peak_memory,
            stats.active_allocations,
            stats.duration_seconds
        )
    });

    CString::new(json)
        .ok()
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

/// Detect memory leaks and return as JSON
#[unsafe(no_mangle)]
pub extern "C" fn otter_runtime_memory_profiler_leaks() -> *mut c_char {
    let profiler = get_profiler();
    let leaks = profiler.detect_leaks();

    let json = serde_json::to_string(&leaks)
        .unwrap_or_else(|_| format!(r#"{{"leak_count":{}}}"#, leaks.len()));

    CString::new(json)
        .ok()
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

/// Set garbage collection strategy
/// strategy: "rc", "mark-sweep", "hybrid", or "none"
///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_runtime_set_gc_strategy(strategy: *const c_char) -> i32 {
    if strategy.is_null() {
        return 0;
    }

    unsafe {
        match CStr::from_ptr(strategy).to_str() {
            Ok(s) => match s.parse::<GcStrategy>() {
                Ok(gc_strategy) => {
                    let gc = get_gc();
                    gc.set_strategy(gc_strategy);
                    1
                }
                Err(_) => 0,
            },
            Err(_) => 0,
        }
    }
}

/// Get runtime statistics as a JSON string
/// Returns a JSON object with various runtime metrics
#[unsafe(no_mangle)]
pub extern "C" fn otter_runtime_stats() -> *mut c_char {
    let stats = RUNTIME_STATS.read();

    // Build JSON string manually (avoiding serde_json dependency for simplicity)
    let mut system = System::new_all();
    system.refresh_cpu();
    system.refresh_memory();

    let cpu_count = system.cpus().len();
    let active_gos = ACTIVE_GOROUTINES.load(Ordering::SeqCst);
    let memory_bytes = stats.heap_bytes;

    let base_fields = || {
        [
            format!("\"gos\":{}", active_gos),
            format!("\"cpu_count\":{}", cpu_count),
            format!("\"memory_bytes\":{}", memory_bytes),
            format!("\"total_memory\":{}", system.total_memory() * 1024),
            format!("\"available_memory\":{}", system.available_memory() * 1024),
        ]
    };

    #[cfg(feature = "task-runtime")]
    let mut fields: Vec<String> = base_fields().into_iter().collect();

    #[cfg(not(feature = "task-runtime"))]
    let fields = base_fields();

    #[cfg(feature = "task-runtime")]
    if let Some(task_field) = task_metrics_field() {
        fields.push(task_field);
    }

    let json = format!("{{{}}}", fields.join(","));

    CString::new(json)
        .ok()
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

/// Get detailed task runtime state as JSON
/// Returns comprehensive information about tasks, workers, and queues
#[cfg(feature = "task-runtime")]
#[unsafe(no_mangle)]
pub extern "C" fn otter_runtime_tasks() -> *mut c_char {
    if let Some(metrics) = task_metrics_clone() {
        let snapshot = metrics.snapshot();

        // Build worker info array
        let worker_json: Vec<String> = snapshot
            .worker_infos
            .iter()
            .map(|w| {
                let state_str = match w.state {
                    WorkerState::Idle => "idle",
                    WorkerState::Busy => "busy",
                    WorkerState::Parked => "parked",
                };
                format!(
                    "{{\"id\":{},\"state\":\"{}\",\"queue_depth\":{},\"tasks_processed\":{}}}",
                    w.id, state_str, w.queue_depth, w.tasks_processed
                )
            })
            .collect();

        let json = format!(
            "{{\"tasks\":{{\"spawned\":{},\"completed\":{},\"waiting\":{}}},\"channels\":{{\"registered\":{},\"waiting\":{},\"backlog\":{}}},\"workers\":{{\"total\":{},\"active\":{}}},\"worker_details\":[{}]}}",
            snapshot.tasks_spawned,
            snapshot.tasks_completed,
            snapshot.tasks_waiting,
            snapshot.channels_registered,
            snapshot.channel_waiters,
            snapshot.channel_backlog,
            snapshot.total_workers,
            snapshot.active_workers,
            worker_json.join(",")
        );

        CString::new(json)
            .ok()
            .map(CString::into_raw)
            .unwrap_or(std::ptr::null_mut())
    } else {
        // Return empty JSON if task runtime not available
        CString::new("{}")
            .ok()
            .map(CString::into_raw)
            .unwrap_or(std::ptr::null_mut())
    }
}

/// Get OtterLang runtime version
#[unsafe(no_mangle)]
pub extern "C" fn otter_runtime_version() -> *mut c_char {
    CString::new(VERSION)
        .ok()
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

/// Free a string returned by runtime functions
/// ///
/// # Safety
///
/// this function dereferences a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_runtime_free_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(ptr));
    }
}

// ============================================================================
// Helper Functions for Sync Module Integration
// ============================================================================

/// Increment active task count
pub fn increment_active_tasks() {
    ACTIVE_GOROUTINES.fetch_add(1, Ordering::SeqCst);
}

/// Decrement active task count
pub fn decrement_active_tasks() {
    ACTIVE_GOROUTINES.fetch_sub(1, Ordering::SeqCst);
}

#[cfg(feature = "task-runtime")]
pub fn register_task_metrics(metrics: Arc<TaskRuntimeMetrics>) {
    *TASK_METRICS.write() = Some(metrics);
}

#[cfg(feature = "task-runtime")]
pub fn task_metrics_clone() -> Option<Arc<TaskRuntimeMetrics>> {
    TASK_METRICS.read().as_ref().map(Arc::clone)
}

#[cfg(feature = "task-runtime")]
pub fn emit_task_metrics_report() {
    use std::env;

    if env::var_os("OTTER_TASKS_DIAGNOSTICS").is_none() {
        return;
    }

    if let Some(metrics) = task_metrics_clone() {
        let snapshot = metrics.snapshot();
        println!(
            "[tasks] spawned={}, completed={}, waiting={}, channels={}, channel_waiters={}, channel_backlog={}",
            snapshot.tasks_spawned,
            snapshot.tasks_completed,
            snapshot.tasks_waiting,
            snapshot.channels_registered,
            snapshot.channel_waiters,
            snapshot.channel_backlog
        );
    }
}

#[cfg(feature = "task-runtime")]
fn task_metrics_field() -> Option<String> {
    let guard = TASK_METRICS.read();
    guard.as_ref().map(|metrics| {
        let snapshot: TaskMetricsSnapshot = metrics.snapshot();
        format!(
            "\"tasks\":{{\"spawned\":{},\"completed\":{},\"waiting\":{}}},\"channels\":{{\"registered\":{},\"waiting\":{},\"backlog\":{}}},\"workers\":{{\"total\":{},\"active\":{}}}",
            snapshot.tasks_spawned,
            snapshot.tasks_completed,
            snapshot.tasks_waiting,
            snapshot.channels_registered,
            snapshot.channel_waiters,
            snapshot.channel_backlog,
            snapshot.total_workers,
            snapshot.active_workers
        )
    })
}

// ============================================================================
// Symbol Registration
// ============================================================================

fn register_std_runtime_symbols(registry: &SymbolRegistry) {
    registry.register(FfiFunction {
        name: "runtime.gos".into(),
        symbol: "otter_runtime_gos".into(),
        signature: FfiSignature::new(vec![], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "runtime.cpu_count".into(),
        symbol: "otter_runtime_cpu_count".into(),
        signature: FfiSignature::new(vec![], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "runtime.memory".into(),
        symbol: "otter_runtime_memory".into(),
        signature: FfiSignature::new(vec![], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "runtime.collect_garbage".into(),
        symbol: "otter_runtime_collect_garbage".into(),
        signature: FfiSignature::new(vec![], FfiType::I64),
    });

    registry.register(FfiFunction {
        name: "runtime.memory_profiler_start".into(),
        symbol: "otter_runtime_memory_profiler_start".into(),
        signature: FfiSignature::new(vec![], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "runtime.memory_profiler_stop".into(),
        symbol: "otter_runtime_memory_profiler_stop".into(),
        signature: FfiSignature::new(vec![], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "runtime.memory_profiler_stats".into(),
        symbol: "otter_runtime_memory_profiler_stats".into(),
        signature: FfiSignature::new(vec![], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "runtime.memory_profiler_leaks".into(),
        symbol: "otter_runtime_memory_profiler_leaks".into(),
        signature: FfiSignature::new(vec![], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "runtime.set_gc_strategy".into(),
        symbol: "otter_runtime_set_gc_strategy".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::I32),
    });

    registry.register(FfiFunction {
        name: "runtime.stats".into(),
        symbol: "otter_runtime_stats".into(),
        signature: FfiSignature::new(vec![], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "runtime.version".into(),
        symbol: "otter_runtime_version".into(),
        signature: FfiSignature::new(vec![], FfiType::Str),
    });

    registry.register(FfiFunction {
        name: "runtime.free".into(),
        symbol: "otter_runtime_free_string".into(),
        signature: FfiSignature::new(vec![FfiType::Str], FfiType::Unit),
    });

    #[cfg(feature = "task-runtime")]
    registry.register(FfiFunction {
        name: "runtime.tasks".into(),
        symbol: "otter_runtime_tasks".into(),
        signature: FfiSignature::new(vec![], FfiType::Str),
    });

    // Error handling runtime functions
    registry.register(FfiFunction {
        name: "runtime.error_push_context".into(),
        symbol: "otter_error_push_context".into(),
        signature: FfiSignature::new(vec![], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "runtime.error_pop_context".into(),
        symbol: "otter_error_pop_context".into(),
        signature: FfiSignature::new(vec![], FfiType::Bool),
    });

    registry.register(FfiFunction {
        name: "runtime.error_has_error".into(),
        symbol: "otter_error_has_error".into(),
        signature: FfiSignature::new(vec![], FfiType::Bool),
    });

    registry.register(FfiFunction {
        name: "runtime.error_clear".into(),
        symbol: "otter_error_clear".into(),
        signature: FfiSignature::new(vec![], FfiType::Unit),
    });

    registry.register(FfiFunction {
        name: "runtime.error_rethrow".into(),
        symbol: "otter_error_rethrow".into(),
        signature: FfiSignature::new(vec![], FfiType::Unit),
    });
}

inventory::submit! {
    crate::runtime::ffi::SymbolProvider {
        namespace: "runtime",
        autoload: false,
        register: register_std_runtime_symbols,
    }
}
