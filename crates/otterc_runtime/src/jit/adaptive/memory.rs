use parking_lot::RwLock;
use std::collections::HashMap;

/// Adaptive memory management based on runtime patterns
pub struct AdaptiveMemoryManager {
    allocation_profiles: RwLock<HashMap<String, AllocationProfile>>,
    pressure_threshold: f64,
}

#[derive(Debug, Clone)]
struct AllocationProfile {
    total_allocations: usize,
    common_sizes: HashMap<usize, usize>,
}

impl AdaptiveMemoryManager {
    pub fn new() -> Self {
        Self {
            allocation_profiles: RwLock::new(HashMap::new()),
            pressure_threshold: 0.8, // 80% memory usage triggers optimization
        }
    }

    pub fn record_allocation(&self, function_name: &str, size: usize) {
        let mut profiles = self.allocation_profiles.write();
        let profile = profiles
            .entry(function_name.to_string())
            .or_insert_with(|| AllocationProfile {
                total_allocations: 0,
                common_sizes: HashMap::new(),
            });

        profile.total_allocations += 1;
        *profile.common_sizes.entry(size).or_insert(0) += 1;
    }

    pub fn detect_memory_pressure(&self, current_usage: usize, max_usage: usize) -> bool {
        if max_usage == 0 {
            return false;
        }
        (current_usage as f64 / max_usage as f64) >= self.pressure_threshold
    }

    pub fn get_optimal_pool_size(&self, function_name: &str) -> Option<usize> {
        let profiles = self.allocation_profiles.read();
        let profile = profiles.get(function_name)?;

        // Find the most common allocation size
        profile
            .common_sizes
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(size, _)| *size)
    }

    pub fn should_optimize(&self, function_name: &str) -> bool {
        let profiles = self.allocation_profiles.read();
        if let Some(profile) = profiles.get(function_name) {
            // Optimize if function has high allocation frequency
            profile.total_allocations > 1000
        } else {
            false
        }
    }
}

impl Default for AdaptiveMemoryManager {
    fn default() -> Self {
        Self::new()
    }
}
