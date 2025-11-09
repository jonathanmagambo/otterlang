use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::{Module, ModuleLoader, ModulePath, ModuleResolver};
use ast::nodes::{Program, Statement};
const DEFAULT_MODULES: &[&str] = &["otter:core"];

/// Processes module imports and loads dependencies
pub struct ModuleProcessor {
    loader: ModuleLoader,
    source_dir: PathBuf,
    stdlib_dir: Option<PathBuf>,
    loaded_modules: HashMap<PathBuf, Module>,
}

impl ModuleProcessor {
    pub fn new(source_dir: PathBuf, stdlib_dir: Option<PathBuf>) -> Self {
        let normalized_stdlib = stdlib_dir.map(|dir| dir.canonicalize().unwrap_or(dir));
        let loader = ModuleLoader::new(source_dir.clone(), normalized_stdlib.clone());
        Self {
            loader,
            source_dir,
            stdlib_dir: normalized_stdlib,
            loaded_modules: HashMap::new(),
        }
    }

    /// Process all `use` statements in a program and load dependencies
    pub fn process_imports(&mut self, program: &Program) -> Result<Vec<PathBuf>> {
        let mut dependencies = Vec::new();
        let mut rust_imports = Vec::new();

        self.load_default_modules(&mut dependencies)?;

        for statement in &program.statements {
            if let Statement::Use { imports } = statement {
                for import in imports {
                    let module = &import.module;
                    let module_path = ModulePath::from_string(module, &self.source_dir)?;

                    match module_path {
                        ModulePath::Rust(_) => {
                            rust_imports.push(module.clone());
                        }
                        ModulePath::Stdlib(_) => {
                            let resolved = {
                                let resolver = self.loader.resolver();
                                resolver.resolve(module)?
                            };
                            self.load_stdlib_dependency(resolved, &mut dependencies)?;
                        }
                        ModulePath::Relative(_) | ModulePath::Absolute(_) => {
                            let source_dir = self.source_dir.clone();
                            let resolved = {
                                let resolver = self.loader.resolver();
                                resolver.resolve(module)?
                            };
                            self.load_local_dependency(&source_dir, resolved, &mut dependencies)?;
                        }
                        ModulePath::Unqualified(_) => {
                            let source_dir = self.source_dir.clone();
                            let resolved = {
                                let resolver = self.loader.resolver();
                                resolver.resolve(module)?
                            };
                            if self.is_stdlib_path(&resolved) {
                                self.load_stdlib_dependency(resolved, &mut dependencies)?;
                            } else {
                                self.load_local_dependency(
                                    &source_dir,
                                    resolved,
                                    &mut dependencies,
                                )?;
                            }
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

        self.load_default_modules(&mut dependencies)?;

        for statement in &module_statements {
            if let Statement::Use { imports } = statement {
                for import in imports {
                    let module = &import.module;
                    let module_dir = module_path.parent().unwrap_or(Path::new("."));
                    let module_path_enum = ModulePath::from_string(module, module_dir)?;

                    match module_path_enum {
                        ModulePath::Rust(_) => {}
                        ModulePath::Stdlib(_) => {
                            let resolver = ModuleResolver::new(
                                module_dir.to_path_buf(),
                                self.stdlib_dir.clone(),
                            );
                            let resolved = resolver.resolve(module)?;
                            self.load_stdlib_dependency(resolved, &mut dependencies)?;
                        }
                        ModulePath::Relative(_) | ModulePath::Absolute(_) => {
                            let resolver = ModuleResolver::new(
                                module_dir.to_path_buf(),
                                self.stdlib_dir.clone(),
                            );
                            let resolved = resolver.resolve(module)?;
                            self.load_local_dependency(module_path, resolved, &mut dependencies)?;
                        }
                        ModulePath::Unqualified(_) => {
                            let resolver = ModuleResolver::new(
                                module_dir.to_path_buf(),
                                self.stdlib_dir.clone(),
                            );
                            let resolved = resolver.resolve(module)?;
                            if self.is_stdlib_path(&resolved) {
                                self.load_stdlib_dependency(resolved, &mut dependencies)?;
                            } else {
                                self.load_local_dependency(
                                    module_path,
                                    resolved,
                                    &mut dependencies,
                                )?;
                            }
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

    /// Iterate over all loaded modules
    pub fn modules(&self) -> impl Iterator<Item = &Module> {
        self.loaded_modules.values()
    }

    /// Set stdlib directory
    pub fn set_stdlib_dir(&mut self, dir: PathBuf) {
        let normalized = dir.canonicalize().unwrap_or(dir);
        self.loader
            .resolver_mut()
            .set_stdlib_dir(normalized.clone());
        self.stdlib_dir = Some(normalized);
    }
}

impl ModuleProcessor {
    fn load_default_modules(&mut self, dependencies: &mut Vec<PathBuf>) -> Result<()> {
        if self.stdlib_dir.is_none() {
            return Ok(());
        }

        for module in DEFAULT_MODULES {
            let resolved = {
                let resolver = self.loader.resolver();
                resolver.resolve(module)?
            };

            if self.is_stdlib_path(&resolved) {
                self.load_stdlib_dependency(resolved, dependencies)?;
            } else {
                self.load_local_dependency(Path::new("."), resolved, dependencies)?;
            }
        }

        Ok(())
    }

    fn load_stdlib_dependency(
        &mut self,
        resolved: PathBuf,
        dependencies: &mut Vec<PathBuf>,
    ) -> Result<()> {
        if !self.loaded_modules.contains_key(&resolved) {
            let module = self.loader.load_file(&resolved)?;
            dependencies.push(resolved.clone());
            self.loaded_modules.insert(resolved, module);
        }
        Ok(())
    }

    fn load_local_dependency(
        &mut self,
        owner: &Path,
        resolved: PathBuf,
        dependencies: &mut Vec<PathBuf>,
    ) -> Result<()> {
        if !self.loaded_modules.contains_key(&resolved) {
            let owner_path = owner.to_path_buf();
            self.loader
                .resolver_mut()
                .add_dependency(owner_path.clone(), resolved.clone());
            self.loader.resolver_mut().check_circular(&owner_path)?;

            let module = self.loader.load_file(&resolved)?;
            dependencies.push(resolved.clone());
            self.loaded_modules.insert(resolved.clone(), module);

            let module_deps = self.process_module_imports(&resolved)?;
            dependencies.extend(module_deps);
        }
        Ok(())
    }

    fn is_stdlib_path(&self, path: &Path) -> bool {
        self.stdlib_dir
            .as_ref()
            .map(|dir| path.starts_with(dir))
            .unwrap_or(false)
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
        let program = parser::parse(&tokens).unwrap();

        let mut processor = ModuleProcessor::new(source_dir.clone(), None);
        let deps = processor.process_imports(&program).unwrap();

        assert_eq!(deps.len(), 1);
        assert!(deps.contains(&math_file.canonicalize().unwrap()));
    }
}
