// Runtime Profiling Infrastructure
pub mod call_profiler;
pub mod compilation_profiler;
pub mod hot_detector;
pub mod memory_profiler;
pub mod sampler;

pub use call_profiler::CallProfiler;
pub use compilation_profiler::{CompilationProfiler, CompilationTimer};
pub use hot_detector::{HotDetector, HotFunction};
pub use memory_profiler::MemoryProfiler;
pub use sampler::Sampler;

use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Duration;

/// Aggregated profiling metrics for a function
#[derive(Debug, Clone)]
pub struct FunctionMetrics {
    pub name: String,
    pub call_count: u64,
    pub total_time: Duration,
    pub avg_time: Duration,
    pub max_time: Duration,
    pub min_time: Duration,
}

impl FunctionMetrics {
    pub fn new(name: String) -> Self {
        Self {
            name,
            call_count: 0,
            total_time: Duration::ZERO,
            avg_time: Duration::ZERO,
            max_time: Duration::ZERO,
            min_time: Duration::ZERO,
        }
    }

    pub fn record_call(&mut self, duration: Duration) {
        self.call_count += 1;
        self.total_time += duration;
        // Calculate average using nanoseconds to avoid division issues
        let avg_nanos = if self.call_count > 0 {
            self.total_time.as_nanos() / self.call_count as u128
        } else {
            0
        };
        self.avg_time = Duration::from_nanos(avg_nanos.min(u64::MAX as u128) as u64);

        if duration > self.max_time {
            self.max_time = duration;
        }
        if self.min_time == Duration::ZERO || duration < self.min_time {
            self.min_time = duration;
        }
    }

    pub fn time_percentage(&self, total_time: Duration) -> f64 {
        if total_time.as_nanos() == 0 {
            return 0.0;
        }
        (self.total_time.as_nanos() as f64 / total_time.as_nanos() as f64) * 100.0
    }
}

/// Global profiler instance
pub struct GlobalProfiler {
    metrics: Arc<RwLock<std::collections::HashMap<String, FunctionMetrics>>>,
    sampler: Arc<RwLock<Sampler>>,
    detector: Arc<RwLock<HotDetector>>,
}

impl GlobalProfiler {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(RwLock::new(std::collections::HashMap::new())),
            sampler: Arc::new(RwLock::new(Sampler::new())),
            detector: Arc::new(RwLock::new(HotDetector::default())),
        }
    }

    pub fn record_call(&self, function_name: &str, duration: Duration) {
        let mut metrics = self.metrics.write();
        let func_metrics = metrics
            .entry(function_name.to_string())
            .or_insert_with(|| FunctionMetrics::new(function_name.to_string()));
        func_metrics.record_call(duration);

        // Sample for hot detection
        self.sampler.write().record_call(function_name, duration);
    }

    pub fn get_metrics(&self, function_name: &str) -> Option<FunctionMetrics> {
        self.metrics.read().get(function_name).cloned()
    }

    pub fn get_all_metrics(&self) -> Vec<FunctionMetrics> {
        self.metrics.read().values().cloned().collect()
    }

    pub fn check_hot_functions(&self) -> Vec<HotFunction> {
        let total_time: Duration = self.metrics.read().values().map(|m| m.total_time).sum();

        let mut detector = self.detector.write();
        detector.detect_hot_functions(&self.metrics.read(), total_time)
    }

    pub fn sampler(&self) -> Arc<RwLock<Sampler>> {
        self.sampler.clone()
    }
}

impl Default for GlobalProfiler {
    fn default() -> Self {
        Self::new()
    }
}
