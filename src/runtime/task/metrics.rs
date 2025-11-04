use parking_lot::RwLock;
use std::cmp::max;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState {
    Idle,
    Busy,
    Parked,
}

#[derive(Debug, Clone)]
pub struct WorkerInfo {
    pub id: usize,
    pub state: WorkerState,
    pub queue_depth: usize,
    pub tasks_processed: u64,
}

#[derive(Debug, Default)]
pub struct TaskRuntimeMetrics {
    spawned: AtomicU64,
    completed: AtomicU64,
    waiting: AtomicI64,
    channels: AtomicU64,
    channel_waiters: AtomicI64,
    channel_backlog: AtomicI64,
    worker_infos: RwLock<Vec<WorkerInfo>>,
    active_workers: AtomicU64,
    total_workers: AtomicU64,
}

impl TaskRuntimeMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn record_spawn(&self) {
        self.spawned.fetch_add(1, Ordering::Relaxed);
        self.waiting.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_completion(&self) {
        self.completed.fetch_add(1, Ordering::Relaxed);
        self.waiting.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn record_waiting_delta(&self, delta: i64) {
        if delta != 0 {
            self.waiting.fetch_add(delta, Ordering::Relaxed);
        }
    }

    pub fn register_channel(&self) {
        self.channels.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_channel_waiters(&self, delta: i64) {
        if delta != 0 {
            self.channel_waiters.fetch_add(delta, Ordering::Relaxed);
        }
    }

    pub fn record_channel_backlog(&self, delta: i64) {
        if delta != 0 {
            self.channel_backlog.fetch_add(delta, Ordering::Relaxed);
        }
    }

    pub fn snapshot(&self) -> TaskMetricsSnapshot {
        let worker_infos = self.worker_infos.read().clone();
        TaskMetricsSnapshot {
            tasks_spawned: self.spawned.load(Ordering::Relaxed),
            tasks_completed: self.completed.load(Ordering::Relaxed),
            tasks_waiting: max(self.waiting.load(Ordering::Relaxed), 0) as u64,
            channels_registered: self.channels.load(Ordering::Relaxed),
            channel_waiters: max(self.channel_waiters.load(Ordering::Relaxed), 0) as u64,
            channel_backlog: max(self.channel_backlog.load(Ordering::Relaxed), 0) as u64,
            active_workers: self.active_workers.load(Ordering::Relaxed),
            total_workers: self.total_workers.load(Ordering::Relaxed),
            worker_infos,
        }
    }

    pub fn update_worker_info(&self, worker_id: usize, state: WorkerState, queue_depth: usize) {
        let mut infos = self.worker_infos.write();
        let current_len = infos.len();
        if worker_id < current_len {
            infos[worker_id].state = state;
            infos[worker_id].queue_depth = queue_depth;
        } else {
            // Grow vector if needed
            while infos.len() <= worker_id {
                let new_id = infos.len();
                infos.push(WorkerInfo {
                    id: new_id,
                    state: WorkerState::Idle,
                    queue_depth: 0,
                    tasks_processed: 0,
                });
            }
            infos[worker_id].state = state;
            infos[worker_id].queue_depth = queue_depth;
        }
    }

    pub fn record_worker_task(&self, worker_id: usize) {
        let mut infos = self.worker_infos.write();
        if worker_id < infos.len() {
            infos[worker_id].tasks_processed += 1;
        }
    }

    pub fn set_total_workers(&self, count: usize) {
        self.total_workers.store(count as u64, Ordering::Relaxed);
    }

    pub fn set_active_workers(&self, count: usize) {
        self.active_workers.store(count as u64, Ordering::Relaxed);
    }

    pub fn get_worker_infos(&self) -> Vec<WorkerInfo> {
        self.worker_infos.read().clone()
    }

    pub fn get_total_queue_depth(&self) -> usize {
        self.worker_infos.read().iter().map(|w| w.queue_depth).sum()
    }
}

#[derive(Debug, Clone)]
pub struct TaskMetricsSnapshot {
    pub tasks_spawned: u64,
    pub tasks_completed: u64,
    pub tasks_waiting: u64,
    pub channels_registered: u64,
    pub channel_waiters: u64,
    pub channel_backlog: u64,
    pub active_workers: u64,
    pub total_workers: u64,
    pub worker_infos: Vec<WorkerInfo>,
}
