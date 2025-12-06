use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};

/// Represents a task that can be executed by the scheduler
pub struct Task {
    pub id: u64,
    pub kind: TaskKind,
    pub priority: TaskPriority,
}

impl Task {
    pub fn async_task<Fut>(future: Fut) -> Self
    where
        Fut: Future<Output = ()> + Send + 'static,
    {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::SeqCst),
            kind: TaskKind::Async(Box::pin(future)),
            priority: TaskPriority::Normal,
        }
    }

    pub fn parallel_task<F>(work: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::SeqCst),
            kind: TaskKind::Parallel(Box::new(work)),
            priority: TaskPriority::Normal,
        }
    }

    pub fn parallel_loop<F>(range: std::ops::Range<usize>, work: F) -> Self
    where
        F: Fn(usize) + Send + Sync + 'static,
    {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::SeqCst),
            kind: TaskKind::ParallelLoop {
                range,
                work: Box::new(work),
            },
            priority: TaskPriority::Normal,
        }
    }

    pub fn with_priority(mut self, priority: TaskPriority) -> Self {
        self.priority = priority;
        self
    }
}

/// Different kinds of tasks
pub enum TaskKind {
    /// Async task (cooperative multitasking)
    Async(Pin<Box<dyn Future<Output = ()> + Send>>),
    /// CPU-bound parallel task
    Parallel(Box<dyn FnOnce() + Send>),
    /// Parallel loop iteration
    ParallelLoop {
        range: std::ops::Range<usize>,
        work: Box<dyn Fn(usize) + Send + Sync>,
    },
}

/// Task priority levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

/// Handle to a spawned task
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskHandle {
    task_id: u64,
}

impl TaskHandle {
    pub fn new(task_id: u64) -> Self {
        Self { task_id }
    }

    pub fn id(&self) -> u64 {
        self.task_id
    }
}
