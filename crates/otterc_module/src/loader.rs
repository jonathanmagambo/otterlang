use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::resolver::ModuleResolver;
use otterc_ast::nodes::{Program, Statement};
use otterc_lexer::tokenize;
use otterc_parser::parse;

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
            match statement.as_ref() {
                Statement::Function(function) => {
                    if function.as_ref().public {
                        exports.add_function(function.as_ref().name.clone());
                    }
                }
                Statement::Let { name, public, .. } => {
                    if *public {
                        exports.add_constant(name.as_ref().clone());
                    }
                }
                Statement::Struct { name, public, .. }
                | Statement::Enum { name, public, .. }
                | Statement::TypeAlias { name, public, .. } => {
                    if *public {
                        exports.add_type(name.clone());
                    }
                }
                _ => {}
            }
        }

        exports
    }

    /// Resolve re-exports for a module after all modules are loaded
    /// This processes `pub use` statements and adds re-exported items to the module's exports
    pub fn resolve_re_exports(
        &self,
        module: &mut Module,
        all_modules: &HashMap<PathBuf, Module>,
    ) -> Result<()> {
        use Statement;

        for statement in &module.program.statements {
            if let Statement::PubUse {
                module: source_module,
                item,
                alias,
            } = statement.as_ref()
            {
                // Resolve the source module path
                let source_path = self.resolver.resolve(source_module)?;

                // Get the source module
                let source_module_data = all_modules.get(&source_path).ok_or_else(|| {
                    anyhow::anyhow!(
                        "re-export source module not found: {} (resolved to {})",
                        source_module,
                        source_path.display()
                    )
                })?;

                if let Some(item_name) = item {
                    // Re-export specific item
                    let export_name = alias.as_ref().unwrap_or(item_name);

                    // Check if the item exists in the source module
                    if source_module_data.exports.functions.contains(item_name) {
                        module.exports.add_function(export_name.clone());
                    } else if source_module_data.exports.constants.contains(item_name) {
                        module.exports.add_constant(export_name.clone());
                    } else if source_module_data.exports.types.contains(item_name) {
                        module.exports.add_type(export_name.clone());
                    } else {
                        // Item not found in source module exports
                        return Err(anyhow::anyhow!(
                            "cannot re-export '{}' from '{}': item not found or not public",
                            item_name,
                            source_module
                        ));
                    }
                } else {
                    // Re-export all public items from the module
                    for func in &source_module_data.exports.functions {
                        module.exports.add_function(func.clone());
                    }
                    for constant in &source_module_data.exports.constants {
                        module.exports.add_constant(constant.clone());
                    }
                    for ty in &source_module_data.exports.types {
                        module.exports.add_type(ty.clone());
                    }
                }
            }
        }

        Ok(())
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
