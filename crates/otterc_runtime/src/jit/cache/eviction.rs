use super::function_cache::CachedFunction;

/// Cache eviction policy
pub struct CacheEvictor {
    max_size: usize,
    current_size: usize,
}

impl CacheEvictor {
    pub fn new(max_size: usize) -> Self {
        Self {
            max_size,
            current_size: 0,
        }
    }

    pub fn should_evict(&self, _function: &CachedFunction) -> bool {
        self.current_size >= self.max_size
    }

    pub fn evict(&mut self, function: &CachedFunction) {
        self.current_size = self.current_size.saturating_sub(function.size());
    }

    pub fn add(&mut self, function: &CachedFunction) {
        self.current_size += function.size();
    }
}
