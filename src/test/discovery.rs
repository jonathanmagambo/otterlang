use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use glob::glob;

use ast::nodes::{Function, Statement};
use lexer::tokenize;
use parser::parse;

#[derive(Debug, Clone)]
pub struct TestCase {
    pub file_path: PathBuf,
    pub function_name: String,
    pub function: Function,
    pub line_number: usize,
}

pub struct TestDiscovery {
    test_files: Vec<PathBuf>,
}

impl TestDiscovery {
    pub fn new() -> Self {
        Self {
            test_files: Vec::new(),
        }
    }

    pub fn discover_files(&mut self, paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();

        for path in paths {
            if path.is_dir() {
                let pattern = format!("{}/**/*.ot", path.display());
                for file_path in (glob(&pattern)?).flatten() {
                    files.push(file_path);
                }
            } else if path.extension().is_some_and(|ext| ext == "ot") {
                files.push(path.clone());
            }
        }

        files.sort();
        files.dedup();

        self.test_files = files.clone();
        Ok(files)
    }

    pub fn discover_tests_in_file(&self, file_path: &Path) -> Result<Vec<TestCase>> {
        let source = std::fs::read_to_string(file_path)
            .with_context(|| format!("failed to read {}", file_path.display()))?;

        let tokens = match tokenize(&source) {
            Ok(tokens) => tokens,
            Err(_) => return Ok(Vec::new()),
        };

        let program = match parse(&tokens) {
            Ok(program) => program,
            Err(_) => return Ok(Vec::new()),
        };

        let mut tests = Vec::new();

        for (idx, stmt) in program.statements.iter().enumerate() {
            if let Statement::Function(func) = stmt
                && Self::is_test_function(func) {
                    let line_number = Self::estimate_line_number(&source, idx);
                    tests.push(TestCase {
                        file_path: file_path.to_path_buf(),
                        function_name: func.name.clone(),
                        function: func.clone(),
                        line_number,
                    });
                }
        }

        Ok(tests)
    }

    pub fn discover_all_tests(&self) -> Result<Vec<TestCase>> {
        let mut all_tests = Vec::new();

        for file_path in &self.test_files {
            match self.discover_tests_in_file(file_path) {
                Ok(tests) => all_tests.extend(tests),
                Err(e) => {
                    eprintln!("Warning: Failed to discover tests in {}: {}", file_path.display(), e);
                }
            }
        }

        Ok(all_tests)
    }

    fn is_test_function(func: &Function) -> bool {
        func.name.starts_with("test_") || (func.public && func.name.starts_with("test"))
    }

    fn estimate_line_number(source: &str, statement_index: usize) -> usize {
        let chars_before = source
            .chars()
            .take(statement_index * 50)
            .filter(|&c| c == '\n')
            .count();
        chars_before + 1
    }
}

impl Default for TestDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

