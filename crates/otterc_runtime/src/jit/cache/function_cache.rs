use std::collections::HashMap;

/// Cached JIT-compiled function
#[derive(Debug, Clone)]
pub struct CachedFunction {
    pub name: String,
    pub address: usize,
    pub size: usize,
    pub last_used: u64,
}

impl CachedFunction {
    pub fn new(name: String, address: usize, size: usize) -> Self {
        Self {
            name,
            address,
            size,
            last_used: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn touch(&mut self) {
        self.last_used = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
}

/// Function cache
pub struct FunctionCache {
    functions: HashMap<String, CachedFunction>,
}

impl Default for FunctionCache {
    fn default() -> Self {
        Self::new()
    }
}

impl FunctionCache {
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
        }
    }

    pub fn new_with_capacity(_capacity: usize) -> Self {
        Self::new()
    }

    pub fn get(&self, name: &str) -> Option<&CachedFunction> {
        self.functions.get(name)
    }

    pub fn put(&mut self, function: CachedFunction) {
        self.functions.insert(function.name.clone(), function);
    }

    pub fn remove(&mut self, name: &str) -> Option<CachedFunction> {
        self.functions.remove(name)
    }

    pub fn clear(&mut self) {
        self.functions.clear();
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            total_functions: self.functions.len(),
            total_size: self.functions.values().map(|f| f.size).sum(),
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    pub total_functions: usize,
    pub total_size: usize,
}
