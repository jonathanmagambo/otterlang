use parking_lot::RwLock;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::task::{Context, Poll, Waker};

use super::task::{Task, TaskHandle, TaskKind};
use super::thread_pool::AdaptiveThreadPool;

/// Unified scheduler that merges async tasks and thread-pool workers
pub struct UnifiedScheduler {
    task_queue: RwLock<mpsc::Receiver<Task>>,
    task_sender: mpsc::Sender<Task>,
    thread_pool: Arc<AdaptiveThreadPool>,
    pending_count: Arc<AtomicU64>,
    running_count: Arc<AtomicU64>,
    completed_count: Arc<AtomicU64>,
    wakers: Arc<RwLock<std::collections::HashMap<u64, Waker>>>,
    next_task_id: Arc<AtomicU64>,
}

impl UnifiedScheduler {
    pub fn new(thread_pool: Arc<AdaptiveThreadPool>) -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel();

        Ok(Self {
            task_queue: RwLock::new(receiver),
            task_sender: sender,
            thread_pool,
            pending_count: Arc::new(AtomicU64::new(0)),
            running_count: Arc::new(AtomicU64::new(0)),
            completed_count: Arc::new(AtomicU64::new(0)),
            wakers: Arc::new(RwLock::new(std::collections::HashMap::new())),
            next_task_id: Arc::new(AtomicU64::new(1)),
        })
    }

    pub fn spawn(&mut self, task: Task) -> TaskHandle {
        let task_id = self.next_task_id.fetch_add(1, Ordering::SeqCst);
        let handle = TaskHandle::new(task_id);

        let _ = self.task_sender.send(task);
        self.pending_count.fetch_add(1, Ordering::SeqCst);

        handle
    }

    pub fn spawn_parallel_loop<F>(&mut self, range: std::ops::Range<usize>, f: F) -> TaskHandle
    where
        F: Fn(usize) + Send + Sync + 'static,
    {
        let task = Task::parallel_loop(range, f);
        self.spawn(task)
    }

    pub fn spawn_async<Fut>(&mut self, future: Fut) -> TaskHandle
    where
        Fut: Future<Output = ()> + Send + 'static,
    {
        let task = Task::async_task(future);
        self.spawn(task)
    }

    pub fn process_tasks(&self) {
        // Try to receive tasks from the queue
        let receiver = self.task_queue.read();
        while let Ok(task) = receiver.try_recv() {
            self.pending_count.fetch_sub(1, Ordering::SeqCst);
            self.running_count.fetch_add(1, Ordering::SeqCst);

            match task.kind {
                TaskKind::Async(future) => {
                    // Handle async task
                    self.handle_async_task(task.id, future);
                }
                TaskKind::Parallel(work) => {
                    // Execute on thread pool
                    let pool = self.thread_pool.clone();
                    let running = self.running_count.clone();
                    let completed = self.completed_count.clone();

                    pool.execute(move || {
                        work();
                        running.fetch_sub(1, Ordering::SeqCst);
                        completed.fetch_add(1, Ordering::SeqCst);
                    });
                }
                TaskKind::ParallelLoop { range, work } => {
                    // Execute parallel loop
                    self.execute_parallel_loop(range, Arc::new(work));
                }
            }
        }
    }

    fn handle_async_task<Fut>(&self, task_id: u64, future: Fut)
    where
        Fut: Future<Output = ()> + Send + 'static,
    {
        // Create a pollable future wrapper
        let wakers = self.wakers.clone();
        let running = self.running_count.clone();
        let completed = self.completed_count.clone();

        // For now, execute async tasks immediately
        // In full implementation, would use async runtime
        self.thread_pool.execute(move || {
            // Poll the future
            let mut pinned = Box::pin(future);
            let waker = create_waker(task_id, wakers.clone());
            let mut cx = Context::from_waker(&waker);

            loop {
                match pinned.as_mut().poll(&mut cx) {
                    Poll::Ready(_) => break,
                    Poll::Pending => {
                        // Yield and continue later
                        std::thread::yield_now();
                    }
                }
            }

            running.fetch_sub(1, Ordering::SeqCst);
            completed.fetch_add(1, Ordering::SeqCst);
        });
    }

    fn execute_parallel_loop<F>(&self, range: std::ops::Range<usize>, work: Arc<F>)
    where
        F: Fn(usize) + Send + Sync + 'static,
    {
        let pool = self.thread_pool.clone();
        let running = self.running_count.clone();
        let completed = self.completed_count.clone();

        // Split range into chunks for parallel execution
        let chunk_size = (range.end - range.start) / pool.get_thread_count().max(1);
        let mut handles = Vec::new();

        for chunk_start in (range.start..range.end).step_by(chunk_size.max(1)) {
            let chunk_end = (chunk_start + chunk_size).min(range.end);
            let work = work.clone();

            let h = pool.spawn(move || {
                for i in chunk_start..chunk_end {
                    work(i);
                }
            });
            handles.push(h);
        }

        // Wait for all chunks to complete
        for handle in handles {
            let _ = handle.join();
        }

        running.fetch_sub(1, Ordering::SeqCst);
        completed.fetch_add(1, Ordering::SeqCst);
    }

    pub fn get_stats(&self) -> SchedulerStats {
        SchedulerStats {
            pending_tasks: self.pending_count.load(Ordering::SeqCst) as usize,
            running_tasks: self.running_count.load(Ordering::SeqCst) as usize,
            completed_tasks: self.completed_count.load(Ordering::SeqCst),
        }
    }
}

fn create_waker(
    _task_id: u64,
    _wakers: Arc<RwLock<std::collections::HashMap<u64, Waker>>>,
) -> Waker {
    use std::task::{RawWaker, RawWakerVTable};

    unsafe fn clone_waker(data: *const ()) -> RawWaker {
        RawWaker::new(data, &VTABLE)
    }

    unsafe fn wake_waker(_data: *const ()) {
        // Wake logic would go here
    }

    unsafe fn wake_by_ref_waker(data: *const ()) {
        unsafe {
            wake_waker(data);
        }
    }

    unsafe fn drop_waker(_data: *const ()) {}

    static VTABLE: RawWakerVTable =
        RawWakerVTable::new(clone_waker, wake_waker, wake_by_ref_waker, drop_waker);

    let data = _task_id as *const ();
    unsafe { Waker::from_raw(RawWaker::new(data, &VTABLE)) }
}

#[derive(Debug, Clone)]
pub struct SchedulerStats {
    pub pending_tasks: usize,
    pub running_tasks: usize,
    pub completed_tasks: u64,
}
