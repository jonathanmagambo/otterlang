//! Object trait and header for managed memory

use std::sync::atomic::{AtomicUsize, Ordering};

/// Base trait for all OtterLang objects that can be reference counted
pub trait OtterObject: Send + Sync {
    /// Called when the object is cloned (reference count incremented)
    fn on_clone(&self) {}

    /// Called when the object is dropped (reference count decremented)
    fn on_drop(&self) {}

    /// Get the type name of the object for debugging
    fn type_name(&self) -> &'static str {
        "OtterObject"
    }

    /// Get the size of the object in bytes
    fn size(&self) -> usize {
        0
    }
}

/// Header for reference-counted objects
#[derive(Debug)]
pub struct ObjectHeader {
    /// Reference count (number of strong references)
    pub ref_count: AtomicUsize,
    /// Weak reference count
    pub weak_count: AtomicUsize,
    /// Type name for debugging
    pub type_name: &'static str,
    /// Size in bytes
    pub size: usize,
}

impl ObjectHeader {
    pub fn new(type_name: &'static str, size: usize) -> Self {
        Self {
            ref_count: AtomicUsize::new(1),
            weak_count: AtomicUsize::new(0),
            type_name,
            size,
        }
    }

    pub fn increment_ref(&self) -> usize {
        self.ref_count.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn decrement_ref(&self) -> usize {
        let prev = self.ref_count.fetch_sub(1, Ordering::SeqCst);
        prev.saturating_sub(1)
    }

    pub fn ref_count(&self) -> usize {
        self.ref_count.load(Ordering::SeqCst)
    }

    pub fn increment_weak(&self) -> usize {
        self.weak_count.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn decrement_weak(&self) -> usize {
        let prev = self.weak_count.fetch_sub(1, Ordering::SeqCst);
        prev.saturating_sub(1)
    }

    pub fn weak_count(&self) -> usize {
        self.weak_count.load(Ordering::SeqCst)
    }
}
