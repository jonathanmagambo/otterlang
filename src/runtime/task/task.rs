use parking_lot::{Condvar, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::Waker;

/// Unique identifier assigned to each task at creation time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(u64);

impl TaskId {
    pub fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub fn raw(&self) -> u64 {
        self.0
    }
}

static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(1);

fn next_task_id() -> TaskId {
    TaskId::new(NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed))
}

pub type TaskFn = Box<dyn FnOnce() + Send + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
    Completed,
    Cancelled,
}

/// Cancellation token shared between task and join handle.
#[derive(Debug, Clone)]
pub struct CancellationToken {
    cancelled: Arc<std::sync::atomic::AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::Acquire)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared synchronization primitive used by join handles.
#[derive(Debug)]
pub struct JoinState {
    inner: Mutex<JoinInner>,
    condvar: Condvar,
}

#[derive(Debug)]
struct JoinInner {
    completed: bool,
    waiters: Vec<Waker>,
}

impl JoinState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(JoinInner {
                completed: false,
                waiters: Vec::new(),
            }),
            condvar: Condvar::new(),
        })
    }

    pub fn mark_complete(&self) {
        let mut inner = self.inner.lock();
        if inner.completed {
            return;
        }
        inner.completed = true;
        for waker in inner.waiters.drain(..) {
            waker.wake();
        }
        self.condvar.notify_all();
    }

    pub fn is_complete(&self) -> bool {
        self.inner.lock().completed
    }

    pub fn wait_blocking(&self) {
        let mut inner = self.inner.lock();
        while !inner.completed {
            self.condvar.wait(&mut inner);
        }
    }

    pub fn register_waker(&self, waker: &Waker) -> bool {
        let mut inner = self.inner.lock();
        if inner.completed {
            return true;
        }
        inner.waiters.push(waker.clone());
        false
    }
}

/// Lightweight task description executed by the scheduler.
pub struct Task {
    id: TaskId,
    name: Option<String>,
    state: TaskState,
    func: Option<TaskFn>,
    join: Arc<JoinState>,
    cancellation_token: CancellationToken,
}

impl Task {
    pub fn new(name: Option<String>, func: TaskFn) -> Self {
        Self {
            id: next_task_id(),
            name,
            state: TaskState::Ready,
            func: Some(func),
            join: JoinState::new(),
            cancellation_token: CancellationToken::new(),
        }
    }

    pub fn id(&self) -> TaskId {
        self.id
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn state(&self) -> TaskState {
        self.state
    }

    pub fn set_state(&mut self, state: TaskState) {
        self.state = state;
    }

    pub fn join_state(&self) -> Arc<JoinState> {
        Arc::clone(&self.join)
    }

    pub fn cancellation_token(&self) -> &CancellationToken {
        &self.cancellation_token
    }

    pub fn cancel(&self) {
        self.cancellation_token.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation_token.is_cancelled()
    }

    pub fn run(mut self) {
        // Check if cancelled before running
        if self.cancellation_token.is_cancelled() {
            self.state = TaskState::Cancelled;
            self.join.mark_complete();
            return;
        }

        self.state = TaskState::Running;

        // Run the function, but check for cancellation periodically
        // Note: For cooperative cancellation, tasks should check cancellation_token themselves
        if let Some(func) = self.func.take() {
            func();
        }

        // Check if cancelled after running
        if self.cancellation_token.is_cancelled() {
            self.state = TaskState::Cancelled;
        } else {
            self.state = TaskState::Completed;
        }
        self.join.mark_complete();
    }
}

pub struct JoinHandle {
    task_id: TaskId,
    state: Arc<JoinState>,
    cancellation_token: CancellationToken,
}

impl JoinHandle {
    pub fn new(
        task_id: TaskId,
        state: Arc<JoinState>,
        cancellation_token: CancellationToken,
    ) -> Self {
        Self {
            task_id,
            state,
            cancellation_token,
        }
    }

    pub fn task_id(&self) -> TaskId {
        self.task_id
    }

    pub fn is_finished(&self) -> bool {
        self.state.is_complete()
    }

    pub fn join(&self) {
        self.state.wait_blocking();
    }

    pub fn cancel(&self) {
        self.cancellation_token.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation_token.is_cancelled()
    }

    pub fn into_state(self) -> Arc<JoinState> {
        self.state
    }
}

pub struct JoinFuture {
    state: Arc<JoinState>,
}

impl JoinFuture {
    pub fn new(state: Arc<JoinState>) -> Self {
        Self { state }
    }
}

impl std::future::Future for JoinFuture {
    type Output = ();

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        if self.state.register_waker(cx.waker()) {
            std::task::Poll::Ready(())
        } else {
            std::task::Poll::Pending
        }
    }
}
