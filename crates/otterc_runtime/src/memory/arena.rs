use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use once_cell::sync::Lazy;
use parking_lot::RwLock;

use super::allocator::BumpAllocator;

/// Simple arena backed by a bump allocator. Allocations live until the arena is reset
/// or destroyed.
pub struct Arena {
    allocator: BumpAllocator,
}

impl Arena {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(4 * 1024);
        Self {
            allocator: BumpAllocator::new(capacity),
        }
    }

    pub fn alloc(&self, size: usize, align: usize) -> Option<*mut u8> {
        self.allocator.alloc(size, align)
    }

    pub fn reset(&self) {
        self.allocator.reset();
    }
}

static NEXT_ARENA_ID: AtomicU64 = AtomicU64::new(1);
static ARENAS: Lazy<RwLock<HashMap<u64, Arc<Arena>>>> = Lazy::new(|| RwLock::new(HashMap::new()));

fn next_id() -> u64 {
    NEXT_ARENA_ID.fetch_add(1, Ordering::SeqCst)
}

pub fn create_arena(capacity: usize) -> u64 {
    let arena = Arc::new(Arena::new(capacity));
    let handle = next_id();
    ARENAS.write().insert(handle, arena);
    handle
}

pub fn destroy_arena(handle: u64) -> bool {
    ARENAS.write().remove(&handle).is_some()
}

pub fn arena_alloc(handle: u64, size: usize, align: usize) -> Option<*mut u8> {
    ARENAS
        .read()
        .get(&handle)
        .and_then(|arena| arena.alloc(size, align.max(1)))
}

pub fn reset_arena(handle: u64) -> bool {
    if let Some(arena) = ARENAS.read().get(&handle).cloned() {
        arena.reset();
        true
    } else {
        false
    }
}
