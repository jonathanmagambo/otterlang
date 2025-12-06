// Self-Optimizing Concurrency Subsystem
pub mod extensions;
pub mod monitoring;
pub mod rebalancer;
pub mod scheduler;
pub mod task;
pub mod thread_pool;
pub mod workload_analyzer;

pub use extensions::{DefaultWorkloadAdapter, WorkloadAdapter, WorkloadType};
pub use monitoring::{LoadMetrics, SystemMonitor};
pub use rebalancer::Rebalancer;
pub use scheduler::UnifiedScheduler;
pub use task::{Task, TaskHandle, TaskKind, TaskPriority};
pub use thread_pool::AdaptiveThreadPool;
pub use workload_analyzer::WorkloadAnalyzer;

use parking_lot::RwLock;
use std::rc::Rc;
use std::sync::Arc;

/// Main concurrency manager that coordinates all subsystems
pub struct ConcurrencyManager {
    scheduler: Rc<RwLock<UnifiedScheduler>>,
    thread_pool: Arc<AdaptiveThreadPool>,
    monitor: Arc<RwLock<SystemMonitor>>,
    #[expect(dead_code, reason = "Work in progress")]
    analyzer: Arc<RwLock<WorkloadAnalyzer>>,
    rebalancer: RwLock<Rebalancer>,
}

impl ConcurrencyManager {
    pub fn new() -> Result<Self, String> {
        let monitor = Arc::new(RwLock::new(SystemMonitor::new()?));
        let thread_pool = Arc::new(AdaptiveThreadPool::new()?);
        let analyzer = Arc::new(RwLock::new(WorkloadAnalyzer::new()));

        let scheduler = Rc::new(RwLock::new(UnifiedScheduler::new(thread_pool.clone())?));

        let rebalancer = RwLock::new(Rebalancer::new(
            scheduler.clone(),
            thread_pool.clone(),
            monitor.clone(),
            analyzer.clone(),
        ));

        Ok(Self {
            scheduler,
            thread_pool,
            monitor,
            analyzer,
            rebalancer,
        })
    }

    pub fn spawn_task(&self, task: Task) -> TaskHandle {
        self.scheduler.write().spawn(task)
    }

    pub fn spawn_async<Fut>(&self, future: Fut) -> TaskHandle
    where
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let task = Task::async_task(future);
        self.spawn_task(task)
    }

    pub fn spawn_parallel<F>(&self, work: F) -> TaskHandle
    where
        F: FnOnce() + Send + 'static,
    {
        let task = Task::parallel_task(work);
        self.spawn_task(task)
    }

    pub fn run_parallel_loop<F>(&self, range: std::ops::Range<usize>, f: F) -> TaskHandle
    where
        F: Fn(usize) + Send + Sync + 'static,
    {
        self.scheduler.write().spawn_parallel_loop(range, f)
    }

    pub fn update_monitoring(&self) -> Result<(), String> {
        self.monitor.write().update()
    }

    pub fn trigger_rebalance(&self) -> Result<(), String> {
        self.rebalancer.write().rebalance()
    }

    pub fn get_stats(&self) -> ConcurrencyStats {
        let load = self.monitor.read().get_current_load();
        let pool_stats = self.thread_pool.get_stats();
        let scheduler_stats = self.scheduler.read().get_stats();

        ConcurrencyStats {
            cpu_load: load.cpu_usage_percent,
            active_threads: pool_stats.active_threads,
            total_threads: pool_stats.total_threads,
            pending_tasks: scheduler_stats.pending_tasks,
            running_tasks: scheduler_stats.running_tasks,
            completed_tasks: scheduler_stats.completed_tasks,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConcurrencyStats {
    pub cpu_load: f64,
    pub active_threads: usize,
    pub total_threads: usize,
    pub pending_tasks: usize,
    pub running_tasks: usize,
    pub completed_tasks: u64,
}

impl Default for ConcurrencyManager {
    fn default() -> Self {
        Self::new().expect("Failed to create ConcurrencyManager")
    }
}
