//! Runtime configuration system
//!
//! Centralized configuration for all runtime components including tiered compilation,
//! profiling, GC, task scheduling, and caching.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

use crate::jit::tiered_compiler::TieredConfig;
use crate::memory::config::GcConfig;

/// Complete runtime configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuntimeConfig {
    /// Tiered compilation configuration
    pub tiered_compilation: TieredConfig,

    /// Garbage collection configuration
    pub gc: GcConfig,

    /// Profiling configuration
    pub profiling: ProfilingConfig,

    /// Cache configuration
    pub cache: CacheConfig,

    /// Task scheduler configuration
    pub scheduler: SchedulerConfig,
}

impl RuntimeConfig {
    /// Load configuration from environment variables
    pub fn from_env() -> Self {
        Self {
            tiered_compilation: TieredConfig::from_env(),
            gc: GcConfig::from_env(),
            profiling: ProfilingConfig::from_env(),
            cache: CacheConfig::from_env(),
            scheduler: SchedulerConfig::from_env(),
        }
    }

    /// Load configuration from TOML file
    #[cfg(feature = "toml-config")]
    pub fn from_file(path: &PathBuf) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let config: RuntimeConfig = toml::from_str(&contents)?;
        Ok(config)
    }

    /// Load configuration from TOML file (stub when toml feature is disabled)
    #[cfg(not(feature = "toml-config"))]
    pub fn from_file(_path: &PathBuf) -> anyhow::Result<Self> {
        anyhow::bail!("TOML support not enabled. Enable the 'toml-config' feature.")
    }

    /// Save configuration to TOML file
    #[cfg(feature = "toml-config")]
    pub fn save_to_file(&self, path: &PathBuf) -> anyhow::Result<()> {
        let contents = toml::to_string_pretty(self)?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// Save configuration to TOML file (stub when toml feature is disabled)
    #[cfg(not(feature = "toml-config"))]
    pub fn save_to_file(&self, _path: &PathBuf) -> anyhow::Result<()> {
        anyhow::bail!("TOML support not enabled. Enable the 'toml-config' feature.")
    }

    /// Merge with environment variables (env vars take precedence)
    pub fn merge_with_env(mut self) -> Self {
        let env_config = Self::from_env();

        // Merge tiered compilation
        if std::env::var("OTTER_TIER_ENABLED").is_ok() {
            self.tiered_compilation.enabled = env_config.tiered_compilation.enabled;
        }

        // Merge profiling
        if std::env::var("OTTER_PROFILE").is_ok() {
            self.profiling.enabled = env_config.profiling.enabled;
        }

        self
    }
}

/// Profiling configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilingConfig {
    /// Enable profiling
    pub enabled: bool,

    /// Enable memory profiling
    pub memory_profiling: bool,

    /// Enable compilation profiling
    pub compilation_profiling: bool,

    /// Sampling rate (1 = every call, 10 = every 10th call)
    pub sampling_rate: u32,

    /// Enable stack trace collection (expensive)
    pub collect_stack_traces: bool,

    /// Maximum profiling history size
    pub max_history_size: usize,
}

impl Default for ProfilingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            memory_profiling: false,
            compilation_profiling: false,
            sampling_rate: 1,
            collect_stack_traces: false,
            max_history_size: 10000,
        }
    }
}

impl ProfilingConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(val) = std::env::var("OTTER_PROFILE") {
            config.enabled = val.parse().unwrap_or(true);
        }

        if let Ok(val) = std::env::var("OTTER_PROFILE_MEMORY") {
            config.memory_profiling = val.parse().unwrap_or(true);
        }

        if let Ok(val) = std::env::var("OTTER_PROFILE_COMPILATION") {
            config.compilation_profiling = val.parse().unwrap_or(true);
        }

        if let Ok(val) = std::env::var("OTTER_PROFILE_SAMPLING_RATE") {
            config.sampling_rate = val.parse().unwrap_or(1);
        }

        if let Ok(val) = std::env::var("OTTER_PROFILE_STACK_TRACES") {
            config.collect_stack_traces = val.parse().unwrap_or(false);
        }

        config
    }
}

/// Cache configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Enable function caching
    pub enabled: bool,

    /// Maximum cache size in bytes
    pub max_size_bytes: usize,

    /// Cache eviction strategy
    pub eviction_strategy: EvictionStrategy,

    /// Enable cache warming (precompile common functions)
    pub cache_warming: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_size_bytes: 100 * 1024 * 1024, // 100 MB
            eviction_strategy: EvictionStrategy::LRU,
            cache_warming: false,
        }
    }
}

impl CacheConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(val) = std::env::var("OTTER_CACHE_ENABLED") {
            config.enabled = val.parse().unwrap_or(true);
        }

        if let Ok(val) = std::env::var("OTTER_CACHE_SIZE_MB") {
            let size_mb: usize = val.parse().unwrap_or(100);
            config.max_size_bytes = size_mb * 1024 * 1024;
        }

        config
    }
}

/// Cache eviction strategy
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum EvictionStrategy {
    /// Least Recently Used
    LRU,
    /// Least Frequently Used
    LFU,
    /// First In First Out
    FIFO,
}

/// Task scheduler configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerConfig {
    /// Number of worker threads (0 = auto-detect)
    pub worker_threads: usize,

    /// Enable work stealing
    pub work_stealing: bool,

    /// Task queue capacity
    pub queue_capacity: usize,

    /// Enable task metrics collection
    pub collect_metrics: bool,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            worker_threads: 0, // Auto-detect
            work_stealing: true,
            queue_capacity: 10000,
            collect_metrics: false,
        }
    }
}

impl SchedulerConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(val) = std::env::var("OTTER_WORKER_THREADS") {
            config.worker_threads = val.parse().unwrap_or(0);
        }

        if let Ok(val) = std::env::var("OTTER_WORK_STEALING") {
            config.work_stealing = val.parse().unwrap_or(true);
        }

        if let Ok(val) = std::env::var("OTTER_TASK_METRICS") {
            config.collect_metrics = val.parse().unwrap_or(false);
        }

        config
    }
}

/// Global runtime configuration manager
pub struct ConfigManager {
    config: Arc<RwLock<RuntimeConfig>>,
}

impl ConfigManager {
    pub fn new() -> Self {
        Self {
            config: Arc::new(RwLock::new(RuntimeConfig::default())),
        }
    }

    /// Initialize from environment and optional config file
    pub fn init(&self, config_file: Option<PathBuf>) -> anyhow::Result<()> {
        let mut config = if let Some(path) = config_file {
            RuntimeConfig::from_file(&path)?
        } else {
            RuntimeConfig::default()
        };

        // Merge with environment variables
        config = config.merge_with_env();

        *self.config.write() = config;
        Ok(())
    }

    /// Get current configuration
    pub fn get(&self) -> RuntimeConfig {
        self.config.read().clone()
    }

    /// Update configuration
    pub fn update<F>(&self, f: F)
    where
        F: FnOnce(&mut RuntimeConfig),
    {
        let mut config = self.config.write();
        f(&mut config);
    }

    /// Get tiered compilation config
    pub fn tiered_compilation(&self) -> TieredConfig {
        self.config.read().tiered_compilation.clone()
    }

    /// Get profiling config
    pub fn profiling(&self) -> ProfilingConfig {
        self.config.read().profiling.clone()
    }

    /// Get cache config
    pub fn cache(&self) -> CacheConfig {
        self.config.read().cache.clone()
    }

    /// Get scheduler config
    pub fn scheduler(&self) -> SchedulerConfig {
        self.config.read().scheduler.clone()
    }

    /// Check if profiling is enabled
    pub fn is_profiling_enabled(&self) -> bool {
        self.config.read().profiling.enabled
    }

    /// Check if tiered compilation is enabled
    pub fn is_tiered_compilation_enabled(&self) -> bool {
        self.config.read().tiered_compilation.enabled
    }
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Global configuration manager instance
static GLOBAL_CONFIG: once_cell::sync::Lazy<ConfigManager> = once_cell::sync::Lazy::new(|| {
    let manager = ConfigManager::new();
    // Try to load from default config file if toml feature is enabled
    #[cfg(feature = "toml-config")]
    {
        let default_config_path = PathBuf::from("otter.toml");
        if default_config_path.exists() {
            let _ = manager.init(Some(default_config_path));
        } else {
            let _ = manager.init(None);
        }
    }
    #[cfg(not(feature = "toml-config"))]
    {
        let _ = manager.init(None);
    }
    manager
});

/// Get the global configuration manager
pub fn get_config() -> &'static ConfigManager {
    &GLOBAL_CONFIG
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RuntimeConfig::default();
        assert!(config.tiered_compilation.enabled);
        assert!(!config.profiling.enabled);
    }

    #[test]
    #[cfg(feature = "toml-config")]
    fn test_config_serialization() {
        let config = RuntimeConfig::default();
        let toml = toml::to_string(&config).unwrap();
        let deserialized: RuntimeConfig = toml::from_str(&toml).unwrap();

        assert_eq!(
            config.tiered_compilation.enabled,
            deserialized.tiered_compilation.enabled
        );
    }

    #[test]
    fn test_config_manager() {
        let manager = ConfigManager::new();
        let config = manager.get();
        assert!(config.cache.enabled);
    }
}
