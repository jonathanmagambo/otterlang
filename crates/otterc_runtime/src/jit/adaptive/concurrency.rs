use parking_lot::RwLock;
use rayon::ThreadPool;
use rayon::ThreadPoolBuilder;
use std::sync::Arc;
use sysinfo::System;

/// Adaptive concurrency management
pub struct AdaptiveConcurrencyManager {
    thread_pool: Arc<RwLock<Option<ThreadPool>>>,
    optimal_threads: usize,
    workload_profile: RwLock<WorkloadProfile>,
}

#[derive(Debug, Clone)]
struct WorkloadProfile {
    total_tasks: usize,
    parallel_tasks: usize,
    avg_task_duration_ms: f64,
}

impl AdaptiveConcurrencyManager {
    pub fn new() -> Self {
        let num_threads = System::new().cpus().len().max(1);
        Self {
            thread_pool: Arc::new(RwLock::new(None)),
            optimal_threads: num_threads,
            workload_profile: RwLock::new(WorkloadProfile {
                total_tasks: 0,
                parallel_tasks: 0,
                avg_task_duration_ms: 0.0,
            }),
        }
    }

    pub fn initialize_thread_pool(&self) -> Result<(), String> {
        let pool = ThreadPoolBuilder::new()
            .num_threads(self.optimal_threads)
            .build()
            .map_err(|e| format!("Failed to create thread pool: {}", e))?;

        *self.thread_pool.write() = Some(pool);
        Ok(())
    }

    pub fn record_task(&self, is_parallel: bool, duration_ms: f64) {
        let mut profile = self.workload_profile.write();
        profile.total_tasks += 1;
        if is_parallel {
            profile.parallel_tasks += 1;
        }

        // Update average duration
        let total_duration =
            profile.avg_task_duration_ms * (profile.total_tasks - 1) as f64 + duration_ms;
        profile.avg_task_duration_ms = total_duration / profile.total_tasks as f64;
    }

    pub fn adjust_thread_count(&mut self) {
        let profile = self.workload_profile.read();
        let parallelism_ratio = if profile.total_tasks > 0 {
            profile.parallel_tasks as f64 / profile.total_tasks as f64
        } else {
            0.0
        };

        // Adjust thread count based on workload characteristics
        let cpu_count = System::new().cpus().len().max(1);
        if parallelism_ratio > 0.5 {
            // High parallelism - use more threads
            self.optimal_threads = cpu_count * 2;
        } else if parallelism_ratio < 0.2 {
            // Low parallelism - use fewer threads
            self.optimal_threads = cpu_count.max(1);
        } else {
            self.optimal_threads = cpu_count;
        }

        // Recreate thread pool with new count
        if let Ok(pool) = ThreadPoolBuilder::new()
            .num_threads(self.optimal_threads)
            .build()
        {
            *self.thread_pool.write() = Some(pool);
        }
    }

    pub fn should_parallelize(&self, task_count: usize, estimated_duration_ms: f64) -> bool {
        let profile = self.workload_profile.read();

        // Parallelize if:
        // 1. Multiple tasks
        // 2. Tasks are long enough to justify overhead
        task_count > 1 && estimated_duration_ms > 1.0 && profile.avg_task_duration_ms > 0.5
    }
}

impl Default for AdaptiveConcurrencyManager {
    fn default() -> Self {
        Self::new()
    }
}
