use std::path::{Path, PathBuf};

/// Cache path utilities
pub fn cache_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut cache_dir = directories::BaseDirs::new()
        .ok_or("Could not determine cache directory")?
        .cache_dir()
        .to_path_buf();
    cache_dir.push("otterlang");
    Ok(cache_dir)
}

pub fn ensure_cache_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let cache_dir = cache_root()?;
    std::fs::create_dir_all(&cache_dir)?;
    Ok(cache_dir)
}

pub fn cache_key_for_file(path: &Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);

    // Also hash file modification time if possible
    if let Ok(metadata) = std::fs::metadata(path)
        && let Ok(modified) = metadata.modified()
    {
        modified.hash(&mut hasher);
    }

    format!("{:x}", hasher.finish())
}
