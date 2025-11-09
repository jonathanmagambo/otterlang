use anyhow::{Context, Result, anyhow, bail};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[cfg(test)]
use std::fs;

/// Represents a module path that can be resolved
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModulePath {
    /// Standard library module: `use otter:math`
    Stdlib(String),
    /// Rust FFI module: `use rust:serde_json`
    Rust(String),
    /// Unqualified module path, prefers stdlib (e.g. `use fmt`)
    Unqualified(String),
    /// Relative path: `use ./math` or `use ../utils`
    Relative(PathBuf),
    /// Absolute path: `use /path/to/module`
    Absolute(PathBuf),
}

impl ModulePath {
    /// Parse a module path from a use statement
    pub fn from_string(module: &str, _source_dir: &Path) -> Result<Self> {
        if let Some((namespace, name)) = module.split_once(':') {
            match namespace {
                "otter" => Ok(ModulePath::Stdlib(name.to_string())),
                "rust" => Ok(ModulePath::Rust(name.to_string())),
                _ => bail!("unknown module namespace: {}", namespace),
            }
        } else if module.starts_with('/') {
            Ok(ModulePath::Absolute(PathBuf::from(module)))
        } else if module.starts_with("./") || module.starts_with("../") {
            // Relative path
            let relative = PathBuf::from(module);
            Ok(ModulePath::Relative(relative))
        } else {
            // Try as unqualified module path (potential stdlib or relative)
            Ok(ModulePath::Unqualified(module.to_string()))
        }
    }

    /// Resolve this module path to an actual file path
    pub fn resolve(&self, source_dir: &Path, stdlib_dir: Option<&Path>) -> Result<PathBuf> {
        match self {
            ModulePath::Stdlib(name) => {
                let stdlib =
                    stdlib_dir.ok_or_else(|| anyhow!("stdlib directory not configured"))?;
                Self::resolve_stdlib_path(stdlib, name)?.ok_or_else(|| {
                    anyhow!(
                        "stdlib module '{}' not found at {}",
                        name,
                        stdlib.join(name).display()
                    )
                })
            }
            ModulePath::Rust(_) => {
                // Rust modules are handled separately via FFI
                bail!("Rust modules should be handled via FFI system")
            }
            ModulePath::Unqualified(name) => {
                if let Some(stdlib) = stdlib_dir {
                    if let Some(path) = Self::resolve_stdlib_path(stdlib, name)? {
                        return Ok(path);
                    }
                }
                let relative = Self::module_name_to_path(name);
                Self::resolve_relative_path(source_dir, &relative)
            }
            ModulePath::Relative(rel_path) => Self::resolve_relative_path(source_dir, rel_path),
            ModulePath::Absolute(abs_path) => Self::resolve_absolute_path(abs_path),
        }
    }
}

impl ModulePath {
    fn module_name_to_path(name: &str) -> PathBuf {
        if name.contains(std::path::MAIN_SEPARATOR) {
            return PathBuf::from(name);
        }

        let mut path = PathBuf::new();
        for segment in name.split('.') {
            if segment.is_empty() {
                continue;
            }
            path.push(segment);
        }

        if path.components().count() == 0 {
            PathBuf::from(name)
        } else {
            path
        }
    }

    fn resolve_stdlib_path(stdlib: &Path, name: &str) -> Result<Option<PathBuf>> {
        let mut path = stdlib.join(Self::module_name_to_path(name));
        if path.is_dir() {
            path = path.join("mod.ot");
        } else {
            // Append .ot if no extension yet
            if !path.extension().map_or(false, |ext| ext == "ot") {
                path.set_extension("ot");
            }
        }

        if path.exists() {
            Ok(Some(path.canonicalize().with_context(|| {
                format!("failed to canonicalize module path {}", path.display())
            })?))
        } else {
            Ok(None)
        }
    }

    fn resolve_relative_path(base: &Path, rel_path: &Path) -> Result<PathBuf> {
        let resolved = base.join(rel_path);
        Self::canonicalize_module_path(resolved)
    }

    fn resolve_absolute_path(abs_path: &Path) -> Result<PathBuf> {
        Self::canonicalize_module_path(abs_path.to_path_buf())
    }

    fn canonicalize_module_path(mut path: PathBuf) -> Result<PathBuf> {
        if path.is_dir() {
            path = path.join("mod.ot");
        }

        if !path.exists() && !path.extension().map_or(false, |ext| ext == "ot") {
            path.set_extension("ot");
        }

        if path.exists() {
            Ok(path.canonicalize().with_context(|| {
                format!("failed to canonicalize module path {}", path.display())
            })?)
        } else {
            bail!("module not found: {}", path.display())
        }
    }
}

/// Tracks module dependencies and detects circular dependencies
#[derive(Debug, Default)]
pub struct DependencyGraph {
    nodes: HashMap<PathBuf, HashSet<PathBuf>>,
    visiting: HashSet<PathBuf>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a dependency edge from `from` to `to`
    pub fn add_dependency(&mut self, from: PathBuf, to: PathBuf) {
        self.nodes
            .entry(from)
            .or_insert_with(HashSet::new)
            .insert(to);
    }

    /// Check for circular dependencies starting from a root node
    pub fn check_circular(&mut self, root: &PathBuf) -> Result<()> {
        self.visiting.clear();
        self.dfs_check(root)
    }

    fn dfs_check(&mut self, node: &PathBuf) -> Result<()> {
        if self.visiting.contains(node) {
            bail!("circular dependency detected involving {}", node.display())
        }

        if let Some(deps) = self.nodes.get(node) {
            let deps_clone: Vec<PathBuf> = deps.iter().cloned().collect();
            self.visiting.insert(node.clone());
            for dep in &deps_clone {
                self.dfs_check(dep)?;
            }
            self.visiting.remove(node);
        }

        Ok(())
    }

    /// Get all dependencies of a module
    pub fn dependencies(&self, module: &PathBuf) -> HashSet<PathBuf> {
        self.nodes.get(module).cloned().unwrap_or_default()
    }

    /// Get all modules in the dependency graph
    pub fn all_modules(&self) -> impl Iterator<Item = &PathBuf> {
        self.nodes.keys()
    }
}

/// Module resolver that handles path resolution and dependency tracking
pub struct ModuleResolver {
    source_dir: PathBuf,
    stdlib_dir: Option<PathBuf>,
    dependency_graph: DependencyGraph,
}

impl ModuleResolver {
    pub fn new(source_dir: PathBuf, stdlib_dir: Option<PathBuf>) -> Self {
        Self {
            source_dir,
            stdlib_dir,
            dependency_graph: DependencyGraph::new(),
        }
    }

    /// Resolve a module path string to a file path
    pub fn resolve(&self, module: &str) -> Result<PathBuf> {
        let module_path = ModulePath::from_string(module, &self.source_dir)?;
        module_path.resolve(&self.source_dir, self.stdlib_dir.as_deref())
    }

    /// Register a dependency relationship
    pub fn add_dependency(&mut self, from: PathBuf, to: PathBuf) {
        self.dependency_graph.add_dependency(from, to);
    }

    /// Check for circular dependencies
    pub fn check_circular(&mut self, root: &PathBuf) -> Result<()> {
        self.dependency_graph.check_circular(root)
    }

    /// Get the dependency graph
    pub fn dependency_graph(&self) -> &DependencyGraph {
        &self.dependency_graph
    }

    /// Set stdlib directory
    pub fn set_stdlib_dir(&mut self, dir: PathBuf) {
        self.stdlib_dir = Some(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_module_path_parsing() {
        let source_dir = PathBuf::from("/tmp");

        // Test stdlib
        let path = ModulePath::from_string("otter:math", &source_dir).unwrap();
        assert!(matches!(path, ModulePath::Stdlib(name) if name == "math"));

        // Test Rust FFI
        let path = ModulePath::from_string("rust:serde_json", &source_dir).unwrap();
        assert!(matches!(path, ModulePath::Rust(name) if name == "serde_json"));

        // Test relative
        let path = ModulePath::from_string("./math", &source_dir).unwrap();
        assert!(matches!(path, ModulePath::Relative(_)));

        // Test absolute
        let path = ModulePath::from_string("/usr/lib/math", &source_dir).unwrap();
        assert!(matches!(path, ModulePath::Absolute(_)));

        // Test unqualified
        let path = ModulePath::from_string("io", &source_dir).unwrap();
        assert!(matches!(path, ModulePath::Unqualified(name) if name == "io"));

        // Explicit stdlib namespace
        let path = ModulePath::from_string("otter:core", &source_dir).unwrap();
        assert!(matches!(path, ModulePath::Stdlib(name) if name == "core"));
    }

    #[test]
    fn test_unqualified_prefers_stdlib() {
        let temp = TempDir::new().unwrap();
        let stdlib_dir = temp.path().join("stdlib");
        fs::create_dir_all(&stdlib_dir).unwrap();

        let io_module = stdlib_dir.join("io.ot");
        fs::write(&io_module, "fn main: pass").unwrap();

        let resolver = ModuleResolver::new(temp.path().to_path_buf(), Some(stdlib_dir));
        let resolved = resolver.resolve("io").unwrap();
        assert_eq!(resolved, io_module.canonicalize().unwrap());

        // Ensure explicit stdlib still works
        let resolved_std = resolver.resolve("otter:io").unwrap();
        assert_eq!(resolved, resolved_std);
    }

    #[test]
    fn test_unqualified_relative_fallback() {
        let temp = TempDir::new().unwrap();
        let source_dir = temp.path().join("src");
        fs::create_dir_all(&source_dir).unwrap();

        let local_module = source_dir.join("utils.ot");
        fs::write(&local_module, "fn main: pass").unwrap();

        let resolver = ModuleResolver::new(source_dir.clone(), None);
        let resolved = resolver.resolve("utils").unwrap();
        assert_eq!(resolved, local_module.canonicalize().unwrap());
    }

    #[test]
    fn test_dependency_graph_no_circular() {
        let mut graph = DependencyGraph::new();
        let a = PathBuf::from("a");
        let b = PathBuf::from("b");
        let c = PathBuf::from("c");

        graph.add_dependency(a.clone(), b.clone());
        graph.add_dependency(b.clone(), c.clone());

        assert!(graph.check_circular(&a).is_ok());
    }

    #[test]
    fn test_dependency_graph_circular() {
        let mut graph = DependencyGraph::new();
        let a = PathBuf::from("a");
        let b = PathBuf::from("b");

        graph.add_dependency(a.clone(), b.clone());
        graph.add_dependency(b.clone(), a.clone());

        assert!(graph.check_circular(&a).is_err());
    }
}
