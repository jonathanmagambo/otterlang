//! Memory profiling for tracking allocations and memory usage patterns

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

/// Memory allocation event
#[derive(Debug, Clone)]
pub struct AllocationEvent {
    /// Pointer address
    pub ptr: usize,

    /// Size in bytes
    pub size: usize,

    /// Function that allocated this memory
    pub function_name: String,

    /// Timestamp (milliseconds since epoch)
    pub timestamp_ms: u64,

    /// Stack trace (optional, for debugging)
    pub stack_trace: Option<Vec<String>>,
}

/// Memory deallocation event
#[derive(Debug, Clone)]
pub struct DeallocationEvent {
    /// Pointer address
    pub ptr: usize,

    /// Timestamp (milliseconds since epoch)
    pub timestamp_ms: u64,
}

/// Memory usage statistics for a function
#[derive(Debug, Clone, Default)]
pub struct FunctionMemoryStats {
    /// Function name
    pub function_name: String,

    /// Total bytes allocated
    pub total_allocated: usize,

    /// Total bytes deallocated
    pub total_deallocated: usize,

    /// Current bytes in use
    pub current_usage: usize,

    /// Peak memory usage
    pub peak_usage: usize,

    /// Number of allocations
    pub allocation_count: u64,

    /// Number of deallocations
    pub deallocation_count: u64,

    /// Allocation size histogram (size range -> count)
    pub size_histogram: HashMap<usize, u64>,
}

impl FunctionMemoryStats {
    pub fn new(function_name: String) -> Self {
        Self {
            function_name,
            ..Default::default()
        }
    }

    /// Record an allocation
    pub fn record_allocation(&mut self, size: usize) {
        self.total_allocated += size;
        self.current_usage += size;
        self.allocation_count += 1;

        if self.current_usage > self.peak_usage {
            self.peak_usage = self.current_usage;
        }

        // Update histogram (bucket by power of 2)
        let bucket = size.next_power_of_two();
        *self.size_histogram.entry(bucket).or_insert(0) += 1;
    }

    /// Record a deallocation
    pub fn record_deallocation(&mut self, size: usize) {
        self.total_deallocated += size;
        self.current_usage = self.current_usage.saturating_sub(size);
        self.deallocation_count += 1;
    }

    /// Get net memory usage (allocated - deallocated)
    pub fn net_usage(&self) -> isize {
        self.total_allocated as isize - self.total_deallocated as isize
    }

    /// Check if there might be a memory leak
    pub fn potential_leak(&self) -> bool {
        // If we've allocated significantly more than deallocated
        // and have many outstanding allocations
        let leak_threshold = 0.8; // 80% of allocations not freed
        let outstanding = self
            .allocation_count
            .saturating_sub(self.deallocation_count);

        outstanding as f64 / self.allocation_count.max(1) as f64 > leak_threshold
            && self.current_usage > 1024 * 1024 // At least 1MB outstanding
    }

    /// Get average allocation size
    pub fn avg_allocation_size(&self) -> f64 {
        if self.allocation_count == 0 {
            0.0
        } else {
            self.total_allocated as f64 / self.allocation_count as f64
        }
    }
}

/// Global memory profiler
pub struct MemoryProfiler {
    /// Per-function memory statistics
    function_stats: Arc<RwLock<HashMap<String, FunctionMemoryStats>>>,

    /// Active allocations (ptr -> (size, function_name))
    active_allocations: Arc<RwLock<HashMap<usize, (usize, String)>>>,

    /// Enable stack trace collection (expensive)
    collect_stack_traces: bool,

    /// Allocation history (for debugging)
    allocation_history: Arc<RwLock<Vec<AllocationEvent>>>,

    /// Maximum history size
    max_history_size: usize,
}

impl MemoryProfiler {
    pub fn new() -> Self {
        Self {
            function_stats: Arc::new(RwLock::new(HashMap::new())),
            active_allocations: Arc::new(RwLock::new(HashMap::new())),
            collect_stack_traces: false,
            allocation_history: Arc::new(RwLock::new(Vec::new())),
            max_history_size: 10000,
        }
    }

    /// Enable or disable stack trace collection
    pub fn set_collect_stack_traces(&mut self, enabled: bool) {
        self.collect_stack_traces = enabled;
    }

    /// Record an allocation
    pub fn record_allocation(&self, ptr: usize, size: usize, function_name: &str) {
        // Update function stats
        let mut stats = self.function_stats.write();
        let func_stats = stats
            .entry(function_name.to_string())
            .or_insert_with(|| FunctionMemoryStats::new(function_name.to_string()));
        func_stats.record_allocation(size);
        drop(stats);

        // Track active allocation
        self.active_allocations
            .write()
            .insert(ptr, (size, function_name.to_string()));

        // Record in history
        let mut history = self.allocation_history.write();
        if history.len() >= self.max_history_size {
            history.remove(0);
        }

        history.push(AllocationEvent {
            ptr,
            size,
            function_name: function_name.to_string(),
            timestamp_ms: current_time_ms(),
            stack_trace: if self.collect_stack_traces {
                Some(collect_stack_trace())
            } else {
                None
            },
        });
    }

    /// Record a deallocation
    pub fn record_deallocation(&self, ptr: usize) {
        if let Some((size, function_name)) = self.active_allocations.write().remove(&ptr) {
            let mut stats = self.function_stats.write();
            if let Some(func_stats) = stats.get_mut(&function_name) {
                func_stats.record_deallocation(size);
            }
        }
    }

    /// Get statistics for a function
    pub fn get_function_stats(&self, function_name: &str) -> Option<FunctionMemoryStats> {
        self.function_stats.read().get(function_name).cloned()
    }

    /// Get all function statistics
    pub fn get_all_stats(&self) -> Vec<FunctionMemoryStats> {
        self.function_stats.read().values().cloned().collect()
    }

    /// Get total memory usage across all functions
    pub fn total_memory_usage(&self) -> usize {
        self.function_stats
            .read()
            .values()
            .map(|s| s.current_usage)
            .sum()
    }

    /// Get functions with potential memory leaks
    pub fn get_potential_leaks(&self) -> Vec<FunctionMemoryStats> {
        self.function_stats
            .read()
            .values()
            .filter(|s| s.potential_leak())
            .cloned()
            .collect()
    }

    /// Get allocation history
    pub fn get_allocation_history(&self) -> Vec<AllocationEvent> {
        self.allocation_history.read().clone()
    }

    /// Clear all statistics
    pub fn clear(&self) {
        self.function_stats.write().clear();
        self.active_allocations.write().clear();
        self.allocation_history.write().clear();
    }

    /// Get memory fragmentation estimate
    pub fn fragmentation_estimate(&self) -> f64 {
        let allocations = self.active_allocations.read();
        if allocations.is_empty() {
            return 0.0;
        }

        // Calculate average gap between allocations
        let mut ptrs: Vec<usize> = allocations.keys().copied().collect();
        ptrs.sort_unstable();

        if ptrs.len() < 2 {
            return 0.0;
        }

        let gaps: Vec<usize> = ptrs.windows(2).map(|w| w[1] - w[0]).collect();
        let avg_gap = gaps.iter().sum::<usize>() as f64 / gaps.len() as f64;
        let avg_size: f64 =
            allocations.values().map(|(s, _)| *s).sum::<usize>() as f64 / allocations.len() as f64;

        // Fragmentation ratio: gap / size
        if avg_size > 0.0 {
            (avg_gap / avg_size).min(1.0)
        } else {
            0.0
        }
    }
}

impl Default for MemoryProfiler {
    fn default() -> Self {
        Self::new()
    }
}

/// Get current time in milliseconds
fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Collect stack trace (placeholder - would need backtrace crate)
fn collect_stack_trace() -> Vec<String> {
    // In a real implementation, use the backtrace crate
    // For now, return empty
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocation_tracking() {
        let profiler = MemoryProfiler::new();

        profiler.record_allocation(0x1000, 1024, "test_fn");
        profiler.record_allocation(0x2000, 2048, "test_fn");

        let stats = profiler.get_function_stats("test_fn").unwrap();
        assert_eq!(stats.total_allocated, 3072);
        assert_eq!(stats.allocation_count, 2);
        assert_eq!(stats.current_usage, 3072);
    }

    #[test]
    fn test_deallocation_tracking() {
        let profiler = MemoryProfiler::new();

        profiler.record_allocation(0x1000, 1024, "test_fn");
        profiler.record_deallocation(0x1000);

        let stats = profiler.get_function_stats("test_fn").unwrap();
        assert_eq!(stats.total_allocated, 1024);
        assert_eq!(stats.total_deallocated, 1024);
        assert_eq!(stats.current_usage, 0);
    }

    #[test]
    fn test_leak_detection() {
        let profiler = MemoryProfiler::new();

        // Allocate a lot without freeing
        for i in 0..1000 {
            profiler.record_allocation(0x1000 + i * 1024, 2048, "leaky_fn");
        }

        let stats = profiler.get_function_stats("leaky_fn").unwrap();
        assert!(stats.potential_leak());
    }

    #[test]
    fn test_total_memory_usage() {
        let profiler = MemoryProfiler::new();

        profiler.record_allocation(0x1000, 1024, "fn1");
        profiler.record_allocation(0x2000, 2048, "fn2");

        assert_eq!(profiler.total_memory_usage(), 3072);
    }
}
