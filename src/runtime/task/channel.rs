use parking_lot::Mutex;
use std::sync::Arc;
use std::task::Waker;

use super::metrics::TaskRuntimeMetrics;

#[derive(Debug)]
pub struct TaskChannel<T> {
    inner: Arc<ChannelInner<T>>,
}

#[derive(Debug)]
struct ChannelInner<T> {
    queue: Mutex<Vec<T>>,
    receiver_wakers: Mutex<Vec<Waker>>,
    metrics: Option<Arc<TaskRuntimeMetrics>>,
    closed: parking_lot::Mutex<bool>,
    condvar: parking_lot::Condvar,
}

impl<T> Default for TaskChannel<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> TaskChannel<T> {
    pub fn new() -> Self {
        Self::with_metrics(None)
    }

    pub fn with_metrics(metrics: Option<Arc<TaskRuntimeMetrics>>) -> Self {
        if let Some(metrics) = &metrics {
            metrics.register_channel();
        }
        Self {
            inner: Arc::new(ChannelInner {
                queue: Mutex::new(Vec::new()),
                receiver_wakers: Mutex::new(Vec::new()),
                metrics,
                closed: parking_lot::Mutex::new(false),
                condvar: parking_lot::Condvar::new(),
            }),
        }
    }

    /// Send a value to the channel, waking any waiting receivers.
    pub fn send(&self, value: T) {
        let mut queue = self.inner.queue.lock();
        let mut wakers = self.inner.receiver_wakers.lock();

        if let Some(metrics) = &self.inner.metrics {
            metrics.record_channel_backlog(1);
        }

        queue.push(value);

        // Wake up one waiting receiver (task-aware)
        if let Some(waker) = wakers.pop() {
            waker.wake();
        }

        // Also notify condvar for blocking recv
        self.inner.condvar.notify_one();
    }

    /// Receive a value, blocking if none is available.
    /// This is a legacy blocking API. For task-aware code, use `recv_async` instead.
    pub fn recv(&self) -> Option<T> {
        // Try immediate receive first
        if let Some(value) = self.try_recv() {
            return Some(value);
        }

        // For blocking recv, use condvar to wait for data
        let mut queue = self.inner.queue.lock();

        loop {
            if !queue.is_empty() {
                let value = queue.remove(0);
                if let Some(metrics) = &self.inner.metrics {
                    metrics.record_channel_backlog(-1);
                }
                return Some(value);
            }

            if *self.inner.closed.lock() {
                return None;
            }

            self.inner.condvar.wait(&mut queue);
        }
    }

    /// Try to receive a value without blocking. Returns None if no value is available.
    pub fn try_recv(&self) -> Option<T> {
        let mut queue = self.inner.queue.lock();
        if queue.is_empty() {
            None
        } else {
            let value = queue.remove(0);
            if let Some(metrics) = &self.inner.metrics {
                metrics.record_channel_backlog(-1);
            }
            Some(value)
        }
    }

    /// Receive a value asynchronously, registering a waker for when data becomes available.
    /// Returns `Ok(value)` if data is available, or `Err(waker)` if the caller should register
    /// the waker and suspend the task.
    pub fn recv_async(&self, waker: &Waker) -> Result<T, Waker> {
        let mut queue = self.inner.queue.lock();

        if let Some(value) = queue.pop() {
            if let Some(metrics) = &self.inner.metrics {
                metrics.record_channel_backlog(-1);
            }
            Ok(value)
        } else if *self.inner.closed.lock() {
            Err(waker.clone())
        } else {
            // Register waker and return pending
            self.inner.receiver_wakers.lock().push(waker.clone());
            Err(waker.clone())
        }
    }

    /// Check if the channel is closed.
    pub fn is_closed(&self) -> bool {
        *self.inner.closed.lock()
    }

    /// Close the channel, waking all waiting receivers.
    pub fn close(&self) {
        let mut closed = self.inner.closed.lock();
        if *closed {
            return;
        }
        *closed = true;

        // Wake all task-aware wakers
        let mut wakers = self.inner.receiver_wakers.lock();
        for waker in wakers.drain(..) {
            waker.wake();
        }

        // Wake blocking receivers
        self.inner.condvar.notify_all();
    }

    /// Get the current queue length.
    pub fn len(&self) -> usize {
        self.inner.queue.lock().len()
    }

    /// Check if the channel is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.queue.lock().is_empty()
    }

    /// Legacy compatibility: create a sender handle (no-op in new implementation).
    pub fn clone_sender(&self) -> Self {
        self.clone()
    }

    /// Legacy compatibility: create a receiver handle (no-op in new implementation).
    pub fn clone_receiver(&self) -> Self {
        self.clone()
    }

    /// Register a waker for when data becomes available.
    /// This is used internally by select operations.
    pub(crate) fn register_waker(&self, waker: &Waker) {
        self.inner.receiver_wakers.lock().push(waker.clone());
    }
}

impl<T> Clone for TaskChannel<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TaskMailBox<T> {
    channel: TaskChannel<T>,
}

impl<T> TaskMailBox<T> {
    pub fn new(channel: TaskChannel<T>) -> Self {
        Self { channel }
    }

    pub fn channel(&self) -> TaskChannel<T> {
        self.channel.clone()
    }
}

/// Select operation result for choosing between two channels.
#[derive(Debug)]
pub enum SelectResult<T1, T2> {
    /// First channel had data available.
    First(T1),
    /// Second channel had data available.
    Second(T2),
}

/// Select operation between two channels.
/// This is a two-way select implementation as specified in Phase 3.
/// Returns immediately if data is available on either channel, otherwise
/// registers wakers on both channels and returns None (caller should suspend).
pub fn select2<T1, T2>(
    ch1: &TaskChannel<T1>,
    ch2: &TaskChannel<T2>,
    waker: &Waker,
) -> Result<SelectResult<T1, T2>, ()> {
    // Try to receive from both channels immediately
    if let Some(value1) = ch1.try_recv() {
        return Ok(SelectResult::First(value1));
    }
    if let Some(value2) = ch2.try_recv() {
        return Ok(SelectResult::Second(value2));
    }

    // Both channels are empty, register wakers on both
    ch1.register_waker(waker);
    ch2.register_waker(waker);

    // Return pending (caller should suspend)
    Err(())
}

/// Select operation between two channels with async support.
/// Returns Ok(result) if data is available, Err(waker) if caller should suspend.
pub fn select2_async<T1, T2>(
    ch1: &TaskChannel<T1>,
    ch2: &TaskChannel<T2>,
    waker: &Waker,
) -> Result<SelectResult<T1, T2>, Waker> {
    // Try to receive from both channels immediately
    if let Some(value1) = ch1.try_recv() {
        return Ok(SelectResult::First(value1));
    }
    if let Some(value2) = ch2.try_recv() {
        return Ok(SelectResult::Second(value2));
    }

    // Both channels are empty, register wakers on both
    ch1.register_waker(waker);
    ch2.register_waker(waker);

    // Return pending
    Err(waker.clone())
}
