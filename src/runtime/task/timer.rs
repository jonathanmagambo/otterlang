//! Timer wheel for delayed task wakeups.
//!
//! Provides a timer wheel that integrates with the task scheduler to wake
//! tasks after a specified delay without blocking OS threads.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Mutex;
use std::task::Waker;
use std::time::{Duration, Instant};

/// Timer entry that stores when a waker should be notified.
#[derive(Debug)]
struct TimerEntry {
    deadline: Instant,
    waker: Waker,
}

impl PartialEq for TimerEntry {
    fn eq(&self, other: &Self) -> bool {
        self.deadline == other.deadline
    }
}

impl Eq for TimerEntry {}

impl PartialOrd for TimerEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // Reverse ordering so earliest deadline is at the top
        Some(other.deadline.cmp(&self.deadline))
    }
}

impl Ord for TimerEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering so earliest deadline is at the top
        other.deadline.cmp(&self.deadline)
    }
}

/// Timer wheel for managing delayed wakeups.
#[derive(Debug)]
pub struct TimerWheel {
    timers: Mutex<BinaryHeap<TimerEntry>>,
}

impl TimerWheel {
    pub fn new() -> Self {
        Self {
            timers: Mutex::new(BinaryHeap::new()),
        }
    }

    /// Schedule a waker to be notified after the specified duration.
    pub fn schedule_wakeup(&self, delay: Duration, waker: Waker) {
        let deadline = Instant::now() + delay;
        let entry = TimerEntry { deadline, waker };
        self.timers.lock().unwrap().push(entry);
    }

    /// Schedule a waker to be notified at a specific instant.
    pub fn schedule_at(&self, deadline: Instant, waker: Waker) {
        let entry = TimerEntry { deadline, waker };
        self.timers.lock().unwrap().push(entry);
    }

    /// Process all expired timers, waking their associated wakers.
    /// Returns the duration until the next timer expires, or None if no timers are scheduled.
    pub fn process_expired(&self) -> Option<Duration> {
        let mut timers = self.timers.lock().unwrap();
        let now = Instant::now();
        let mut next_deadline = None;

        while let Some(entry) = timers.peek() {
            if entry.deadline <= now {
                let entry = timers.pop().unwrap();
                entry.waker.wake();
            } else {
                next_deadline = Some(entry.deadline);
                break;
            }
        }

        next_deadline.map(|deadline| {
            let delay = deadline.duration_since(now);
            delay.max(Duration::from_millis(0))
        })
    }

    /// Get the duration until the next timer expires, or None if no timers are scheduled.
    pub fn next_timeout(&self) -> Option<Duration> {
        let timers = self.timers.lock().unwrap();
        timers.peek().map(|entry| {
            let delay = entry.deadline.duration_since(Instant::now());
            delay.max(Duration::from_millis(0))
        })
    }

    /// Check if there are any pending timers.
    pub fn has_pending(&self) -> bool {
        !self.timers.lock().unwrap().is_empty()
    }

    /// Clear all pending timers.
    pub fn clear(&self) {
        self.timers.lock().unwrap().clear();
    }
}

impl Default for TimerWheel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::task::{RawWaker, RawWakerVTable};

    fn create_test_waker(flag: Arc<AtomicBool>) -> Waker {
        unsafe fn clone(data: *const ()) -> RawWaker {
            RawWaker::new(data, &VTABLE)
        }

        unsafe fn wake(data: *const ()) {
            let flag = Arc::from_raw(data as *const AtomicBool);
            flag.store(true, Ordering::SeqCst);
            // Note: Arc::from_raw takes ownership, but since we're in a test waker
            // that never actually drops the Arc (the drop function is a no-op),
            // we need to forget it to avoid double-free
            std::mem::forget(flag);
        }

        unsafe fn wake_by_ref(data: *const ()) {
            // For wake_by_ref, we don't take ownership, so just use the pointer directly
            let flag = &*(data as *const AtomicBool);
            flag.store(true, Ordering::SeqCst);
        }

        unsafe fn drop(_data: *const ()) {}

        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop);

        let flag_ptr = Arc::into_raw(flag);
        unsafe { Waker::from_raw(RawWaker::new(flag_ptr as *const (), &VTABLE)) }
    }

    #[test]
    fn test_timer_schedule_and_process() {
        let wheel = TimerWheel::new();
        let flag1 = Arc::new(AtomicBool::new(false));
        let flag2 = Arc::new(AtomicBool::new(false));

        // Record start time to track elapsed time precisely
        let start = Instant::now();
        let waker1 = create_test_waker(Arc::clone(&flag1));
        let waker2 = create_test_waker(Arc::clone(&flag2));

        // Schedule two wakeups with longer delays to avoid timing issues
        wheel.schedule_wakeup(Duration::from_millis(200), waker1);
        wheel.schedule_wakeup(Duration::from_millis(400), waker2);

        // Process immediately - should not wake yet
        wheel.process_expired();
        assert!(!flag1.load(Ordering::SeqCst));
        assert!(!flag2.load(Ordering::SeqCst));

        // Wait for first timer to expire - wait until we've passed the deadline
        while start.elapsed() < Duration::from_millis(250) {
            std::thread::sleep(Duration::from_millis(10));
            wheel.process_expired();
        }

        // Small delay to ensure waker flags are updated
        std::thread::sleep(Duration::from_millis(20));

        // First timer should have fired, second should not
        assert!(
            flag1.load(Ordering::SeqCst),
            "First timer should have fired"
        );
        assert!(
            !flag2.load(Ordering::SeqCst),
            "Second timer should not have fired yet (only {}ms elapsed)",
            start.elapsed().as_millis()
        );

        // Wait for second timer to expire - wait until we've passed the deadline
        while start.elapsed() < Duration::from_millis(450) {
            std::thread::sleep(Duration::from_millis(10));
            wheel.process_expired();
        }

        // Small delay to ensure waker flags are updated
        std::thread::sleep(Duration::from_millis(20));

        // Both should have fired
        assert!(flag1.load(Ordering::SeqCst));
        assert!(
            flag2.load(Ordering::SeqCst),
            "Second timer should have fired"
        );
    }

    #[test]
    fn test_timer_next_timeout() {
        let wheel = TimerWheel::new();
        let flag = Arc::new(AtomicBool::new(false));
        let waker = create_test_waker(Arc::clone(&flag));

        assert_eq!(wheel.next_timeout(), None);

        wheel.schedule_wakeup(Duration::from_millis(100), waker);

        let timeout = wheel.next_timeout();
        assert!(timeout.is_some());
        let timeout = timeout.unwrap();
        assert!(timeout <= Duration::from_millis(100));
        assert!(timeout >= Duration::from_millis(50)); // Allow some tolerance
    }
}
