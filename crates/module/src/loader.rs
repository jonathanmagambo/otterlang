use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::resolver::ModuleResolver;
use ast::nodes::Program;
use lexer::tokenize;
use parser::parse;

/// Represents a loaded module with its exports
#[derive(Debug, Clone)]
pub struct Module {
    pub path: PathBuf,
    pub program: Program,
    pub exports: ModuleExports,
}

/// Tracks what items are exported from a module
#[derive(Debug, Clone, Default)]
pub struct ModuleExports {
    pub functions: Vec<String>,
    pub constants: Vec<String>,
    pub types: Vec<String>,
}

impl ModuleExports {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_function(&mut self, name: String) {
        self.functions.push(name);
    }

    pub fn add_constant(&mut self, name: String) {
        self.constants.push(name);
    }

    pub fn add_type(&mut self, name: String) {
        self.types.push(name);
    }

    pub fn is_exported(&self, name: &str) -> bool {
        self.functions.contains(&name.to_string())
            || self.constants.contains(&name.to_string())
            || self.types.contains(&name.to_string())
    }
}

/// Loads and caches .ot module files
pub struct ModuleLoader {
    cache: HashMap<PathBuf, Module>,
    resolver: ModuleResolver,
}

impl ModuleLoader {
    pub fn new(source_dir: PathBuf, stdlib_dir: Option<PathBuf>) -> Self {
        Self {
            cache: HashMap::new(),
            resolver: ModuleResolver::new(source_dir, stdlib_dir),
        }
    }

    /// Load a module from a path string
    pub fn load(&mut self, module: &str) -> Result<Module> {
        let resolved_path = self.resolver.resolve(module)?;

        if let Some(cached) = self.cache.get(&resolved_path) {
            return Ok(cached.clone());
        }

        let module = self.load_file(&resolved_path)?;
        let path_clone = resolved_path.clone();
        let module_clone = module.clone();
        self.cache.insert(path_clone, module);
        Ok(module_clone)
    }

    /// Load a module from a file path
    pub fn load_file(&mut self, path: &Path) -> Result<Module> {
        let source = fs::read_to_string(path)
            .with_context(|| format!("failed to read module file {}", path.display()))?;

        let tokens = tokenize(&source).map_err(|errors| {
            anyhow::anyhow!(
                "failed to tokenize module {}: {} errors",
                path.display(),
                errors.len()
            )
        })?;

        let program = parse(&tokens).map_err(|errors| {
            anyhow::anyhow!(
                "failed to parse module {}: {} errors",
                path.display(),
                errors.len()
            )
        })?;

        let exports = self.extract_exports(&program);

        Ok(Module {
            path: path.to_path_buf(),
            program,
            exports,
        })
    }

    /// Extract exported items from a parsed program
    fn extract_exports(&self, program: &Program) -> ModuleExports {
        let mut exports = ModuleExports::new();

        for statement in &program.statements {
            match statement {
                ast::nodes::Statement::Function(function) => {
                    if function.public {
                        exports.add_function(function.name.clone());
                    }
                }
                ast::nodes::Statement::Let { name, public, .. } => {
                    if *public {
                        exports.add_constant(name.clone());
                    }
                }
                ast::nodes::Statement::Struct { name, public, .. } => {
                    if *public {
                        exports.add_type(name.clone());
                    }
                }
                ast::nodes::Statement::TypeAlias { name, public, .. } => {
                    if *public {
                        exports.add_type(name.clone());
                    }
                }
                _ => {}
            }
        }

        exports
    }

    /// Get the module resolver
    pub fn resolver(&self) -> &ModuleResolver {
        &self.resolver
    }

    /// Get mutable access to resolver
    pub fn resolver_mut(&mut self) -> &mut ModuleResolver {
        &mut self.resolver
    }

    /// Check if a module is already cached
    pub fn is_cached(&self, path: &PathBuf) -> bool {
        self.cache.contains_key(path)
    }

    /// Clear the module cache
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

impl Default for ModuleLoader {
    fn default() -> Self {
        Self::new(PathBuf::from("."), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_module_loader() {
        let temp_dir = TempDir::new().unwrap();
        let module_path = temp_dir.path().join("test.ot");

        fs::write(&module_path, "fn main:\n    print(\"test\")\n").unwrap();

        let mut loader = ModuleLoader::new(temp_dir.path().to_path_buf(), None);
        let module = loader.load_file(&module_path).unwrap();

        assert_eq!(module.path, module_path);
        assert!(!module.program.statements.is_empty());
    }
}
