use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

/// Adaptive thread pool that dynamically tunes thread count
pub struct AdaptiveThreadPool {
    threads: Arc<RwLock<Vec<thread::JoinHandle<()>>>>,
    work_queue: Arc<Receiver<Box<dyn FnOnce() + Send>>>,
    work_sender: Sender<Box<dyn FnOnce() + Send>>,
    thread_count: Arc<AtomicUsize>,
    active_threads: Arc<AtomicUsize>,
    min_threads: usize,
    max_threads: usize,
    shutdown: Arc<AtomicBool>,
}

impl AdaptiveThreadPool {
    pub fn new() -> Result<Self, String> {
        let min_threads = 1;
        let max_threads = num_cpus().max(4);

        let (sender, receiver) = unbounded();

        let pool = Self {
            threads: Arc::new(RwLock::new(Vec::new())),
            work_queue: Arc::new(receiver),
            work_sender: sender,
            thread_count: Arc::new(AtomicUsize::new(min_threads)),
            active_threads: Arc::new(AtomicUsize::new(0)),
            min_threads,
            max_threads,
            shutdown: Arc::new(AtomicBool::new(false)),
        };

        // Start initial threads
        pool.adjust_thread_count(min_threads)?;

        Ok(pool)
    }

    pub fn execute<F>(&self, work: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let _ = self.work_sender.send(Box::new(work));
    }

    pub fn spawn<F>(&self, work: F) -> thread::JoinHandle<()>
    where
        F: FnOnce() + Send + 'static,
    {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.execute(move || {
            work();
            let _ = sender.send(());
        });

        thread::spawn(move || {
            let _ = receiver.recv();
        })
    }

    pub fn adjust_thread_count(&self, target_count: usize) -> Result<(), String> {
        let current = self.thread_count.load(Ordering::SeqCst);
        let target = target_count.max(self.min_threads).min(self.max_threads);

        if target > current {
            // Add threads
            for _ in current..target {
                self.add_worker_thread()?;
            }
        } else if target < current {
            // Remove threads (they'll finish current work and exit)
            // In practice, we'd signal them to stop accepting new work
        }

        self.thread_count.store(target, Ordering::SeqCst);
        Ok(())
    }

    fn add_worker_thread(&self) -> Result<(), String> {
        let queue = self.work_queue.clone();
        let active = self.active_threads.clone();
        let shutdown = self.shutdown.clone();

        let handle = thread::spawn(move || {
            active.fetch_add(1, Ordering::SeqCst);

            loop {
                if shutdown.load(Ordering::SeqCst) {
                    break;
                }

                match queue.recv_timeout(Duration::from_millis(100)) {
                    Ok(work) => {
                        work();
                    }
                    Err(_) => {
                        // Timeout - check if we should exit
                        // In adaptive mode, idle threads might exit
                    }
                }
            }

            active.fetch_sub(1, Ordering::SeqCst);
        });

        self.threads.write().push(handle);
        Ok(())
    }

    pub fn get_thread_count(&self) -> usize {
        self.thread_count.load(Ordering::SeqCst)
    }

    pub fn get_active_threads(&self) -> usize {
        self.active_threads.load(Ordering::SeqCst)
    }

    pub fn get_stats(&self) -> ThreadPoolStats {
        ThreadPoolStats {
            total_threads: self.get_thread_count(),
            active_threads: self.get_active_threads(),
            min_threads: self.min_threads,
            max_threads: self.max_threads,
        }
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let threads = self.threads.write();
        for handle in threads.iter() {
            handle.thread().unpark();
        }
    }
}

impl Drop for AdaptiveThreadPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Debug, Clone)]
pub struct ThreadPoolStats {
    pub total_threads: usize,
    pub active_threads: usize,
    pub min_threads: usize,
    pub max_threads: usize,
}

fn num_cpus() -> usize {
    sysinfo::System::new().cpus().len().max(1)
}
