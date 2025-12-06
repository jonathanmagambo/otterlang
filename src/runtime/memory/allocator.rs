//! Bump pointer allocator for the GC nursery

use std::alloc::{Layout, alloc, dealloc};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A simple bump pointer allocator for the nursery generation
pub struct BumpAllocator {
    memory: NonNull<u8>,
    size: usize,
    allocated: AtomicUsize,
}

impl BumpAllocator {
    /// Create a new bump allocator with the given size
    pub fn new(size: usize) -> Self {
        let layout = Layout::from_size_align(size, 8).unwrap();
        let memory = unsafe { alloc(layout) };

        if memory.is_null() {
            #[expect(clippy::panic, reason = "TODO: Use proper error handling")]
            {
                panic!("Failed to allocate nursery memory");
            }
        }

        Self {
            memory: NonNull::new(memory).unwrap(),
            size,
            allocated: AtomicUsize::new(0),
        }
    }

    /// Allocate memory from the bump pointer
    pub fn alloc(&self, size: usize, align: usize) -> Option<*mut u8> {
        let current = self.allocated.load(Ordering::Relaxed);

        // Calculate alignment padding
        let padding = (align - (current % align)) % align;
        let new_allocated = current + size + padding;

        if new_allocated > self.size {
            return None;
        }

        // Try to update the allocated pointer
        if self
            .allocated
            .compare_exchange(current, new_allocated, Ordering::SeqCst, Ordering::Relaxed)
            .is_ok()
        {
            unsafe {
                let ptr = self.memory.as_ptr().add(current + padding);
                Some(ptr)
            }
        } else {
            // Contention, retry (in a real implementation, we might loop or fail)
            // For now, just fail to trigger GC
            None
        }
    }

    /// Reset the allocator (clearing all allocations)
    pub fn reset(&self) {
        self.allocated.store(0, Ordering::SeqCst);
    }

    /// Get the start of the memory block
    pub fn start(&self) -> *mut u8 {
        self.memory.as_ptr()
    }

    /// Get the end of the currently allocated memory
    pub fn end(&self) -> *mut u8 {
        unsafe {
            self.memory
                .as_ptr()
                .add(self.allocated.load(Ordering::Relaxed))
        }
    }

    /// Get total size
    pub fn size(&self) -> usize {
        self.size
    }
}

impl Drop for BumpAllocator {
    fn drop(&mut self) {
        let layout = Layout::from_size_align(self.size, 8).unwrap();
        unsafe {
            dealloc(self.memory.as_ptr(), layout);
        }
    }
}

unsafe impl Send for BumpAllocator {}
unsafe impl Sync for BumpAllocator {}
