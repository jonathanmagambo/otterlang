//! Garbage collection implementations

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use parking_lot::RwLock;

use crate::runtime::memory::config::GcStrategy;
use crate::runtime::memory::profiler::get_profiler;

/// Trait for garbage collection strategies
pub trait GcStrategyTrait: Send + Sync {
    /// Run garbage collection
    fn collect(&self) -> GcStats;

    /// Get the strategy name
    fn name(&self) -> &'static str;
}

/// Statistics from a garbage collection run
#[derive(Debug, Clone, Default)]
pub struct GcStats {
    /// Number of objects collected
    pub objects_collected: usize,
    /// Bytes freed
    pub bytes_freed: usize,
    /// Duration of GC in milliseconds
    pub duration_ms: u64,
}

/// Reference counting garbage collector
pub struct RcGC {
    // Reference counting is handled automatically by RcOtter
    // This GC just provides statistics
}

impl RcGC {
    pub fn new() -> Self {
        Self {}
    }
}

impl GcStrategyTrait for RcGC {
    fn collect(&self) -> GcStats {
        // Reference counting handles cleanup automatically
        // This is mainly for statistics
        GcStats {
            objects_collected: 0,
            bytes_freed: 0,
            duration_ms: 0,
        }
    }

    fn name(&self) -> &'static str {
        "ReferenceCounting"
    }
}

impl Default for RcGC {
    fn default() -> Self {
        Self::new()
    }
}

/// Mark-and-sweep garbage collector
pub struct MarkSweepGC {
    roots: Arc<RwLock<HashSet<usize>>>, // Root object pointers
    objects: Arc<RwLock<HashMap<usize, ObjectInfo>>>,
}

#[derive(Debug, Clone)]
struct ObjectInfo {
    size: usize,
    references: Vec<usize>, // Pointers to other objects
}

impl MarkSweepGC {
    pub fn new() -> Self {
        Self {
            roots: Arc::new(RwLock::new(HashSet::new())),
            objects: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a root object (object that should not be collected)
    pub fn add_root(&self, ptr: usize) {
        self.roots.write().insert(ptr);
    }

    /// Remove a root object
    pub fn remove_root(&self, ptr: usize) {
        self.roots.write().remove(&ptr);
    }

    /// Register an object for GC tracking
    pub fn register_object(&self, ptr: usize, size: usize, references: Vec<usize>) {
        self.objects
            .write()
            .insert(ptr, ObjectInfo { size, references });
    }

    /// Unregister an object
    pub fn unregister_object(&self, ptr: usize) {
        self.objects.write().remove(&ptr);
    }

    /// Mark phase: mark all reachable objects
    fn mark(&self) -> HashSet<usize> {
        let mut marked = HashSet::new();
        let roots = self.roots.read().clone();
        let objects = self.objects.read().clone();

        let mut stack: Vec<usize> = roots.iter().copied().collect();

        while let Some(ptr) = stack.pop() {
            if marked.contains(&ptr) {
                continue;
            }

            marked.insert(ptr);

            if let Some(info) = objects.get(&ptr) {
                for &ref_ptr in &info.references {
                    if !marked.contains(&ref_ptr) {
                        stack.push(ref_ptr);
                    }
                }
            }
        }

        marked
    }

    /// Sweep phase: collect unmarked objects
    fn sweep(&self, marked: &HashSet<usize>) -> GcStats {
        let mut objects = self.objects.write();
        let mut objects_collected = 0;
        let mut bytes_freed = 0;

        let unmarked: Vec<usize> = objects
            .keys()
            .filter(|ptr| !marked.contains(ptr))
            .copied()
            .collect();

        for ptr in unmarked {
            if let Some(info) = objects.remove(&ptr) {
                objects_collected += 1;
                bytes_freed += info.size;

                // Record deallocation in profiler
                get_profiler().record_deallocation(ptr);
            }
        }

        GcStats {
            objects_collected,
            bytes_freed,
            duration_ms: 0, // Will be set by caller
        }
    }
}

impl GcStrategyTrait for MarkSweepGC {
    fn collect(&self) -> GcStats {
        let start = std::time::Instant::now();

        let marked = self.mark();
        let mut stats = self.sweep(&marked);

        stats.duration_ms = start.elapsed().as_millis() as u64;

        stats
    }

    fn name(&self) -> &'static str {
        "MarkSweep"
    }
}

impl Default for MarkSweepGC {
    fn default() -> Self {
        Self::new()
    }
}

/// Hybrid GC: reference counting + periodic mark-sweep
pub struct HybridGC {
    _rc_gc: RcGC,
    mark_sweep_gc: MarkSweepGC,
    cycle_detection_interval: usize,
    allocations_since_cycle_check: Arc<AtomicUsize>,
}

impl HybridGC {
    pub fn new() -> Self {
        Self {
            _rc_gc: RcGC::new(),
            mark_sweep_gc: MarkSweepGC::new(),
            cycle_detection_interval: 1000, // Check for cycles every 1000 allocations
            allocations_since_cycle_check: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Record an allocation (for cycle detection scheduling)
    pub fn record_allocation(&self) {
        let count = self
            .allocations_since_cycle_check
            .fetch_add(1, Ordering::SeqCst)
            + 1;
        if count >= self.cycle_detection_interval {
            self.allocations_since_cycle_check
                .store(0, Ordering::SeqCst);
            // Trigger cycle detection
            let _ = self.mark_sweep_gc.collect();
        }
    }
}

impl GcStrategyTrait for HybridGC {
    fn collect(&self) -> GcStats {
        // Run mark-sweep for cycle detection
        self.mark_sweep_gc.collect()
    }

    fn name(&self) -> &'static str {
        "Hybrid"
    }
}

impl Default for HybridGC {
    fn default() -> Self {
        Self::new()
    }
}

/// GC manager that handles different strategies
pub struct GcManager {
    strategy: Arc<RwLock<Box<dyn GcStrategyTrait>>>,
    config: Arc<RwLock<crate::runtime::memory::config::GcConfig>>,
}

impl GcManager {
    pub fn new(config: crate::runtime::memory::config::GcConfig) -> Self {
        let strategy: Box<dyn GcStrategyTrait> = match config.strategy {
            GcStrategy::ReferenceCounting => Box::new(RcGC::new()),
            GcStrategy::MarkSweep => Box::new(MarkSweepGC::new()),
            GcStrategy::Hybrid => Box::new(HybridGC::new()),
            GcStrategy::None => Box::new(NoOpGC),
        };

        Self {
            strategy: Arc::new(RwLock::new(strategy)),
            config: Arc::new(RwLock::new(config)),
        }
    }

    pub fn collect(&self) -> GcStats {
        self.strategy.read().collect()
    }

    pub fn set_strategy(&self, strategy: GcStrategy) {
        let new_strategy: Box<dyn GcStrategyTrait> = match strategy {
            GcStrategy::ReferenceCounting => Box::new(RcGC::new()),
            GcStrategy::MarkSweep => Box::new(MarkSweepGC::new()),
            GcStrategy::Hybrid => Box::new(HybridGC::new()),
            GcStrategy::None => Box::new(NoOpGC),
        };
        *self.strategy.write() = new_strategy;
        self.config.write().strategy = strategy;
    }

    pub fn config(&self) -> Arc<RwLock<crate::runtime::memory::config::GcConfig>> {
        self.config.clone()
    }
}

/// No-op GC (for manual memory management)
struct NoOpGC;

impl GcStrategyTrait for NoOpGC {
    fn collect(&self) -> GcStats {
        GcStats::default()
    }

    fn name(&self) -> &'static str {
        "None"
    }
}

/// Global GC manager
static GLOBAL_GC: once_cell::sync::Lazy<GcManager> = once_cell::sync::Lazy::new(|| {
    let config = crate::runtime::memory::config::GcConfig::from_env();
    GcManager::new(config)
});

/// Get the global GC manager
pub fn get_gc() -> &'static GcManager {
    &GLOBAL_GC
}
