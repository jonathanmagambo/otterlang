//! Garbage Collection FFI bindings

use crate::memory::{arena, get_gc};

/// Allocate memory on the heap managed by the GC
///
/// # Safety
/// This function is unsafe because it returns a raw pointer
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_alloc(size: i64) -> *mut u8 {
    let gc = get_gc();

    // Try to allocate using the current GC strategy
    if let Some(ptr) = gc.alloc(size as usize) {
        ptr
    } else {
        // Fallback to system allocator if GC allocation fails (shouldn't happen with proper GC)
        unsafe { std::alloc::alloc(std::alloc::Layout::from_size_align(size as usize, 8).unwrap()) }
    }
}

/// Add a root object to the GC
///
/// # Safety
/// Caller must ensure `ptr` points to a valid GC-managed object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_gc_add_root(ptr: *mut u8) {
    get_gc().add_root(ptr as usize);
}

/// Remove a root object from the GC
///
/// # Safety
/// Caller must ensure `ptr` was previously registered as a root.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_gc_remove_root(ptr: *mut u8) {
    get_gc().remove_root(ptr as usize);
}

/// Enable garbage collection. Returns previous GC state.
///
/// # Safety
/// This function is safe to call from any context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_gc_enable() -> bool {
    get_gc().enable()
}

/// Disable garbage collection. Returns previous GC state.
///
/// # Safety
/// This function is safe to call from any context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_gc_disable() -> bool {
    get_gc().disable()
}

/// Query whether garbage collection is currently enabled.
///
/// # Safety
/// This function is safe to call from any context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_gc_is_enabled() -> bool {
    get_gc().is_enabled()
}

/// Create a dedicated arena allocator and return its handle.
///
/// # Safety
/// This function is safe to call from any context. The returned handle must be properly managed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_arena_create(capacity: i64) -> u64 {
    let capacity = if capacity <= 0 {
        64 * 1024 // default 64KB
    } else {
        capacity as usize
    };
    arena::create_arena(capacity)
}

/// Destroy a previously created arena.
///
/// # Safety
/// The handle must be a valid arena handle returned by `otter_arena_create`.
/// After calling this function, the handle becomes invalid and must not be used.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_arena_destroy(handle: u64) -> bool {
    arena::destroy_arena(handle)
}

/// Allocate bytes from an arena.
///
/// # Safety
/// - The handle must be a valid arena handle returned by `otter_arena_create`.
/// - The returned pointer is valid until the arena is destroyed or reset.
/// - The caller is responsible for not accessing the pointer after arena destruction/reset.
/// - The allocated memory is uninitialized and must be properly initialized before use.
/// - Size and alignment must be reasonable (size > 0, alignment is a power of 2).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_arena_alloc(handle: u64, size: i64, align: i64) -> *mut u8 {
    if size <= 0 {
        return std::ptr::null_mut();
    }
    let align = if align <= 0 { 8 } else { align as usize };
    arena::arena_alloc(handle, size as usize, align).unwrap_or(std::ptr::null_mut())
}

/// Reset an arena, freeing all allocations at once.
///
/// # Safety
/// - The handle must be a valid arena handle returned by `otter_arena_create`.
/// - After calling this function, all pointers previously returned by `otter_arena_alloc`
///   for this arena become invalid and must not be accessed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn otter_arena_reset(handle: u64) -> bool {
    arena::reset_arena(handle)
}
