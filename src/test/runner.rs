use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use crate::cli::CompilationSettings;
use crate::test::{TestCase, TestResult};

pub struct TestRunner {
    settings: CompilationSettings,
    update_snapshots: bool,
}

impl TestRunner {
    pub fn new(settings: CompilationSettings, update_snapshots: bool) -> Self {
        Self {
            settings,
            update_snapshots,
        }
    }

    pub fn run_test(&self, test: &TestCase) -> TestResult {
        let start = Instant::now();

        let compile_result = self.compile_test_file(&test.file_path);
        if let Err(e) = compile_result {
            return TestResult::Failed {
                error: format!("Compilation failed: {}", e),
                duration: start.elapsed(),
                output: String::new(),
                span: Some((test.line_number, test.line_number)),
            };
        }

        let binary_path = compile_result.unwrap();

        let mut command = Command::new(&binary_path);
        self.settings.apply_runtime_env(&mut command);
        command.env("OTTER_TEST_MODE", "1");
        command.env("OTTER_TEST_NAME", &test.function_name);
        if self.update_snapshots {
            command.env("OTTER_UPDATE_SNAPSHOTS", "1");
        }

        let output = command.output();
        let duration = start.elapsed();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let combined_output = if stderr.is_empty() {
                    stdout
                } else {
                    format!("{}\n{}", stdout, stderr)
                };

                if output.status.success() {
                    TestResult::Passed {
                        duration,
                        output: combined_output,
                    }
                } else {
                    TestResult::Failed {
                        error: format!(
                            "Test failed with exit code {}",
                            output.status.code().unwrap_or(-1)
                        ),
                        duration,
                        output: combined_output,
                        span: Some((test.line_number, test.line_number)),
                    }
                }
            }
            Err(e) => TestResult::Failed {
                error: format!("Failed to execute test: {}", e),
                duration,
                output: String::new(),
                span: Some((test.line_number, test.line_number)),
            },
        }
    }

    fn compile_test_file(&self, file_path: &Path) -> Result<std::path::PathBuf> {
        use crate::cli::{compile_pipeline, read_source};

        let source = read_source(file_path)?;
        let stage = compile_pipeline(file_path, &source, &self.settings)
            .with_context(|| format!("failed to compile test file {}", file_path.display()))?;

        let binary_path = match &stage.result {
            crate::cli::CompilationResult::CacheHit(entry) => entry.binary_path.clone(),
            crate::cli::CompilationResult::Compiled { artifact, .. } => artifact.binary.clone(),
            crate::cli::CompilationResult::Checked => {
                unreachable!("check_only should be false for tests")
            }
        };

        Ok(binary_path)
    }
}
