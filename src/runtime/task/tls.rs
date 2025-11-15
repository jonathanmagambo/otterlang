//! Task-local storage implementation.
//!
//! Provides task-local storage similar to thread-local storage, where each task
//! has its own isolated storage space.

use parking_lot::Mutex;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use super::task::TaskId;

/// Task-local storage container.
#[derive(Debug, Clone)]
pub struct TaskLocalStorage {
    storage: Arc<Mutex<HashMap<TypeId, Box<dyn Any + Send + Sync>>>>,
}

impl TaskLocalStorage {
    pub fn new() -> Self {
        Self {
            storage: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Set a value in task-local storage.
    pub fn set<T: Send + Sync + 'static>(&self, value: T) {
        let mut storage = self.storage.lock();
        storage.insert(TypeId::of::<T>(), Box::new(value));
    }

    /// Get a value from task-local storage.
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<T>
    where
        T: Clone,
    {
        let storage = self.storage.lock();
        storage
            .get(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_ref::<T>())
            .cloned()
    }

    /// Get a reference to a value in task-local storage (without cloning).
    pub fn get_ref<T: Send + Sync + 'static>(
        &self,
    ) -> Option<std::sync::LockResult<parking_lot::MutexGuard<'_, T>>>
    where
        T: 'static,
    {
        // For non-cloneable types, we wrap them in Mutex
        // This is a simplified version - in practice you'd want more sophisticated handling
        None
    }

    /// Remove a value from task-local storage.
    pub fn remove<T: Send + Sync + 'static>(&self) -> Option<T> {
        let mut storage = self.storage.lock();
        storage
            .remove(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast::<T>().ok())
            .map(|boxed| *boxed)
    }

    /// Clear all task-local storage.
    pub fn clear(&self) {
        let mut storage = self.storage.lock();
        storage.clear();
    }

    /// Check if a value exists in task-local storage.
    pub fn contains<T: Send + Sync + 'static>(&self) -> bool {
        let storage = self.storage.lock();
        storage.contains_key(&TypeId::of::<T>())
    }
}

impl Default for TaskLocalStorage {
    fn default() -> Self {
        Self::new()
    }
}

/// Global registry mapping task IDs to their task-local storage.
#[derive(Debug)]
pub struct TaskLocalRegistry {
    registry: Arc<Mutex<HashMap<TaskId, TaskLocalStorage>>>,
}

impl TaskLocalRegistry {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get or create task-local storage for a task.
    pub fn get_or_create(&self, task_id: TaskId) -> TaskLocalStorage {
        let mut registry = self.registry.lock();
        registry
            .entry(task_id)
            .or_default()
            .clone()
    }

    /// Get task-local storage for a task, if it exists.
    pub fn get(&self, task_id: TaskId) -> Option<TaskLocalStorage> {
        let registry = self.registry.lock();
        registry.get(&task_id).cloned()
    }

    /// Remove task-local storage for a task.
    pub fn remove(&self, task_id: TaskId) {
        let mut registry = self.registry.lock();
        registry.remove(&task_id);
    }

    /// Clear all task-local storage.
    pub fn clear(&self) {
        let mut registry = self.registry.lock();
        registry.clear();
    }
}

impl Default for TaskLocalRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global task-local storage registry.
static TASK_LOCAL_REGISTRY: once_cell::sync::Lazy<TaskLocalRegistry> =
    once_cell::sync::Lazy::new(TaskLocalRegistry::new);

/// Get task-local storage for the current task.
///
/// # Safety
/// This function requires that the current task ID is available through some mechanism.
/// In practice, this would be set by the scheduler when running a task.
pub fn get_task_local_storage(task_id: TaskId) -> TaskLocalStorage {
    TASK_LOCAL_REGISTRY.get_or_create(task_id)
}

/// Remove task-local storage for a task.
pub fn cleanup_task_local_storage(task_id: TaskId) {
    TASK_LOCAL_REGISTRY.remove(task_id);
}
