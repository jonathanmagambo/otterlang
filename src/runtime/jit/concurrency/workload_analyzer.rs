use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::scheduler::SchedulerStats;
use super::thread_pool::ThreadPoolStats;

/// Analyzes workload characteristics to guide optimization
pub struct WorkloadAnalyzer {
    task_profiles: HashMap<String, TaskProfile>,
    last_analysis: Instant,
    analysis_interval: Duration,
}

#[derive(Debug, Clone)]
struct TaskProfile {
    count: u64,
    avg_duration: Duration,
    max_duration: Duration,
    min_duration: Duration,
    last_seen: Instant,
    is_cpu_bound: bool,
    is_io_bound: bool,
}

impl Default for WorkloadAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkloadAnalyzer {
    pub fn new() -> Self {
        Self {
            task_profiles: HashMap::new(),
            last_analysis: Instant::now(),
            analysis_interval: Duration::from_secs(1),
        }
    }

    pub fn record_task(&mut self, task_type: &str, duration: Duration, is_cpu_bound: bool) {
        let profile = self
            .task_profiles
            .entry(task_type.to_string())
            .or_insert_with(|| TaskProfile {
                count: 0,
                avg_duration: Duration::ZERO,
                max_duration: Duration::ZERO,
                min_duration: Duration::MAX,
                last_seen: Instant::now(),
                is_cpu_bound: false,
                is_io_bound: false,
            });

        profile.count += 1;
        profile.last_seen = Instant::now();

        // Update average duration
        let total_nanos =
            profile.avg_duration.as_nanos() * (profile.count - 1) as u128 + duration.as_nanos();
        profile.avg_duration = Duration::from_nanos(
            (total_nanos / profile.count as u128).min(u64::MAX as u128) as u64,
        );

        if duration > profile.max_duration {
            profile.max_duration = duration;
        }
        if duration < profile.min_duration {
            profile.min_duration = duration;
        }

        profile.is_cpu_bound = is_cpu_bound;
        profile.is_io_bound = !is_cpu_bound;
    }

    pub fn analyze_workload(
        &mut self,
        scheduler_stats: &SchedulerStats,
        pool_stats: &ThreadPoolStats,
    ) -> WorkloadAnalysis {
        let now = Instant::now();
        if now.duration_since(self.last_analysis) < self.analysis_interval {
            return WorkloadAnalysis::default();
        }

        // Analyze task characteristics
        let total_tasks = scheduler_stats.pending_tasks + scheduler_stats.running_tasks;
        let cpu_bound_ratio = self.calculate_cpu_bound_ratio();
        let avg_task_duration = self.calculate_avg_task_duration();

        // Determine optimal thread count
        let optimal_threads =
            self.calculate_optimal_threads(total_tasks, cpu_bound_ratio, pool_stats.total_threads);

        // Detect workload patterns
        let is_mostly_cpu_bound = cpu_bound_ratio > 0.7;
        let is_mostly_io_bound = cpu_bound_ratio < 0.3;
        let is_mixed = !is_mostly_cpu_bound && !is_mostly_io_bound;

        self.last_analysis = now;

        WorkloadAnalysis {
            optimal_thread_count: optimal_threads,
            cpu_bound_ratio,
            is_mostly_cpu_bound,
            is_mostly_io_bound,
            is_mixed,
            avg_task_duration,
            recommendation: self.generate_recommendation(
                optimal_threads,
                is_mostly_cpu_bound,
                is_mostly_io_bound,
            ),
        }
    }

    fn calculate_cpu_bound_ratio(&self) -> f64 {
        if self.task_profiles.is_empty() {
            return 0.5; // Default: mixed
        }

        let cpu_bound_count: u64 = self
            .task_profiles
            .values()
            .filter(|p| p.is_cpu_bound)
            .map(|p| p.count)
            .sum();

        let total_count: u64 = self.task_profiles.values().map(|p| p.count).sum();

        if total_count == 0 {
            return 0.5;
        }

        cpu_bound_count as f64 / total_count as f64
    }

    fn calculate_avg_task_duration(&self) -> Duration {
        if self.task_profiles.is_empty() {
            return Duration::from_millis(10);
        }

        let total_duration: u128 = self
            .task_profiles
            .values()
            .map(|p| p.avg_duration.as_nanos() * p.count as u128)
            .sum();

        let total_count: u64 = self.task_profiles.values().map(|p| p.count).sum();

        if total_count == 0 {
            return Duration::from_millis(10);
        }

        Duration::from_nanos((total_duration / total_count as u128).min(u64::MAX as u128) as u64)
    }

    fn calculate_optimal_threads(
        &self,
        pending_tasks: usize,
        cpu_bound_ratio: f64,
        current_threads: usize,
    ) -> usize {
        let cpu_count = sysinfo::System::new().cpus().len().max(1);

        // Base threads on CPU count
        let mut optimal = cpu_count;

        // Adjust based on workload type
        if cpu_bound_ratio > 0.7 {
            // CPU-bound: use more threads
            optimal = cpu_count * 2;
        } else if cpu_bound_ratio < 0.3 {
            // I/O-bound: use fewer threads (but more than CPU count for parallelism)
            optimal = cpu_count.max(4);
        }

        // Adjust based on pending tasks
        if pending_tasks > current_threads * 2 {
            optimal = optimal.max(pending_tasks / 2);
        }

        optimal.min(cpu_count * 4).max(1)
    }

    fn generate_recommendation(
        &self,
        optimal_threads: usize,
        is_cpu_bound: bool,
        is_io_bound: bool,
    ) -> String {
        if is_cpu_bound {
            format!(
                "CPU-bound workload detected. Use {} threads for optimal performance.",
                optimal_threads
            )
        } else if is_io_bound {
            format!(
                "I/O-bound workload detected. Use {} threads to handle concurrent I/O.",
                optimal_threads
            )
        } else {
            format!(
                "Mixed workload detected. Use {} threads for balanced performance.",
                optimal_threads
            )
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkloadAnalysis {
    pub optimal_thread_count: usize,
    pub cpu_bound_ratio: f64,
    pub is_mostly_cpu_bound: bool,
    pub is_mostly_io_bound: bool,
    pub is_mixed: bool,
    pub avg_task_duration: Duration,
    pub recommendation: String,
}

impl Default for WorkloadAnalysis {
    fn default() -> Self {
        Self {
            optimal_thread_count: sysinfo::System::new().cpus().len().max(1),
            cpu_bound_ratio: 0.5,
            is_mostly_cpu_bound: false,
            is_mostly_io_bound: false,
            is_mixed: true,
            avg_task_duration: Duration::from_millis(10),
            recommendation: String::new(),
        }
    }
}
