use serde::{Deserialize, Serialize};

/// JIT cache metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMetadata {
    pub function_name: String,
    pub source_hash: String,
    pub compiled_at: u64,
    pub size: usize,
}

impl CacheMetadata {
    pub fn new(function_name: String, source_hash: String, size: usize) -> Self {
        Self {
            function_name,
            source_hash,
            size,
            compiled_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        }
    }

    pub fn is_valid(&self, current_source_hash: &str) -> bool {
        self.source_hash == current_source_hash
    }
}
