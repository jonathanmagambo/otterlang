//! Reference counting for OtterLang objects

use std::ptr::NonNull;
use std::sync::atomic::Ordering;

use crate::runtime::memory::object::{ObjectHeader, OtterObject};

/// Reference-counted pointer to an OtterLang object
pub struct RcOtter<T: OtterObject> {
    ptr: NonNull<T>,
    header: NonNull<ObjectHeader>,
}

unsafe impl<T: OtterObject> Send for RcOtter<T> {}
unsafe impl<T: OtterObject> Sync for RcOtter<T> {}

impl<T: OtterObject> RcOtter<T> {
    /// Create a new reference-counted object
    pub fn new(obj: T) -> Self {
        let size = obj.size();
        let type_name = obj.type_name();
        let header = Box::new(ObjectHeader::new(type_name, size));
        let header_ptr = NonNull::new(Box::into_raw(header)).unwrap();

        let boxed = Box::new(obj);
        let ptr = NonNull::new(Box::into_raw(boxed)).unwrap();

        Self {
            ptr,
            header: header_ptr,
        }
    }

    /// Get the reference count
    pub fn ref_count(&self) -> usize {
        unsafe { self.header.as_ref().ref_count() }
    }

    /// Get the weak reference count
    pub fn weak_count(&self) -> usize {
        unsafe { self.header.as_ref().weak_count() }
    }

    /// Get a raw pointer to the object (for FFI)
    pub fn as_ptr(&self) -> *const T {
        self.ptr.as_ptr()
    }

    /// Get a mutable raw pointer to the object (for FFI)
    pub fn as_mut_ptr(&self) -> *mut T {
        self.ptr.as_ptr()
    }
}

impl<T: OtterObject> Clone for RcOtter<T> {
    fn clone(&self) -> Self {
        unsafe {
            let header = self.header.as_ref();
            header.increment_ref();
        }
        Self {
            ptr: self.ptr,
            header: self.header,
        }
    }
}

impl<T: OtterObject> std::ops::Deref for RcOtter<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.ptr.as_ptr() }
    }
}

impl<T: OtterObject> Drop for RcOtter<T> {
    fn drop(&mut self) {
        unsafe {
            let header = self.header.as_ref();
            let count = header.decrement_ref();

            if count == 0 {
                // Last reference dropped, destroy the object
                let _ = Box::from_raw(self.ptr.as_ptr());

                // If there are no weak references, destroy the header
                if header.weak_count() == 0 {
                    let _ = Box::from_raw(self.header.as_ptr());
                }
            }
        }
    }
}

/// Weak reference to a reference-counted object
pub struct WeakOtter<T: OtterObject> {
    ptr: NonNull<T>,
    header: NonNull<ObjectHeader>,
}

unsafe impl<T: OtterObject> Send for WeakOtter<T> {}
unsafe impl<T: OtterObject> Sync for WeakOtter<T> {}

impl<T: OtterObject> WeakOtter<T> {
    /// Create a weak reference from a strong reference
    pub fn new(rc: &RcOtter<T>) -> Self {
        unsafe {
            let header = rc.header.as_ref();
            header.increment_weak();
        }
        Self {
            ptr: rc.ptr,
            header: rc.header,
        }
    }

    /// Try to upgrade to a strong reference
    pub fn upgrade(&self) -> Option<RcOtter<T>> {
        unsafe {
            let header = self.header.as_ref();
            let mut count = header.ref_count();

            // Increment if not zero
            loop {
                if count == 0 {
                    return None;
                }

                match header.ref_count.compare_exchange_weak(
                    count,
                    count + 1,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(x) => count = x,
                }
            }

            Some(RcOtter {
                ptr: self.ptr,
                header: self.header,
            })
        }
    }

    /// Get the reference count (may be 0 if object was dropped)
    pub fn ref_count(&self) -> usize {
        unsafe { self.header.as_ref().ref_count() }
    }
}

impl<T: OtterObject> Clone for WeakOtter<T> {
    fn clone(&self) -> Self {
        unsafe {
            self.header.as_ref().increment_weak();
        }
        Self {
            ptr: self.ptr,
            header: self.header,
        }
    }
}

impl<T: OtterObject> Drop for WeakOtter<T> {
    fn drop(&mut self) {
        unsafe {
            let header = self.header.as_ref();
            let weak_count = header.decrement_weak();

            // If both ref count and weak count are zero, destroy header
            if weak_count == 0 && header.ref_count() == 0 {
                let _ = Box::from_raw(self.header.as_ptr());
            }
        }
    }
}
