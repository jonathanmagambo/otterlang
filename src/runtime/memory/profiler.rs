//! Memory profiling and allocation tracking

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// Information about a single allocation
#[derive(Debug, Clone)]
pub struct AllocationInfo {
    /// Size of the allocation in bytes
    pub size: usize,
    /// Function name where allocation occurred
    pub function: Option<String>,
    /// File name where allocation occurred
    pub file: Option<String>,
    /// Line number where allocation occurred
    pub line: Option<u32>,
    /// Timestamp when allocation occurred
    pub timestamp: Instant,
    /// Object type/class name
    pub object_type: Option<String>,
}

impl serde::Serialize for AllocationInfo {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("AllocationInfo", 6)?;
        state.serialize_field("size", &self.size)?;
        state.serialize_field("function", &self.function)?;
        state.serialize_field("file", &self.file)?;
        state.serialize_field("line", &self.line)?;
        state.serialize_field("timestamp_secs", &self.timestamp.elapsed().as_secs_f64())?;
        state.serialize_field("object_type", &self.object_type)?;
        state.end()
    }
}

/// Memory profiler that tracks allocations and deallocations
pub struct MemoryProfiler {
    enabled: Arc<AtomicBool>,
    allocations: Arc<RwLock<HashMap<usize, AllocationInfo>>>,
    total_allocated: Arc<AtomicUsize>,
    total_freed: Arc<AtomicUsize>,
    peak_memory: Arc<AtomicUsize>,
    current_memory: Arc<AtomicUsize>,
    start_time: Arc<RwLock<Option<Instant>>>,
}

impl MemoryProfiler {
    pub fn new() -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            allocations: Arc::new(RwLock::new(HashMap::new())),
            total_allocated: Arc::new(AtomicUsize::new(0)),
            total_freed: Arc::new(AtomicUsize::new(0)),
            peak_memory: Arc::new(AtomicUsize::new(0)),
            current_memory: Arc::new(AtomicUsize::new(0)),
            start_time: Arc::new(RwLock::new(None)),
        }
    }

    /// Start profiling
    pub fn start(&self) {
        self.enabled.store(true, Ordering::SeqCst);
        *self.start_time.write() = Some(Instant::now());
        self.allocations.write().clear();
        self.total_allocated.store(0, Ordering::SeqCst);
        self.total_freed.store(0, Ordering::SeqCst);
        self.current_memory.store(0, Ordering::SeqCst);
        self.peak_memory.store(0, Ordering::SeqCst);
    }

    /// Stop profiling
    pub fn stop(&self) {
        self.enabled.store(false, Ordering::SeqCst);
    }

    /// Check if profiling is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Record an allocation
    pub fn record_allocation(
        &self,
        ptr: usize,
        size: usize,
        function: Option<String>,
        file: Option<String>,
        line: Option<u32>,
        object_type: Option<String>,
    ) {
        if !self.is_enabled() {
            return;
        }

        let info = AllocationInfo {
            size,
            function,
            file,
            line,
            timestamp: Instant::now(),
            object_type,
        };

        self.allocations.write().insert(ptr, info.clone());
        self.total_allocated.fetch_add(size, Ordering::SeqCst);
        let current = self.current_memory.fetch_add(size, Ordering::SeqCst) + size;

        // Update peak memory
        loop {
            let peak = self.peak_memory.load(Ordering::SeqCst);
            if current <= peak {
                break;
            }
            if self
                .peak_memory
                .compare_exchange_weak(peak, current, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                break;
            }
        }
    }

    /// Record a deallocation
    pub fn record_deallocation(&self, ptr: usize) {
        if !self.is_enabled() {
            return;
        }

        if let Some(info) = self.allocations.write().remove(&ptr) {
            self.total_freed.fetch_add(info.size, Ordering::SeqCst);
            self.current_memory.fetch_sub(info.size, Ordering::SeqCst);
        }
    }

    /// Get profiling statistics
    pub fn get_stats(&self) -> ProfilingStats {
        let allocations = self.allocations.read();
        let start_time = self.start_time.read();

        let mut size_histogram: HashMap<usize, usize> = HashMap::new();
        let mut function_allocations: HashMap<String, (usize, usize)> = HashMap::new();

        for info in allocations.values() {
            // Size histogram
            *size_histogram.entry(info.size).or_insert(0) += 1;

            // Function allocations
            if let Some(ref func) = info.function {
                let entry = function_allocations.entry(func.clone()).or_insert((0, 0));
                entry.0 += info.size;
                entry.1 += 1;
            }
        }

        let duration = start_time.as_ref().map(|t| t.elapsed()).unwrap_or_default();

        ProfilingStats {
            enabled: self.is_enabled(),
            total_allocated: self.total_allocated.load(Ordering::SeqCst),
            total_freed: self.total_freed.load(Ordering::SeqCst),
            current_memory: self.current_memory.load(Ordering::SeqCst),
            peak_memory: self.peak_memory.load(Ordering::SeqCst),
            active_allocations: allocations.len(),
            duration_seconds: duration.as_secs_f64(),
            size_histogram,
            top_allocators: {
                let mut v: Vec<_> = function_allocations.into_iter().collect();
                v.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
                v.into_iter().take(10).collect()
            },
        }
    }

    /// Detect memory leaks (allocations without matching deallocations)
    pub fn detect_leaks(&self) -> Vec<LeakInfo> {
        let allocations = self.allocations.read();
        let mut leaks = Vec::new();

        for (ptr, info) in allocations.iter() {
            leaks.push(LeakInfo {
                pointer: *ptr,
                size: info.size,
                function: info.function.clone(),
                file: info.file.clone(),
                line: info.line,
                object_type: info.object_type.clone(),
                age_seconds: info.timestamp.elapsed().as_secs_f64(),
            });
        }

        leaks.sort_by(|a, b| b.size.cmp(&a.size));
        leaks
    }
}

impl Default for MemoryProfiler {
    fn default() -> Self {
        Self::new()
    }
}

/// Profiling statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilingStats {
    pub enabled: bool,
    pub total_allocated: usize,
    pub total_freed: usize,
    pub current_memory: usize,
    pub peak_memory: usize,
    pub active_allocations: usize,
    pub duration_seconds: f64,
    pub size_histogram: HashMap<usize, usize>,
    pub top_allocators: Vec<(String, (usize, usize))>, // (function_name, (total_bytes, count))
}

/// Information about a memory leak
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeakInfo {
    pub pointer: usize,
    pub size: usize,
    pub function: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub object_type: Option<String>,
    pub age_seconds: f64,
}

/// Global memory profiler instance
static GLOBAL_PROFILER: once_cell::sync::Lazy<MemoryProfiler> =
    once_cell::sync::Lazy::new(MemoryProfiler::new);

/// Get the global memory profiler
pub fn get_profiler() -> &'static MemoryProfiler {
    &GLOBAL_PROFILER
}
