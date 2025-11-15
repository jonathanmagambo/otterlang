use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

/// Cache metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMetadata {
    pub key: String,
    pub source_path: PathBuf,
    pub cache_path: PathBuf,
    pub created_at: u64,
    pub dependencies: Vec<String>,
    pub binary_path: PathBuf,
    pub binary_size: u64,
    pub build_time_ms: u64,
    pub llvm_version: Option<String>,
}

impl CacheMetadata {
    pub fn new(
        key: String,
        source_path: PathBuf,
        dependencies: Vec<String>,
        binary_path: PathBuf,
        binary_size: u64,
        build_time_ms: u64,
        cache_path: PathBuf,
    ) -> Self {
        Self {
            key,
            source_path,
            cache_path,
            binary_path,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            dependencies,
            binary_size,
            build_time_ms,
            llvm_version: None,
        }
    }

    pub fn with_llvm_version(&mut self, version: String) {
        self.llvm_version = Some(version);
    }

    pub fn binary_size(&self) -> u64 {
        self.binary_size
    }

    pub fn is_valid(&self) -> bool {
        // Check if source file still exists and hasn't been modified

        if let Ok(metadata) = fs::metadata(&self.source_path)
            && let Ok(modified) = metadata.modified()
        {
            let modified_secs = modified
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            return modified_secs <= self.created_at;
        }

        false
    }
}
