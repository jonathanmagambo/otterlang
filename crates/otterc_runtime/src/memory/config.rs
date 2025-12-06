//! Garbage collection configuration

use serde::{Deserialize, Serialize};

/// Garbage collection strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GcStrategy {
    /// Reference counting only (no cycle detection)
    ReferenceCounting,
    /// Mark-and-sweep garbage collection
    MarkSweep,
    /// Generational: Nursery (Bump Pointer) + Old Gen (Mark-Sweep)
    #[default]
    Generational,
    /// No garbage collection (manual management)
    None,
}

impl std::str::FromStr for GcStrategy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "rc" | "reference-counting" | "reference_counting" => Ok(GcStrategy::ReferenceCounting),
            "mark-sweep" | "mark_sweep" | "ms" => Ok(GcStrategy::MarkSweep),
            "generational" | "gen" => Ok(GcStrategy::Generational),
            "none" => Ok(GcStrategy::None),
            _ => Err(format!("Unknown GC strategy: {}", s)),
        }
    }
}

/// Garbage collection configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcConfig {
    /// GC strategy to use
    pub strategy: GcStrategy,
    /// Memory threshold (0.0-1.0) before triggering GC
    pub memory_threshold: f64,
    /// Interval between GC cycles in milliseconds (0 = disabled)
    pub gc_interval_ms: u64,
    /// Enable automatic GC on memory pressure
    pub auto_gc: bool,
    /// Maximum heap size in bytes (0 = unlimited)
    pub max_heap_size: usize,
    /// Maximum bytes that may be allocated while GC is disabled (0 = unlimited)
    pub disabled_heap_limit: usize,
}

impl Default for GcConfig {
    fn default() -> Self {
        Self {
            strategy: GcStrategy::MarkSweep,
            memory_threshold: 0.8, // 80% memory usage
            gc_interval_ms: 5000,  // 5 seconds
            auto_gc: true,
            max_heap_size: 0,                      // Unlimited
            disabled_heap_limit: 64 * 1024 * 1024, // 64MB safeguard while GC disabled
        }
    }
}

impl GcConfig {
    /// Create a new GC configuration
    pub fn new(strategy: GcStrategy) -> Self {
        Self {
            strategy,
            ..Default::default()
        }
    }

    /// Load configuration from environment variables
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(strategy_str) = std::env::var("OTTER_GC_STRATEGY")
            && let Ok(strategy) = strategy_str.parse()
        {
            config.strategy = strategy;
        }

        if let Ok(threshold) = std::env::var("OTTER_GC_THRESHOLD")
            && let Ok(threshold_val) = threshold.parse::<f64>()
        {
            config.memory_threshold = threshold_val.clamp(0., 1.);
        }

        if let Ok(interval) = std::env::var("OTTER_GC_INTERVAL")
            && let Ok(interval_ms) = interval.parse::<u64>()
        {
            config.gc_interval_ms = interval_ms;
        }

        if let Ok(limit) = std::env::var("OTTER_GC_DISABLED_MAX_BYTES")
            && let Ok(limit_bytes) = limit.parse::<usize>()
        {
            config.disabled_heap_limit = limit_bytes;
        }

        config
    }
}
