use std::time::{Duration, Instant};
use sysinfo::System;

/// Monitors system load and performance metrics
pub struct SystemMonitor {
    system: System,
    last_update: Instant,
    update_interval: Duration,
    metrics: LoadMetrics,
}

impl SystemMonitor {
    pub fn new() -> Result<Self, String> {
        let mut system = System::new();
        system.refresh_cpu();

        Ok(Self {
            system,
            last_update: Instant::now(),
            update_interval: Duration::from_millis(100),
            metrics: LoadMetrics::default(),
        })
    }

    pub fn update(&mut self) -> Result<(), String> {
        let now = Instant::now();
        if now.duration_since(self.last_update) < self.update_interval {
            return Ok(());
        }

        self.system.refresh_cpu();
        self.system.refresh_memory();

        // Calculate CPU usage
        let cpu_usage: f64 = self
            .system
            .cpus()
            .iter()
            .map(|cpu| cpu.cpu_usage() as f64)
            .sum::<f64>()
            / self.system.cpus().len() as f64;

        // Calculate memory usage
        let total_memory = self.system.total_memory();
        let used_memory = self.system.used_memory();
        let memory_usage_percent = if total_memory > 0 {
            (used_memory as f64 / total_memory as f64) * 100.0
        } else {
            0.0
        };

        self.metrics = LoadMetrics {
            cpu_usage_percent: cpu_usage,
            memory_usage_percent,
            active_threads: self.get_active_thread_count(),
            timestamp: now,
        };

        self.last_update = now;
        Ok(())
    }

    pub fn get_current_load(&self) -> LoadMetrics {
        self.metrics.clone()
    }

    pub fn detect_blocking(&self) -> bool {
        // Detect if system is experiencing blocking (high CPU wait time)
        // Simplified: check if CPU usage is high but throughput is low
        self.metrics.cpu_usage_percent > 80.0
    }

    pub fn detect_contention(&self) -> bool {
        // Detect contention (many threads competing for resources)
        // High thread count + high CPU usage suggests contention
        self.metrics.active_threads > sysinfo::System::new().cpus().len() * 2
            && self.metrics.cpu_usage_percent > 70.0
    }

    pub fn detect_idle_cycles(&self) -> bool {
        // Detect idle CPU cycles
        self.metrics.cpu_usage_percent < 20.0
            && self.metrics.active_threads < sysinfo::System::new().cpus().len()
    }

    fn get_active_thread_count(&self) -> usize {
        // Approximation: use system info
        // In production, would track actual thread counts
        sysinfo::System::new().cpus().len()
    }
}

#[derive(Debug, Clone)]
pub struct LoadMetrics {
    pub cpu_usage_percent: f64,
    pub memory_usage_percent: f64,
    pub active_threads: usize,
    pub timestamp: Instant,
}

impl Default for LoadMetrics {
    fn default() -> Self {
        Self {
            cpu_usage_percent: 0.0,
            memory_usage_percent: 0.0,
            active_threads: 0,
            timestamp: Instant::now(),
        }
    }
}
