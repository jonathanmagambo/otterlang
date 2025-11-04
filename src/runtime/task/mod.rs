//! Task runtime scaffolding.
//!
//! Provides a lightweight task scheduler, join handles, and runtime metrics
//! used by the standard library FFI bindings.

mod channel;
mod metrics;
mod scheduler;
mod task;
mod timer;
mod tls;

pub use channel::{select2, select2_async, SelectResult, TaskChannel, TaskMailBox};
pub use metrics::{TaskMetricsSnapshot, TaskRuntimeMetrics, WorkerInfo, WorkerState};
pub use scheduler::{SchedulerConfig, TaskScheduler};
pub use task::{CancellationToken, JoinFuture, JoinHandle, Task, TaskFn, TaskId, TaskState};
pub use timer::TimerWheel;
pub use tls::{
    cleanup_task_local_storage, get_task_local_storage, TaskLocalRegistry, TaskLocalStorage,
};

use std::sync::Once;

#[derive(Debug)]
pub struct TaskRuntime {
    scheduler: TaskScheduler,
}

impl TaskRuntime {
    fn new() -> Self {
        register_exit_hook();
        let scheduler = TaskScheduler::new(SchedulerConfig::default());
        // Register metrics with runtime for FFI access
        #[cfg(feature = "task-runtime")]
        crate::runtime::stdlib::runtime::register_task_metrics(scheduler.metrics());
        Self { scheduler }
    }

    pub fn scheduler(&self) -> &TaskScheduler {
        &self.scheduler
    }
}

static RUNTIME: once_cell::sync::Lazy<TaskRuntime> = once_cell::sync::Lazy::new(TaskRuntime::new);

pub fn runtime() -> &'static TaskRuntime {
    &*RUNTIME
}

/// Initializes the task runtime and returns a scheduler handle.
pub fn init_runtime() -> TaskScheduler {
    runtime().scheduler().clone()
}

fn register_exit_hook() {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        #[cfg(feature = "task-runtime")]
        extern "C" fn at_exit() {
            crate::runtime::stdlib::runtime::emit_task_metrics_report();
        }

        #[cfg(feature = "task-runtime")]
        unsafe {
            libc::atexit(at_exit);
        }
    });
}
