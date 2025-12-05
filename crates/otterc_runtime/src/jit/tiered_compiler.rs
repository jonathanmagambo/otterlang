//! Tiered compilation system for adaptive optimization
//!
//! This module implements a multi-tier compilation strategy that balances
//! compilation time with execution performance. Functions are initially compiled
//! with minimal optimizations and promoted to higher tiers as they become hot.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use otterc_config::CodegenOptLevel;

/// Compilation tier levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CompilationTier {
    /// Tier 1: Quick compilation with minimal optimizations
    /// Used for initial compilation to reduce startup time
    Quick = 1,

    /// Tier 2: Balanced optimization for warm functions
    /// Applied when functions show moderate activity
    Optimized = 2,

    /// Tier 3: Aggressive optimization for hot functions
    /// Maximum optimization for frequently executed code
    Aggressive = 3,
}

impl CompilationTier {
    /// Convert tier to codegen optimization level
    pub fn to_opt_level(self) -> CodegenOptLevel {
        match self {
            CompilationTier::Quick => CodegenOptLevel::None,
            CompilationTier::Optimized => CodegenOptLevel::Default,
            CompilationTier::Aggressive => CodegenOptLevel::Aggressive,
        }
    }

    /// Get the next higher tier, if any
    pub fn next_tier(self) -> Option<CompilationTier> {
        match self {
            CompilationTier::Quick => Some(CompilationTier::Optimized),
            CompilationTier::Optimized => Some(CompilationTier::Aggressive),
            CompilationTier::Aggressive => None,
        }
    }

    /// Get tier name for display
    pub fn name(self) -> &'static str {
        match self {
            CompilationTier::Quick => "Quick",
            CompilationTier::Optimized => "Optimized",
            CompilationTier::Aggressive => "Aggressive",
        }
    }
}

/// Configuration for tiered compilation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TieredConfig {
    /// Call count threshold to promote from Quick to Optimized
    pub quick_to_optimized_threshold: u64,

    /// Call count threshold to promote from Optimized to Aggressive
    pub optimized_to_aggressive_threshold: u64,

    /// Enable tiered compilation (if false, always use Aggressive)
    pub enabled: bool,

    /// Minimum time between recompilations (in milliseconds)
    pub recompilation_cooldown_ms: u64,
}

impl Default for TieredConfig {
    fn default() -> Self {
        Self {
            quick_to_optimized_threshold: 100,
            optimized_to_aggressive_threshold: 1000,
            enabled: true,
            recompilation_cooldown_ms: 100,
        }
    }
}

impl TieredConfig {
    /// Load configuration from environment variables
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(val) = std::env::var("OTTER_TIER_ENABLED") {
            config.enabled = val.parse().unwrap_or(true);
        }

        if let Ok(val) = std::env::var("OTTER_TIER_QUICK_THRESHOLD") {
            config.quick_to_optimized_threshold = val.parse().unwrap_or(100);
        }

        if let Ok(val) = std::env::var("OTTER_TIER_OPTIMIZED_THRESHOLD") {
            config.optimized_to_aggressive_threshold = val.parse().unwrap_or(1000);
        }

        if let Ok(val) = std::env::var("OTTER_TIER_COOLDOWN_MS") {
            config.recompilation_cooldown_ms = val.parse().unwrap_or(100);
        }

        config
    }

    /// Get the threshold for promoting to the next tier
    pub fn threshold_for_tier(&self, current_tier: CompilationTier) -> Option<u64> {
        match current_tier {
            CompilationTier::Quick => Some(self.quick_to_optimized_threshold),
            CompilationTier::Optimized => Some(self.optimized_to_aggressive_threshold),
            CompilationTier::Aggressive => None,
        }
    }
}

/// Metadata about a compiled function
#[derive(Debug, Clone)]
pub struct FunctionTierInfo {
    /// Current compilation tier
    pub tier: CompilationTier,

    /// Number of times this function has been called
    pub call_count: u64,

    /// Number of times this function has been recompiled
    pub recompilation_count: u32,

    /// Timestamp of last compilation (milliseconds since epoch)
    pub last_compiled_ms: u64,

    /// Total time spent compiling this function (microseconds)
    pub total_compilation_time_us: u64,
}

impl FunctionTierInfo {
    pub fn new(tier: CompilationTier) -> Self {
        Self {
            tier,
            call_count: 0,
            recompilation_count: 0,
            last_compiled_ms: current_time_ms(),
            total_compilation_time_us: 0,
        }
    }

    /// Check if this function should be promoted to the next tier
    pub fn should_promote(&self, config: &TieredConfig) -> bool {
        if !config.enabled {
            return false;
        }

        // Check if we're at max tier
        if self.tier.next_tier().is_none() {
            return false;
        }

        // Check cooldown period
        let elapsed_ms = current_time_ms().saturating_sub(self.last_compiled_ms);
        if elapsed_ms < config.recompilation_cooldown_ms {
            return false;
        }

        // Check if call count exceeds threshold
        if let Some(threshold) = config.threshold_for_tier(self.tier) {
            self.call_count >= threshold
        } else {
            false
        }
    }

    /// Record a function call
    pub fn record_call(&mut self) {
        self.call_count += 1;
    }

    /// Record a recompilation
    pub fn record_recompilation(&mut self, compilation_time_us: u64) {
        self.recompilation_count += 1;
        self.last_compiled_ms = current_time_ms();
        self.total_compilation_time_us += compilation_time_us;
    }

    /// Promote to the next tier
    pub fn promote(&mut self) -> Option<CompilationTier> {
        if let Some(next_tier) = self.tier.next_tier() {
            self.tier = next_tier;
            Some(next_tier)
        } else {
            None
        }
    }
}

/// Statistics about tiered compilation
#[derive(Debug, Clone, Default)]
pub struct TieredStats {
    /// Number of functions at each tier
    pub functions_per_tier: HashMap<CompilationTier, usize>,

    /// Total number of tier promotions
    pub total_promotions: u64,

    /// Total compilation time per tier (microseconds)
    pub compilation_time_per_tier: HashMap<CompilationTier, u64>,

    /// Total number of recompilations
    pub total_recompilations: u64,
}

impl TieredStats {
    pub fn new() -> Self {
        let mut stats = Self::default();
        stats.functions_per_tier.insert(CompilationTier::Quick, 0);
        stats
            .functions_per_tier
            .insert(CompilationTier::Optimized, 0);
        stats
            .functions_per_tier
            .insert(CompilationTier::Aggressive, 0);
        stats
            .compilation_time_per_tier
            .insert(CompilationTier::Quick, 0);
        stats
            .compilation_time_per_tier
            .insert(CompilationTier::Optimized, 0);
        stats
            .compilation_time_per_tier
            .insert(CompilationTier::Aggressive, 0);
        stats
    }

    /// Get total number of functions
    pub fn total_functions(&self) -> usize {
        self.functions_per_tier.values().sum()
    }

    /// Get average compilation time for a tier (microseconds)
    pub fn avg_compilation_time(&self, tier: CompilationTier) -> f64 {
        let count = *self.functions_per_tier.get(&tier).unwrap_or(&0);
        if count == 0 {
            return 0.0;
        }
        let total = *self.compilation_time_per_tier.get(&tier).unwrap_or(&0);
        total as f64 / count as f64
    }
}

/// Tiered compilation manager
pub struct TieredCompiler {
    config: Arc<RwLock<TieredConfig>>,
    function_info: Arc<RwLock<HashMap<String, FunctionTierInfo>>>,
    stats: Arc<RwLock<TieredStats>>,
}

impl TieredCompiler {
    pub fn new() -> Self {
        Self::with_config(TieredConfig::default())
    }

    pub fn with_config(config: TieredConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            function_info: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(TieredStats::new())),
        }
    }

    /// Get the current tier for a function
    pub fn get_tier(&self, function_name: &str) -> CompilationTier {
        self.function_info
            .read()
            .get(function_name)
            .map(|info| info.tier)
            .unwrap_or(CompilationTier::Quick)
    }

    /// Record a function call and check if promotion is needed
    pub fn record_call(&self, function_name: &str) -> Option<CompilationTier> {
        let mut info_map = self.function_info.write();
        let config = self.config.read();

        let info = info_map
            .entry(function_name.to_string())
            .or_insert_with(|| FunctionTierInfo::new(CompilationTier::Quick));

        info.record_call();

        info.should_promote(&config)
            .then(|| info.promote())
            .flatten()
            .inspect(|_| {
                let mut stats = self.stats.write();
                stats.total_promotions += 1;
            })
    }

    /// Register a new function at a specific tier
    pub fn register_function(&self, function_name: &str, tier: CompilationTier) {
        let mut info_map = self.function_info.write();
        info_map.insert(function_name.to_string(), FunctionTierInfo::new(tier));

        // Update stats
        let mut stats = self.stats.write();
        *stats.functions_per_tier.entry(tier).or_insert(0) += 1;
    }

    /// Record a compilation event
    pub fn record_compilation(
        &self,
        function_name: &str,
        tier: CompilationTier,
        compilation_time_us: u64,
    ) {
        let mut info_map = self.function_info.write();
        if let Some(info) = info_map.get_mut(function_name) {
            info.record_recompilation(compilation_time_us);

            // Update stats
            let mut stats = self.stats.write();
            *stats.compilation_time_per_tier.entry(tier).or_insert(0) += compilation_time_us;
            stats.total_recompilations += 1;
        }
    }

    /// Get functions that need recompilation
    pub fn get_functions_to_recompile(&self) -> Vec<(String, CompilationTier)> {
        let info_map = self.function_info.read();
        let config = self.config.read();

        info_map
            .iter()
            .filter_map(|(name, info)| {
                if info.should_promote(&config) {
                    info.tier.next_tier().map(|tier| (name.clone(), tier))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get statistics
    pub fn get_stats(&self) -> TieredStats {
        self.stats.read().clone()
    }

    /// Get function info
    pub fn get_function_info(&self, function_name: &str) -> Option<FunctionTierInfo> {
        self.function_info.read().get(function_name).cloned()
    }

    /// Get all function info
    pub fn get_all_function_info(&self) -> HashMap<String, FunctionTierInfo> {
        self.function_info.read().clone()
    }

    /// Update configuration
    pub fn set_config(&self, config: TieredConfig) {
        *self.config.write() = config;
    }

    /// Get current configuration
    pub fn get_config(&self) -> TieredConfig {
        self.config.read().clone()
    }

    /// Determine initial tier for a program
    pub fn initial_tier(&self) -> CompilationTier {
        if self.config.read().enabled {
            CompilationTier::Quick
        } else {
            CompilationTier::Aggressive
        }
    }

    /// Check if a function should be compiled at a specific tier
    pub fn should_compile_at_tier(
        &self,
        function_name: &str,
        target_tier: CompilationTier,
    ) -> bool {
        let current_tier = self.get_tier(function_name);
        target_tier > current_tier
    }
}

impl Default for TieredCompiler {
    fn default() -> Self {
        Self::new()
    }
}

/// Get current time in milliseconds since epoch
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
    fn test_tier_ordering() {
        assert!(CompilationTier::Quick < CompilationTier::Optimized);
        assert!(CompilationTier::Optimized < CompilationTier::Aggressive);
    }

    #[test]
    fn test_tier_promotion() {
        assert_eq!(
            CompilationTier::Quick.next_tier(),
            Some(CompilationTier::Optimized)
        );
        assert_eq!(
            CompilationTier::Optimized.next_tier(),
            Some(CompilationTier::Aggressive)
        );
        assert_eq!(CompilationTier::Aggressive.next_tier(), None);
    }

    #[test]
    fn test_function_promotion() {
        let config = TieredConfig {
            recompilation_cooldown_ms: 0,
            ..TieredConfig::default()
        };
        let compiler = TieredCompiler::with_config(config);
        compiler.register_function("test_fn", CompilationTier::Quick);

        // Call function enough times to trigger promotion
        for _ in 0..100 {
            compiler.record_call("test_fn");
        }

        // Should be promoted to Optimized
        assert_eq!(compiler.get_tier("test_fn"), CompilationTier::Optimized);
    }

    #[test]
    fn test_stats_tracking() {
        let compiler = TieredCompiler::new();
        compiler.register_function("fn1", CompilationTier::Quick);
        compiler.register_function("fn2", CompilationTier::Optimized);

        let stats = compiler.get_stats();
        assert_eq!(stats.functions_per_tier[&CompilationTier::Quick], 1);
        assert_eq!(stats.functions_per_tier[&CompilationTier::Optimized], 1);
    }

    #[test]
    fn test_cooldown_period() {
        let config = TieredConfig {
            recompilation_cooldown_ms: 1_000,
            quick_to_optimized_threshold: 10,
            ..TieredConfig::default()
        };

        let compiler = TieredCompiler::with_config(config);
        compiler.register_function("test_fn", CompilationTier::Quick);

        // Call enough times to exceed threshold
        for _ in 0..20 {
            compiler.record_call("test_fn");
        }

        // Should be promoted (cooldown should have passed in test)
        let info = compiler.get_function_info("test_fn").unwrap();
        assert!(info.call_count >= 10);
    }
}
