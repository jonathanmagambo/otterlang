//! Compilation profiling for tracking compilation performance

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::jit::tiered_compiler::CompilationTier;

/// Compilation event
#[derive(Debug, Clone)]
pub struct CompilationEvent {
    /// Function name
    pub function_name: String,

    /// Compilation tier
    pub tier: CompilationTier,

    /// Compilation duration
    pub duration: Duration,

    /// Timestamp
    pub timestamp_ms: u64,

    /// Generated code size in bytes
    pub code_size: usize,

    /// Number of optimization passes applied
    pub optimization_passes: u32,

    /// Whether this was a recompilation
    pub is_recompilation: bool,
}

/// Statistics for compilation at a specific tier
#[derive(Debug, Clone, Default)]
pub struct TierCompilationStats {
    /// Total number of compilations at this tier
    pub compilation_count: u64,

    /// Total time spent compiling (microseconds)
    pub total_time_us: u64,

    /// Minimum compilation time (microseconds)
    pub min_time_us: u64,

    /// Maximum compilation time (microseconds)
    pub max_time_us: u64,

    /// Total code size generated
    pub total_code_size: usize,

    /// Total optimization passes
    pub total_opt_passes: u32,
}

impl TierCompilationStats {
    pub fn new() -> Self {
        Self {
            min_time_us: u64::MAX,
            ..Default::default()
        }
    }

    /// Record a compilation event
    pub fn record_compilation(&mut self, duration_us: u64, code_size: usize, opt_passes: u32) {
        self.compilation_count += 1;
        self.total_time_us += duration_us;
        self.total_code_size += code_size;
        self.total_opt_passes += opt_passes;

        if duration_us < self.min_time_us {
            self.min_time_us = duration_us;
        }
        if duration_us > self.max_time_us {
            self.max_time_us = duration_us;
        }
    }

    /// Get average compilation time (microseconds)
    pub fn avg_time_us(&self) -> f64 {
        if self.compilation_count == 0 {
            0.0
        } else {
            self.total_time_us as f64 / self.compilation_count as f64
        }
    }

    /// Get average code size
    pub fn avg_code_size(&self) -> f64 {
        if self.compilation_count == 0 {
            0.0
        } else {
            self.total_code_size as f64 / self.compilation_count as f64
        }
    }

    /// Get average optimization passes
    pub fn avg_opt_passes(&self) -> f64 {
        if self.compilation_count == 0 {
            0.0
        } else {
            self.total_opt_passes as f64 / self.compilation_count as f64
        }
    }
}

/// Per-function compilation statistics
#[derive(Debug, Clone, Default)]
pub struct FunctionCompilationStats {
    /// Function name
    pub function_name: String,

    /// Number of times compiled
    pub compilation_count: u32,

    /// Total compilation time (microseconds)
    pub total_compilation_time_us: u64,

    /// Current tier
    pub current_tier: Option<CompilationTier>,

    /// Last compilation timestamp
    pub last_compiled_ms: u64,

    /// Code size at each tier
    pub code_size_per_tier: HashMap<CompilationTier, usize>,
}

impl FunctionCompilationStats {
    pub fn new(function_name: String) -> Self {
        Self {
            function_name,
            ..Default::default()
        }
    }

    /// Record a compilation
    pub fn record_compilation(
        &mut self,
        tier: CompilationTier,
        duration_us: u64,
        code_size: usize,
    ) {
        self.compilation_count += 1;
        self.total_compilation_time_us += duration_us;
        self.current_tier = Some(tier);
        self.last_compiled_ms = current_time_ms();
        self.code_size_per_tier.insert(tier, code_size);
    }

    /// Get average compilation time
    pub fn avg_compilation_time_us(&self) -> f64 {
        if self.compilation_count == 0 {
            0.0
        } else {
            self.total_compilation_time_us as f64 / self.compilation_count as f64
        }
    }

    /// Get code size growth from tier to tier
    pub fn code_size_growth(&self) -> HashMap<(CompilationTier, CompilationTier), f64> {
        let mut growth = HashMap::new();

        if let (Some(quick_size), Some(opt_size)) = (
            self.code_size_per_tier.get(&CompilationTier::Quick),
            self.code_size_per_tier.get(&CompilationTier::Optimized),
        ) {
            let ratio = *opt_size as f64 / *quick_size as f64;
            growth.insert((CompilationTier::Quick, CompilationTier::Optimized), ratio);
        }

        if let (Some(quick_size), Some(agg_size)) = (
            self.code_size_per_tier.get(&CompilationTier::Quick),
            self.code_size_per_tier.get(&CompilationTier::Aggressive),
        ) {
            let ratio = *agg_size as f64 / *quick_size as f64;
            growth.insert((CompilationTier::Quick, CompilationTier::Aggressive), ratio);
        }

        if let (Some(opt_size), Some(agg_size)) = (
            self.code_size_per_tier.get(&CompilationTier::Optimized),
            self.code_size_per_tier.get(&CompilationTier::Aggressive),
        ) {
            let ratio = *agg_size as f64 / *opt_size as f64;
            growth.insert(
                (CompilationTier::Optimized, CompilationTier::Aggressive),
                ratio,
            );
        }

        growth
    }
}

/// Compilation profiler
pub struct CompilationProfiler {
    /// Statistics per tier
    tier_stats: Arc<RwLock<HashMap<CompilationTier, TierCompilationStats>>>,

    /// Per-function statistics
    function_stats: Arc<RwLock<HashMap<String, FunctionCompilationStats>>>,

    /// Compilation event history
    event_history: Arc<RwLock<Vec<CompilationEvent>>>,

    /// Maximum history size
    max_history_size: usize,
}

impl CompilationProfiler {
    pub fn new() -> Self {
        let mut tier_stats = HashMap::new();
        tier_stats.insert(CompilationTier::Quick, TierCompilationStats::new());
        tier_stats.insert(CompilationTier::Optimized, TierCompilationStats::new());
        tier_stats.insert(CompilationTier::Aggressive, TierCompilationStats::new());

        Self {
            tier_stats: Arc::new(RwLock::new(tier_stats)),
            function_stats: Arc::new(RwLock::new(HashMap::new())),
            event_history: Arc::new(RwLock::new(Vec::new())),
            max_history_size: 1000,
        }
    }

    /// Start timing a compilation
    pub fn start_compilation(&self, function_name: &str) -> CompilationTimer {
        CompilationTimer::new(function_name.to_string())
    }

    /// Record a completed compilation
    pub fn record_compilation(
        &self,
        function_name: &str,
        tier: CompilationTier,
        duration: Duration,
        code_size: usize,
        optimization_passes: u32,
        is_recompilation: bool,
    ) {
        let duration_us = duration.as_micros() as u64;

        // Update tier stats
        let mut tier_stats = self.tier_stats.write();
        if let Some(stats) = tier_stats.get_mut(&tier) {
            stats.record_compilation(duration_us, code_size, optimization_passes);
        }
        drop(tier_stats);

        // Update function stats
        let mut func_stats = self.function_stats.write();
        let stats = func_stats
            .entry(function_name.to_string())
            .or_insert_with(|| FunctionCompilationStats::new(function_name.to_string()));
        stats.record_compilation(tier, duration_us, code_size);
        drop(func_stats);

        // Record event
        let mut history = self.event_history.write();
        if history.len() >= self.max_history_size {
            history.remove(0);
        }
        history.push(CompilationEvent {
            function_name: function_name.to_string(),
            tier,
            duration,
            timestamp_ms: current_time_ms(),
            code_size,
            optimization_passes,
            is_recompilation,
        });
    }

    /// Get statistics for a tier
    pub fn get_tier_stats(&self, tier: CompilationTier) -> Option<TierCompilationStats> {
        self.tier_stats.read().get(&tier).cloned()
    }

    /// Get all tier statistics
    pub fn get_all_tier_stats(&self) -> HashMap<CompilationTier, TierCompilationStats> {
        self.tier_stats.read().clone()
    }

    /// Get statistics for a function
    pub fn get_function_stats(&self, function_name: &str) -> Option<FunctionCompilationStats> {
        self.function_stats.read().get(function_name).cloned()
    }

    /// Get all function statistics
    pub fn get_all_function_stats(&self) -> Vec<FunctionCompilationStats> {
        self.function_stats.read().values().cloned().collect()
    }

    /// Get compilation event history
    pub fn get_event_history(&self) -> Vec<CompilationEvent> {
        self.event_history.read().clone()
    }

    /// Get total compilation time across all tiers
    pub fn total_compilation_time(&self) -> Duration {
        let total_us: u64 = self
            .tier_stats
            .read()
            .values()
            .map(|s| s.total_time_us)
            .sum();
        Duration::from_micros(total_us)
    }

    /// Get total code size generated
    pub fn total_code_size(&self) -> usize {
        self.tier_stats
            .read()
            .values()
            .map(|s| s.total_code_size)
            .sum()
    }

    /// Get compilation throughput (functions per second)
    pub fn compilation_throughput(&self, tier: CompilationTier) -> f64 {
        if let Some(stats) = self.tier_stats.read().get(&tier) {
            if stats.total_time_us == 0 {
                return 0.0;
            }
            let seconds = stats.total_time_us as f64 / 1_000_000.0;
            stats.compilation_count as f64 / seconds
        } else {
            0.0
        }
    }

    /// Clear all statistics
    pub fn clear(&self) {
        self.tier_stats.write().clear();
        self.function_stats.write().clear();
        self.event_history.write().clear();
    }
}

impl Default for CompilationProfiler {
    fn default() -> Self {
        Self::new()
    }
}

/// Timer for measuring compilation duration
pub struct CompilationTimer {
    function_name: String,
    start: Instant,
}

impl CompilationTimer {
    fn new(function_name: String) -> Self {
        Self {
            function_name,
            start: Instant::now(),
        }
    }

    /// Get elapsed time
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Get function name
    pub fn function_name(&self) -> &str {
        &self.function_name
    }
}

/// Get current time in milliseconds
fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tier_stats() {
        let profiler = CompilationProfiler::new();

        profiler.record_compilation(
            "test_fn",
            CompilationTier::Quick,
            Duration::from_micros(1000),
            512,
            1,
            false,
        );

        let stats = profiler.get_tier_stats(CompilationTier::Quick).unwrap();
        assert_eq!(stats.compilation_count, 1);
        assert_eq!(stats.total_time_us, 1000);
        assert_eq!(stats.total_code_size, 512);
    }

    #[test]
    fn test_function_stats() {
        let profiler = CompilationProfiler::new();

        profiler.record_compilation(
            "test_fn",
            CompilationTier::Quick,
            Duration::from_micros(1000),
            512,
            1,
            false,
        );

        profiler.record_compilation(
            "test_fn",
            CompilationTier::Optimized,
            Duration::from_micros(2000),
            1024,
            3,
            true,
        );

        let stats = profiler.get_function_stats("test_fn").unwrap();
        assert_eq!(stats.compilation_count, 2);
        assert_eq!(stats.total_compilation_time_us, 3000);
        assert_eq!(stats.current_tier, Some(CompilationTier::Optimized));
    }

    #[test]
    fn test_compilation_timer() {
        let timer = CompilationTimer::new("test_fn".to_string());
        std::thread::sleep(Duration::from_millis(10));
        let elapsed = timer.elapsed();
        assert!(elapsed >= Duration::from_millis(10));
    }
}
