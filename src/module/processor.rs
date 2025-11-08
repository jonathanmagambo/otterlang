use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::module::{Module, ModuleLoader, ModulePath, ModuleResolver};
use ast::nodes::{Program, Statement};

/// Processes module imports and loads dependencies
pub struct ModuleProcessor {
    loader: ModuleLoader,
    source_dir: PathBuf,
    stdlib_dir: Option<PathBuf>,
    loaded_modules: HashMap<PathBuf, Module>,
}

impl ModuleProcessor {
    pub fn new(source_dir: PathBuf, stdlib_dir: Option<PathBuf>) -> Self {
        let loader = ModuleLoader::new(source_dir.clone(), stdlib_dir.clone());
        Self {
            loader,
            source_dir,
            stdlib_dir,
            loaded_modules: HashMap::new(),
        }
    }

    /// Process all `use` statements in a program and load dependencies
    pub fn process_imports(&mut self, program: &Program) -> Result<Vec<PathBuf>> {
        let mut dependencies = Vec::new();
        let mut rust_imports = Vec::new();

        for statement in &program.statements {
            if let Statement::Use { module, .. } = statement {
                let module_path = ModulePath::from_string(module, &self.source_dir)?;

                match module_path {
                    ModulePath::Rust(_) => {
                        // Rust FFI imports are handled separately
                        rust_imports.push(module.clone());
                    }
                    ModulePath::Stdlib(_) => {
                        // Load stdlib module
                        let resolved = self.loader.resolver().resolve(module)?;
                        if !self.loaded_modules.contains_key(&resolved) {
                            let module = self.loader.load_file(&resolved)?;
                            dependencies.push(resolved.clone());
                            self.loaded_modules.insert(resolved, module);
                        }
                    }
                    ModulePath::Relative(_) | ModulePath::Absolute(_) => {
                        // Load relative/absolute module
                        let resolved = self.loader.resolver().resolve(module)?;
                        if !self.loaded_modules.contains_key(&resolved) {
                            // Check for circular dependencies
                            self.loader
                                .resolver_mut()
                                .add_dependency(self.source_dir.clone(), resolved.clone());
                            self.loader
                                .resolver_mut()
                                .check_circular(&self.source_dir)?;

                            let module = self.loader.load_file(&resolved)?;
                            dependencies.push(resolved.clone());
                            self.loaded_modules.insert(resolved.clone(), module);

                            // Recursively process imports in the loaded module
                            let module_deps = self.process_module_imports(&resolved)?;
                            dependencies.extend(module_deps);
                        }
                    }
                }
            }
        }

        // Note: Rust imports are returned separately as they're handled by FFI system
        Ok(dependencies)
    }

    /// Process imports for a specific module file
    fn process_module_imports(&mut self, module_path: &PathBuf) -> Result<Vec<PathBuf>> {
        // Get module statements without borrowing self mutably
        let module_statements = {
            let module = self
                .loaded_modules
                .get(module_path)
                .ok_or_else(|| anyhow::anyhow!("module not loaded: {}", module_path.display()))?;
            module.program.statements.clone()
        };

        let mut dependencies = Vec::new();

        for statement in &module_statements {
            if let Statement::Use {
                module: module_name,
                ..
            } = statement
            {
                let module_dir = module_path.parent().unwrap_or(Path::new("."));
                let module_path_enum = ModulePath::from_string(module_name, module_dir)?;

                match module_path_enum {
                    ModulePath::Rust(_) => {
                        // Skip Rust imports
                    }
                    ModulePath::Stdlib(_) => {
                        let resolved = self.loader.resolver().resolve(module_name)?;
                        if !self.loaded_modules.contains_key(&resolved) {
                            let module = self.loader.load_file(&resolved)?;
                            dependencies.push(resolved.clone());
                            self.loaded_modules.insert(resolved.clone(), module);
                        }
                    }
                    ModulePath::Relative(_) | ModulePath::Absolute(_) => {
                        // Resolve relative to the current module's directory
                        let module_dir = module_path.parent().unwrap_or(Path::new("."));
                        let resolver =
                            ModuleResolver::new(module_dir.to_path_buf(), self.stdlib_dir.clone());
                        let resolved = resolver.resolve(module_name)?;

                        if !self.loaded_modules.contains_key(&resolved) {
                            // Check for circular dependencies
                            self.loader
                                .resolver_mut()
                                .add_dependency(module_path.clone(), resolved.clone());
                            self.loader.resolver_mut().check_circular(module_path)?;

                            let module = self.loader.load_file(&resolved)?;
                            dependencies.push(resolved.clone());
                            self.loaded_modules.insert(resolved.clone(), module);

                            // Recursively process
                            let module_deps = self.process_module_imports(&resolved)?;
                            dependencies.extend(module_deps);
                        }
                    }
                }
            }
        }

        Ok(dependencies)
    }

    /// Get all loaded module dependencies
    pub fn dependencies(&self) -> Vec<PathBuf> {
        self.loaded_modules.keys().cloned().collect()
    }

    /// Get a loaded module by path
    pub fn get_module(&self, path: &PathBuf) -> Option<&Module> {
        self.loaded_modules.get(path)
    }

    /// Set stdlib directory
    pub fn set_stdlib_dir(&mut self, dir: PathBuf) {
        self.loader.resolver_mut().set_stdlib_dir(dir.clone());
        self.stdlib_dir = Some(dir);
    }
}

impl Default for ModuleProcessor {
    fn default() -> Self {
        Self::new(PathBuf::from("."), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_process_imports() {
        let temp_dir = TempDir::new().unwrap();
        let source_dir = temp_dir.path().join("src");
        fs::create_dir_all(&source_dir).unwrap();

        let main_file = source_dir.join("main.ot");
        let math_file = source_dir.join("math.ot");

        fs::write(&main_file, "use ./math\nfn main:\n    print(\"test\")\n").unwrap();
        fs::write(
            &math_file,
            "pub fn add(a: f64, b: f64) -> f64:\n    return a + b\n",
        )
        .unwrap();

        let tokens = lexer::tokenize(&fs::read_to_string(&main_file).unwrap()).unwrap();
        let program = crate::parser::parse(&tokens).unwrap();

        let mut processor = ModuleProcessor::new(source_dir.clone(), None);
        let deps = processor.process_imports(&program).unwrap();

        assert_eq!(deps.len(), 1);
        assert!(deps.contains(&math_file.canonicalize().unwrap()));
    }
}
