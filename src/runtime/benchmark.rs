//! Built-in benchmarking utilities for performance testing

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Statistical summary of benchmark results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkStats {
    /// Number of iterations
    pub iterations: usize,

    /// Mean execution time
    pub mean: Duration,

    /// Median execution time
    pub median: Duration,

    /// Standard deviation
    pub std_dev: Duration,

    /// Minimum execution time
    pub min: Duration,

    /// Maximum execution time
    pub max: Duration,

    /// Percentiles (p50, p90, p95, p99)
    pub percentiles: HashMap<u8, Duration>,
}

impl BenchmarkStats {
    /// Calculate statistics from a list of durations
    pub fn from_durations(mut durations: Vec<Duration>) -> Self {
        if durations.is_empty() {
            return Self::default();
        }

        durations.sort();
        let iterations = durations.len();

        let min = durations[0];
        let max = durations[iterations - 1];

        // Calculate mean
        let total_nanos: u128 = durations.iter().map(|d| d.as_nanos()).sum();
        let mean_nanos = total_nanos / iterations as u128;
        let mean = Duration::from_nanos(mean_nanos as u64);

        // Calculate median
        let median = if iterations.is_multiple_of(2) {
            let mid1 = durations[iterations / 2 - 1];
            let mid2 = durations[iterations / 2];
            Duration::from_nanos(((mid1.as_nanos() + mid2.as_nanos()) / 2) as u64)
        } else {
            durations[iterations / 2]
        };

        // Calculate standard deviation
        let variance: f64 = durations
            .iter()
            .map(|d| {
                let diff = d.as_nanos() as f64 - mean_nanos as f64;
                diff * diff
            })
            .sum::<f64>()
            / iterations as f64;
        let std_dev = Duration::from_nanos(variance.sqrt() as u64);

        // Calculate percentiles
        let mut percentiles = HashMap::new();
        percentiles.insert(50, percentile(&durations, 50.0));
        percentiles.insert(90, percentile(&durations, 90.0));
        percentiles.insert(95, percentile(&durations, 95.0));
        percentiles.insert(99, percentile(&durations, 99.0));

        Self {
            iterations,
            mean,
            median,
            std_dev,
            min,
            max,
            percentiles,
        }
    }

    /// Format statistics as a human-readable string
    pub fn format(&self) -> String {
        format!(
            "Iterations: {}\n\
             Mean:       {:?}\n\
             Median:     {:?}\n\
             Std Dev:    {:?}\n\
             Min:        {:?}\n\
             Max:        {:?}\n\
             P50:        {:?}\n\
             P90:        {:?}\n\
             P95:        {:?}\n\
             P99:        {:?}",
            self.iterations,
            self.mean,
            self.median,
            self.std_dev,
            self.min,
            self.max,
            self.percentiles.get(&50).unwrap(),
            self.percentiles.get(&90).unwrap(),
            self.percentiles.get(&95).unwrap(),
            self.percentiles.get(&99).unwrap(),
        )
    }
}

impl Default for BenchmarkStats {
    fn default() -> Self {
        Self {
            iterations: 0,
            mean: Duration::ZERO,
            median: Duration::ZERO,
            std_dev: Duration::ZERO,
            min: Duration::ZERO,
            max: Duration::ZERO,
            percentiles: HashMap::new(),
        }
    }
}

/// Benchmark configuration
#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    /// Number of warmup iterations (not measured)
    pub warmup_iterations: usize,

    /// Number of measured iterations
    pub iterations: usize,

    /// Minimum duration to run (will override iterations if needed)
    pub min_duration: Option<Duration>,

    /// Maximum duration to run (will stop early if exceeded)
    pub max_duration: Option<Duration>,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            warmup_iterations: 10,
            iterations: 100,
            min_duration: None,
            max_duration: None,
        }
    }
}

/// Benchmark result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    /// Name of the benchmark
    pub name: String,

    /// Statistical summary
    pub stats: BenchmarkStats,

    /// Timestamp when benchmark was run
    pub timestamp_ms: u64,
}

impl BenchmarkResult {
    /// Export to JSON
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Export to CSV row
    pub fn to_csv_row(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{},{},{}",
            self.name,
            self.stats.iterations,
            self.stats.mean.as_nanos(),
            self.stats.median.as_nanos(),
            self.stats.std_dev.as_nanos(),
            self.stats.min.as_nanos(),
            self.stats.max.as_nanos(),
            self.stats.percentiles.get(&90).unwrap().as_nanos(),
            self.stats.percentiles.get(&95).unwrap().as_nanos(),
            self.stats.percentiles.get(&99).unwrap().as_nanos(),
        )
    }

    /// CSV header
    pub fn csv_header() -> String {
        "name,iterations,mean_ns,median_ns,std_dev_ns,min_ns,max_ns,p90_ns,p95_ns,p99_ns"
            .to_string()
    }
}

/// Benchmark runner
pub struct Benchmark {
    name: String,
    config: BenchmarkConfig,
}

impl Benchmark {
    /// Create a new benchmark
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            config: BenchmarkConfig::default(),
        }
    }

    /// Set configuration
    pub fn with_config(mut self, config: BenchmarkConfig) -> Self {
        self.config = config;
        self
    }

    /// Set warmup iterations
    pub fn warmup(mut self, iterations: usize) -> Self {
        self.config.warmup_iterations = iterations;
        self
    }

    /// Set measured iterations
    pub fn iterations(mut self, iterations: usize) -> Self {
        self.config.iterations = iterations;
        self
    }

    /// Run the benchmark
    pub fn run<F>(self, mut f: F) -> BenchmarkResult
    where
        F: FnMut(),
    {
        // Warmup phase
        for _ in 0..self.config.warmup_iterations {
            f();
        }

        // Measurement phase
        let mut durations = Vec::with_capacity(self.config.iterations);
        let start_time = Instant::now();

        for _ in 0..self.config.iterations {
            let iter_start = Instant::now();
            f();
            let iter_duration = iter_start.elapsed();
            durations.push(iter_duration);

            // Check max duration
            if self
                .config
                .max_duration
                .is_some_and(|max_dur| start_time.elapsed() > max_dur)
            {
                break;
            }
        }

        // Check min duration - run more iterations if needed
        if let Some(min_dur) = self.config.min_duration {
            while start_time.elapsed() < min_dur {
                let iter_start = Instant::now();
                f();
                let iter_duration = iter_start.elapsed();
                durations.push(iter_duration);
            }
        }

        let stats = BenchmarkStats::from_durations(durations);

        BenchmarkResult {
            name: self.name,
            stats,
            timestamp_ms: current_time_ms(),
        }
    }

    /// Run and print results
    pub fn run_and_print<F>(self, f: F)
    where
        F: FnMut(),
    {
        let name = self.name.clone();
        let result = self.run(f);
        #[expect(clippy::print_stdout, reason = "TODO: Use robust logging")]
        {
            println!("Benchmark: {}", name);
            println!("{}", result.stats.format());
        }
    }
}

/// Compare two benchmark results
pub fn compare_benchmarks(baseline: &BenchmarkResult, current: &BenchmarkResult) -> String {
    let baseline_mean = baseline.stats.mean.as_nanos() as f64;
    let current_mean = current.stats.mean.as_nanos() as f64;

    let diff_percent = ((current_mean - baseline_mean) / baseline_mean) * 100.0;

    let status = if diff_percent.abs() < 5.0 {
        "~"
    } else if diff_percent < 0.0 {
        "✓ FASTER"
    } else {
        "✗ SLOWER"
    };

    format!(
        "{} vs {}: {:.2}% {} (baseline: {:?}, current: {:?})",
        current.name,
        baseline.name,
        diff_percent.abs(),
        status,
        baseline.stats.mean,
        current.stats.mean
    )
}

/// Calculate percentile from sorted durations
fn percentile(sorted_durations: &[Duration], p: f64) -> Duration {
    if sorted_durations.is_empty() {
        return Duration::ZERO;
    }

    let index = ((p / 100.0) * (sorted_durations.len() - 1) as f64).round() as usize;
    sorted_durations[index.min(sorted_durations.len() - 1)]
}

/// Get current time in milliseconds
fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_benchmark_stats() {
        let durations = vec![
            Duration::from_millis(10),
            Duration::from_millis(20),
            Duration::from_millis(15),
            Duration::from_millis(25),
            Duration::from_millis(18),
        ];

        let stats = BenchmarkStats::from_durations(durations);
        assert_eq!(stats.iterations, 5);
        assert_eq!(stats.min, Duration::from_millis(10));
        assert_eq!(stats.max, Duration::from_millis(25));
    }

    #[test]
    fn test_benchmark_run() {
        let result = Benchmark::new("test").warmup(2).iterations(10).run(|| {
            // Simulate some work
            std::thread::sleep(Duration::from_micros(100));
        });

        assert_eq!(result.stats.iterations, 10);
        assert!(result.stats.mean > Duration::ZERO);
    }

    #[test]
    fn test_percentile() {
        let durations = vec![
            Duration::from_millis(1),
            Duration::from_millis(2),
            Duration::from_millis(3),
            Duration::from_millis(4),
            Duration::from_millis(5),
        ];

        let p50 = percentile(&durations, 50.0);
        assert_eq!(p50, Duration::from_millis(3));
    }
}
