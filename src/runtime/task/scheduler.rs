use crossbeam_deque::{Injector, Steal, Stealer, Worker};
use crossbeam_utils::Backoff;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::metrics::{TaskRuntimeMetrics, WorkerState};
use super::task::{JoinHandle, Task, TaskFn};
use super::timer::TimerWheel;
use super::tls::cleanup_task_local_storage;

#[derive(Debug, Clone, Copy)]
pub struct SchedulerConfig {
    pub max_workers: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        Self {
            max_workers: workers,
        }
    }
}

#[derive(Debug)]
struct SchedulerCore {
    injector: Injector<Task>,
    _stealers: Arc<Vec<Stealer<Task>>>,
    metrics: Arc<TaskRuntimeMetrics>,
    shutdown: AtomicBool,
    timer_wheel: Arc<TimerWheel>,
    worker_count: AtomicUsize,
    _config: SchedulerConfig,
}

#[derive(Debug, Clone)]
pub struct TaskScheduler {
    core: Arc<SchedulerCore>,
}

impl TaskScheduler {
    pub fn new(config: SchedulerConfig) -> Self {
        let metrics = TaskRuntimeMetrics::new();
        let injector = Injector::new();
        let timer_wheel = Arc::new(TimerWheel::new());
        let mut workers = Vec::with_capacity(config.max_workers);
        let mut stealer_store = Vec::with_capacity(config.max_workers);

        for _ in 0..config.max_workers {
            let worker = Worker::new_fifo();
            stealer_store.push(worker.stealer());
            workers.push(worker);
        }

        let stealers = Arc::new(stealer_store);

        let core = Arc::new(SchedulerCore {
            injector,
            _stealers: Arc::clone(&stealers),
            metrics: Arc::clone(&metrics),
            shutdown: AtomicBool::new(false),
            timer_wheel: Arc::clone(&timer_wheel),
            worker_count: AtomicUsize::new(config.max_workers),
            _config: config,
        });

        metrics.set_total_workers(config.max_workers);
        metrics.set_active_workers(config.max_workers);

        // Spawn auto-scaling thread
        let autoscale_core = Arc::clone(&core);
        thread::Builder::new()
            .name("otter-autoscaler".into())
            .spawn(move || autoscaler_loop(autoscale_core))
            .expect("failed to spawn autoscaler");

        // Spawn timer processing thread
        let timer_core = Arc::clone(&core);
        thread::Builder::new()
            .name("otter-timer-processor".into())
            .spawn(move || timer_processor_loop(timer_core))
            .expect("failed to spawn timer processor");

        for (index, worker) in workers.into_iter().enumerate() {
            let core = Arc::clone(&core);
            let stealers = Arc::clone(&stealers);
            thread::Builder::new()
                .name(format!("otter-task-worker-{}", index))
                .spawn(move || worker_loop(core, stealers, worker, index))
                .expect("failed to spawn task worker");
        }

        Self { core }
    }

    pub fn timer_wheel(&self) -> Arc<TimerWheel> {
        Arc::clone(&self.core.timer_wheel)
    }

    pub fn metrics(&self) -> Arc<TaskRuntimeMetrics> {
        Arc::clone(&self.core.metrics)
    }

    pub fn spawn_fn<F>(&self, name: Option<String>, func: F) -> JoinHandle
    where
        F: FnOnce() + Send + 'static,
    {
        let task = Task::new(name, Box::new(func) as TaskFn);
        let cancellation_token = task.cancellation_token().clone();
        let join = JoinHandle::new(task.id(), task.join_state(), cancellation_token);
        self.core.metrics.record_spawn();
        self.core.injector.push(task);
        join
    }

    pub fn get_worker_count(&self) -> usize {
        self.core.worker_count.load(Ordering::Relaxed)
    }

    pub fn get_queue_depth(&self) -> usize {
        // Estimate queue depth from injector (this is approximate)
        // crossbeam-deque doesn't expose exact size, so we estimate based on metrics
        let snapshot = self.core.metrics.snapshot();
        snapshot.tasks_waiting as usize
    }
}

fn worker_loop(
    core: Arc<SchedulerCore>,
    stealers: Arc<Vec<Stealer<Task>>>,
    local: Worker<Task>,
    index: usize,
) {
    let stealers: Vec<_> = stealers
        .iter()
        .enumerate()
        .filter_map(|(i, stealer)| {
            if i != index {
                Some(stealer.clone())
            } else {
                None
            }
        })
        .collect();
    let backoff = Backoff::new();
    let mut consecutive_idle = 0;

    loop {
        if core.shutdown.load(Ordering::SeqCst) {
            break;
        }

        let queue_depth = local.len();
        core.metrics
            .update_worker_info(index, WorkerState::Busy, queue_depth);

        if let Some(task) = local.pop() {
            backoff.reset();
            consecutive_idle = 0;
            // Skip cancelled tasks
            let task_id = task.id();
            if task.is_cancelled() {
                core.metrics.record_completion();
                cleanup_task_local_storage(task_id);
                continue;
            }
            task.run();
            core.metrics.record_completion();
            core.metrics.record_worker_task(index);
            cleanup_task_local_storage(task_id);
            continue;
        }

        match core.injector.steal_batch_and_pop(&local) {
            Steal::Success(task) => {
                backoff.reset();
                consecutive_idle = 0;
                // Skip cancelled tasks
                let task_id = task.id();
                if task.is_cancelled() {
                    core.metrics.record_completion();
                    cleanup_task_local_storage(task_id);
                    continue;
                }
                task.run();
                core.metrics.record_completion();
                core.metrics.record_worker_task(index);
                cleanup_task_local_storage(task_id);
                continue;
            }
            Steal::Retry => {
                backoff.spin();
                continue;
            }
            Steal::Empty => {}
        }

        let mut stolen = None;
        for stealer in &stealers {
            match stealer.steal() {
                Steal::Success(task) => {
                    stolen = Some(task);
                    break;
                }
                Steal::Retry => {
                    stolen = None;
                    break;
                }
                Steal::Empty => continue,
            }
        }

        if let Some(task) = stolen {
            backoff.reset();
            consecutive_idle = 0;
            // Skip cancelled tasks
            let task_id = task.id();
            if task.is_cancelled() {
                core.metrics.record_completion();
                cleanup_task_local_storage(task_id);
                continue;
            }
            task.run();
            core.metrics.record_completion();
            core.metrics.record_worker_task(index);
            cleanup_task_local_storage(task_id);
            continue;
        }

        // Nothing to do
        consecutive_idle += 1;
        let queue_depth = local.len();
        if consecutive_idle > 10 {
            core.metrics
                .update_worker_info(index, WorkerState::Idle, queue_depth);
        } else {
            core.metrics
                .update_worker_info(index, WorkerState::Parked, queue_depth);
        }

        // Check for timer wakeups and yield slightly.
        if backoff.is_completed() {
            // Process timers periodically
            core.timer_wheel.process_expired();
            thread::sleep(Duration::from_micros(100));
        } else {
            backoff.snooze();
        }
    }
}

fn autoscaler_loop(core: Arc<SchedulerCore>) {
    loop {
        if core.shutdown.load(Ordering::SeqCst) {
            break;
        }

        // Check metrics every 2 seconds
        thread::sleep(Duration::from_secs(2));

        let snapshot = core.metrics.snapshot();
        let _current_workers = core.worker_count.load(Ordering::Relaxed);
        let _queue_depth = snapshot.tasks_waiting;
        let active_workers = snapshot
            .worker_infos
            .iter()
            .filter(|w| w.state == WorkerState::Busy)
            .count();

        // Simple auto-scaling logic:
        // - If queue depth > 2x workers and we haven't hit max, consider scaling up
        // - If most workers are idle for extended period, consider scaling down
        // Note: Actual worker addition/removal is complex with crossbeam-deque
        // For now, we just track and report the metrics

        // Update active worker count
        core.metrics.set_active_workers(active_workers);
    }
}

fn timer_processor_loop(core: Arc<SchedulerCore>) {
    loop {
        if core.shutdown.load(Ordering::SeqCst) {
            break;
        }

        // Process expired timers
        if let Some(next_timeout) = core.timer_wheel.process_expired() {
            // Sleep until the next timer expires or a short timeout
            let sleep_duration = next_timeout.min(Duration::from_millis(100));
            thread::sleep(sleep_duration);
        } else {
            // No timers scheduled, sleep briefly
            thread::sleep(Duration::from_millis(100));
        }
    }
}
