// GPU and I/O-bound workload extensions
// These traits and types allow the scheduler to be extended for specialized workloads

use super::task::TaskHandle;

/// Trait for GPU workload executors
pub trait GpuExecutor: Send + Sync {
    /// Execute a GPU-bound task
    fn execute_gpu<F>(&self, work: F) -> TaskHandle
    where
        F: FnOnce() + Send + 'static;

    /// Get GPU utilization percentage
    fn get_gpu_utilization(&self) -> f64;

    /// Get available GPU memory
    fn get_gpu_memory(&self) -> (u64, u64); // (used, total)
}

/// Trait for I/O-bound workload executors
pub trait IoExecutor: Send + Sync {
    /// Execute an I/O-bound task
    fn execute_io<F>(&self, work: F) -> TaskHandle
    where
        F: FnOnce() + Send + 'static;

    /// Get I/O wait time percentage
    fn get_io_wait_percent(&self) -> f64;

    /// Get pending I/O operations count
    fn get_pending_io(&self) -> usize;
}

/// Workload type classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkloadType {
    CpuBound,
    IoBound,
    GpuBound,
    Mixed,
}

impl WorkloadType {
    pub fn from_characteristics(cpu_ratio: f64, io_ratio: f64, gpu_ratio: f64) -> Self {
        if cpu_ratio > 0.7 {
            WorkloadType::CpuBound
        } else if io_ratio > 0.7 {
            WorkloadType::IoBound
        } else if gpu_ratio > 0.7 {
            WorkloadType::GpuBound
        } else {
            WorkloadType::Mixed
        }
    }
}

/// Extension point for workload-specific optimizations
pub trait WorkloadAdapter: Send + Sync {
    /// Adapt thread count based on workload type
    fn adapt_thread_count(&self, workload_type: WorkloadType, current_threads: usize) -> usize;

    /// Suggest optimal concurrency level for workload type
    fn suggest_concurrency(&self, workload_type: WorkloadType) -> usize;
}

/// Default workload adapter implementation
pub struct DefaultWorkloadAdapter;

impl WorkloadAdapter for DefaultWorkloadAdapter {
    fn adapt_thread_count(&self, workload_type: WorkloadType, current_threads: usize) -> usize {
        let cpu_count = sysinfo::System::new().cpus().len().max(1);

        match workload_type {
            WorkloadType::CpuBound => {
                // CPU-bound: use 2x CPU count for hyperthreading
                (cpu_count * 2).max(current_threads)
            }
            WorkloadType::IoBound => {
                // I/O-bound: use more threads to handle concurrent I/O
                (cpu_count * 4).max(current_threads)
            }
            WorkloadType::GpuBound => {
                // GPU-bound: fewer CPU threads, GPU handles most work
                cpu_count.max(1)
            }
            WorkloadType::Mixed => {
                // Mixed: balanced approach
                (cpu_count * 2).max(current_threads)
            }
        }
    }

    fn suggest_concurrency(&self, workload_type: WorkloadType) -> usize {
        let cpu_count = sysinfo::System::new().cpus().len().max(1);

        match workload_type {
            WorkloadType::CpuBound | WorkloadType::Mixed => cpu_count * 2,
            WorkloadType::IoBound => cpu_count * 4,
            WorkloadType::GpuBound => cpu_count,
        }
    }
}
