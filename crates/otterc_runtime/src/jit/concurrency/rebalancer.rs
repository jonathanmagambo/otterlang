use parking_lot::RwLock;
use std::rc::Rc;
use std::sync::Arc;

use super::monitoring::SystemMonitor;
use super::scheduler::UnifiedScheduler;
use super::thread_pool::AdaptiveThreadPool;
use super::workload_analyzer::WorkloadAnalyzer;

/// Automatically rebalances the system based on detected conditions
pub struct Rebalancer {
    scheduler: Rc<RwLock<UnifiedScheduler>>,
    thread_pool: Arc<AdaptiveThreadPool>,
    monitor: Arc<RwLock<SystemMonitor>>,
    analyzer: Arc<RwLock<WorkloadAnalyzer>>,
    rebalance_interval: std::time::Duration,
    last_rebalance: std::time::Instant,
}

impl Rebalancer {
    pub fn new(
        scheduler: Rc<RwLock<UnifiedScheduler>>,
        thread_pool: Arc<AdaptiveThreadPool>,
        monitor: Arc<RwLock<SystemMonitor>>,
        analyzer: Arc<RwLock<WorkloadAnalyzer>>,
    ) -> Self {
        Self {
            scheduler,
            thread_pool,
            monitor,
            analyzer,
            rebalance_interval: std::time::Duration::from_secs(2),
            last_rebalance: std::time::Instant::now(),
        }
    }

    pub fn rebalance(&mut self) -> Result<(), String> {
        let now = std::time::Instant::now();
        if now.duration_since(self.last_rebalance) < self.rebalance_interval {
            return Ok(());
        }

        // Update monitoring data
        self.monitor.write().update()?;

        // Get current metrics
        let scheduler_stats = self.scheduler.read().get_stats();
        let pool_stats = self.thread_pool.get_stats();

        // Analyze workload
        let analysis = self
            .analyzer
            .write()
            .analyze_workload(&scheduler_stats, &pool_stats);

        // Detect conditions
        let is_blocking = self.monitor.read().detect_blocking();
        let has_contention = self.monitor.read().detect_contention();
        let has_idle_cycles = self.monitor.read().detect_idle_cycles();

        // Adjust thread count based on analysis
        if analysis.optimal_thread_count != pool_stats.total_threads {
            self.thread_pool
                .adjust_thread_count(analysis.optimal_thread_count)?;
        }

        // Handle blocking: reduce thread count to avoid context switching overhead
        if is_blocking && pool_stats.total_threads > sysinfo::System::new().cpus().len() {
            let target = sysinfo::System::new().cpus().len();
            self.thread_pool.adjust_thread_count(target)?;
        }

        // Handle contention: reduce thread count or increase based on workload
        if has_contention {
            if analysis.is_mostly_cpu_bound {
                // For CPU-bound with contention, reduce threads slightly
                let target = (pool_stats.total_threads * 9 / 10).max(1);
                self.thread_pool.adjust_thread_count(target)?;
            } else {
                // For I/O-bound with contention, might need more threads
                let target = pool_stats.total_threads + 2;
                self.thread_pool.adjust_thread_count(target)?;
            }
        }

        // Handle idle cycles: reduce thread count to save resources
        if has_idle_cycles {
            let target = (pool_stats.total_threads * 3 / 4).max(1);
            self.thread_pool.adjust_thread_count(target)?;
        }

        self.last_rebalance = now;
        Ok(())
    }

    pub fn trigger_immediate_rebalance(&mut self) -> Result<(), String> {
        self.rebalance_interval = std::time::Duration::from_millis(100);
        self.rebalance()
    }

    pub fn get_rebalance_info(&self) -> RebalanceInfo {
        let load = self.monitor.read().get_current_load();
        let scheduler_stats = self.scheduler.read().get_stats();
        let pool_stats = self.thread_pool.get_stats();

        RebalanceInfo {
            cpu_load: load.cpu_usage_percent,
            active_threads: pool_stats.active_threads,
            total_threads: pool_stats.total_threads,
            pending_tasks: scheduler_stats.pending_tasks,
            is_blocking: self.monitor.read().detect_blocking(),
            has_contention: self.monitor.read().detect_contention(),
            has_idle_cycles: self.monitor.read().detect_idle_cycles(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RebalanceInfo {
    pub cpu_load: f64,
    pub active_threads: usize,
    pub total_threads: usize,
    pub pending_tasks: usize,
    pub is_blocking: bool,
    pub has_contention: bool,
    pub has_idle_cycles: bool,
}
