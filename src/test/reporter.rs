#![expect(
    clippy::print_stdout,
    reason = "Printing to stdout is acceptable in tests"
)]

use colored::*;
use std::time::Duration;

use crate::test::TestCase;

#[derive(Debug, Clone)]
pub enum TestResult {
    Passed {
        duration: Duration,
        output: String,
    },
    Failed {
        error: String,
        duration: Duration,
        output: String,
        span: Option<(usize, usize)>,
    },
    Skipped {
        reason: String,
    },
}

pub struct TestReporter {
    verbose: bool,
    results: Vec<(TestCase, TestResult)>,
    start_time: std::time::Instant,
}

impl TestReporter {
    pub fn new(verbose: bool) -> Self {
        Self {
            verbose,
            results: Vec::new(),
            start_time: std::time::Instant::now(),
        }
    }

    pub fn record_result(&mut self, test: TestCase, result: TestResult) {
        self.results.push((test, result));
    }

    pub fn print_result(&self, test: &TestCase, result: &TestResult) {
        match result {
            TestResult::Passed { duration, output } => {
                print!("{}", "✓".green());
                println!(
                    " {} ({:.2}ms)",
                    test.function_name,
                    duration.as_secs_f64() * 1000.0
                );
                if self.verbose && !output.is_empty() {
                    for line in output.lines() {
                        println!("  {}", line);
                    }
                }
            }
            TestResult::Failed {
                error,
                duration,
                output,
                span,
            } => {
                print!("{}", "✗".red());
                println!(
                    " {} ({:.2}ms)",
                    test.function_name,
                    duration.as_secs_f64() * 1000.0
                );
                println!("  {} {}", "Error:".red().bold(), error);

                if let Some((start, _end)) = span {
                    println!(
                        "  {} {}:{}",
                        "Location:".yellow(),
                        test.file_path.display(),
                        start
                    );
                }

                if !output.is_empty() {
                    println!("  {}:", "Output:".yellow());
                    for line in output.lines() {
                        println!("    {}", line);
                    }
                }
            }
            TestResult::Skipped { reason } => {
                print!("{}", "⊘".yellow());
                println!(" {} ({})", test.function_name, reason);
            }
        }
    }

    pub fn print_summary(&self) {
        let total_duration = self.start_time.elapsed();
        let passed = self
            .results
            .iter()
            .filter(|(_, r)| matches!(r, TestResult::Passed { .. }))
            .count();
        let failed = self
            .results
            .iter()
            .filter(|(_, r)| matches!(r, TestResult::Failed { .. }))
            .count();
        let skipped = self
            .results
            .iter()
            .filter(|(_, r)| matches!(r, TestResult::Skipped { .. }))
            .count();
        let total = self.results.len();

        println!("\n{}", "Test Summary".bold());
        println!("  Total:   {}", total);
        println!("  {} {}", "Passed:".green(), passed);
        println!("  {} {}", "Failed:".red(), failed);
        if skipped > 0 {
            println!("  {} {}", "Skipped:".yellow(), skipped);
        }
        println!("  Time:    {:.2}s", total_duration.as_secs_f64());

        if failed > 0 {
            println!("\n{}", "Failed Tests:".red().bold());
            for (test, result) in &self.results {
                if matches!(result, TestResult::Failed { .. })
                    && let TestResult::Failed { error, .. } = result
                {
                    println!("  {} - {}", test.function_name.red(), error);
                }
            }
        }
    }

    pub fn has_failures(&self) -> bool {
        self.results
            .iter()
            .any(|(_, r)| matches!(r, TestResult::Failed { .. }))
    }
}
