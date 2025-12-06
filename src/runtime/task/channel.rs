use anyhow::{Result, bail};
use parking_lot::{Condvar, Mutex};
use std::collections::VecDeque;
use std::sync::Arc;
use std::task::Waker;

use super::metrics::TaskRuntimeMetrics;

#[derive(Debug)]
pub struct TaskChannel<T> {
    inner: Arc<ChannelInner<T>>,
}

#[derive(Debug)]
struct ChannelInner<T> {
    queue: Mutex<VecDeque<T>>,
    receiver_wakers: Mutex<Vec<Waker>>,
    metrics: Option<Arc<TaskRuntimeMetrics>>,
    closed: Mutex<bool>,
    condvar: Condvar,
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
                queue: Mutex::new(VecDeque::new()),
                receiver_wakers: Mutex::new(Vec::new()),
                metrics,
                closed: Mutex::new(false),
                condvar: Condvar::new(),
            }),
        }
    }

    /// Send a value to the channel, waking any waiting receivers.
    pub fn send(&self, value: T) {
        {
            let mut queue = self.inner.queue.lock();
            queue.push_back(value);
        }

        if let Some(metrics) = &self.inner.metrics {
            metrics.record_channel_backlog(1);
        }

        // Wake up one waiting receiver (task-aware)
        if let Some(waker) = self.take_next_waker() {
            if let Some(metrics) = &self.inner.metrics {
                metrics.record_channel_waiters(-1);
            }
            waker.wake();
        }

        // Also notify condvar for blocking recv
        self.inner.condvar.notify_one();
    }

    /// Receive a value, blocking if none is available.
    /// This is a legacy blocking API. For task-aware code, use `recv_async` instead.
    pub fn recv(&self) -> Option<T> {
        // For blocking recv, use condvar to wait for data
        let mut queue = self.inner.queue.lock();

        loop {
            if let Some(value) = queue.pop_front() {
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
        let value = queue.pop_front();
        if value.is_some()
            && let Some(metrics) = &self.inner.metrics
        {
            metrics.record_channel_backlog(-1);
        }
        value
    }

    /// Receive a value asynchronously, registering a waker for when data becomes available.
    /// Returns `Ok(value)` if data is available, or `Err(waker)` if the caller should register
    /// the waker and suspend the task.
    pub fn recv_async(&self, waker: &Waker) -> Result<T, Waker> {
        if let Some(value) = self.take_next_value() {
            if let Some(metrics) = &self.inner.metrics {
                metrics.record_channel_backlog(-1);
            }
            return Ok(value);
        }

        if self.is_closed() {
            return Err(waker.clone());
        }

        // Register waker and return pending
        self.register_waker(waker);
        Err(waker.clone())
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
        let pending = wakers.len() as i64;
        for waker in wakers.drain(..) {
            waker.wake();
        }
        if pending > 0
            && let Some(metrics) = &self.inner.metrics
        {
            metrics.record_channel_waiters(-pending);
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
        let mut wakers = self.inner.receiver_wakers.lock();
        if wakers.iter().any(|existing| existing.will_wake(waker)) {
            return;
        }
        wakers.push(waker.clone());
        if let Some(metrics) = &self.inner.metrics {
            metrics.record_channel_waiters(1);
        }
    }

    fn take_next_value(&self) -> Option<T> {
        let mut queue = self.inner.queue.lock();
        queue.pop_front()
    }

    fn take_next_waker(&self) -> Option<Waker> {
        let mut wakers = self.inner.receiver_wakers.lock();
        wakers.pop()
    }

    #[cfg(test)]
    fn pending_wakers(&self) -> usize {
        self.inner.receiver_wakers.lock().len()
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
) -> Result<SelectResult<T1, T2>> {
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
    bail!("Both channels are empty; return pending")
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{RawWaker, RawWakerVTable, Waker};
    use std::thread;
    use std::time::Duration;

    static TEST_WAKER_VTABLE: RawWakerVTable =
        RawWakerVTable::new(test_clone, test_wake, test_wake_by_ref, test_drop);

    #[test]
    fn fifo_ordering() {
        let channel = TaskChannel::new();
        channel.send(1);
        channel.send(2);

        assert_eq!(channel.try_recv(), Some(1));
        assert_eq!(channel.try_recv(), Some(2));
    }

    #[test]
    fn close_before_send_unblocks_blocking_recv() {
        let channel: TaskChannel<()> = TaskChannel::new();
        let closer = channel.clone();
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            closer.close();
        });

        assert!(channel.recv().is_none());
        handle.join().unwrap();
    }

    #[test]
    fn waker_registration_deduplicated_and_drained_on_close() {
        let channel = TaskChannel::<i32>::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let waker = test_waker(counter.clone());

        assert_eq!(channel.pending_wakers(), 0);

        assert!(channel.recv_async(&waker).is_err());
        assert_eq!(channel.pending_wakers(), 1);

        // Registering the same waker again should be a no-op
        assert!(channel.recv_async(&waker).is_err());
        assert_eq!(channel.pending_wakers(), 1);

        channel.close();

        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert_eq!(channel.pending_wakers(), 0);
    }

    fn test_waker(counter: Arc<AtomicUsize>) -> Waker {
        unsafe {
            Waker::from_raw(RawWaker::new(
                Arc::into_raw(counter) as *const (),
                &TEST_WAKER_VTABLE,
            ))
        }
    }

    unsafe fn test_clone(data: *const ()) -> RawWaker {
        let arc = unsafe { Arc::<AtomicUsize>::from_raw(data as *const AtomicUsize) };
        let cloned = arc.clone();
        std::mem::forget(arc);
        RawWaker::new(Arc::into_raw(cloned) as *const (), &TEST_WAKER_VTABLE)
    }

    unsafe fn test_wake(data: *const ()) {
        let arc = unsafe { Arc::<AtomicUsize>::from_raw(data as *const AtomicUsize) };
        arc.fetch_add(1, Ordering::SeqCst);
    }

    unsafe fn test_wake_by_ref(data: *const ()) {
        let arc = unsafe { Arc::<AtomicUsize>::from_raw(data as *const AtomicUsize) };
        arc.fetch_add(1, Ordering::SeqCst);
        std::mem::forget(arc);
    }

    unsafe fn test_drop(data: *const ()) {
        drop(unsafe { Arc::<AtomicUsize>::from_raw(data as *const AtomicUsize) });
    }
}
