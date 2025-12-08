# OtterLang Garbage Collection Guide

OtterLang ships several garbage collectors, arena allocators, and root-registration helpers that can be tuned both from the CLI and from FFI code. This guide summarizes the knobs that are available today and how to use them safely.

## 1. Choosing a Collector

Every `otter` command accepts GC flags so you can switch collectors per run:

```
otter run hello.ot --gc-strategy mark-sweep --gc-threshold 0.9 \
    --gc-interval-ms 0 --gc-disabled-max-bytes 134217728
```

The same settings can be supplied via environment variables:

| Flag | Env var | Description |
|------|---------|-------------|
| `--gc-strategy` | `OTTER_GC_STRATEGY` | `rc`, `mark-sweep`, `generational`, or `none` |
| `--gc-threshold` | `OTTER_GC_THRESHOLD` | Heap usage fraction that triggers a collection |
| `--gc-interval-ms` | `OTTER_GC_INTERVAL` | Minimum interval between automatic cycles |
| `--gc-disabled-max-bytes` | `OTTER_GC_DISABLED_MAX_BYTES` | Allocation budget while GC is disabled |

When the CLI flags are omitted, the runtime honors the environment variables. If neither is present the defaults from `GcConfig::default()` apply (generational GC with an 80% threshold).

## 2. Working with the GC from Otter code

The `runtime` module exposes inspector helpers:

```otter
use runtime

fn main():
    println(f"GC enabled? {runtime.collect_garbage()}")
    runtime.collect_garbage()  # Force a collection
    println(f"Heap bytes: {runtime.memory()}")
```

Keep long-lived data in normal Otter values and let the collector manage it. If you temporarily disable the GC (e.g., via FFI) make sure to re-enable it and honor the `OTTER_GC_DISABLED_MAX_BYTES` limit to avoid exhausting memory.

## 3. Root management from FFI

When interoperating with Rust/C code you must pin references that the GC cannot see. The runtime exports the following functions from `src/runtime/stdlib/gc.rs`:

| Symbol | Purpose |
|--------|---------|
| `otter_gc_add_root(ptr)` | Register a GC-managed pointer as a root so it will not be collected. |
| `otter_gc_remove_root(ptr)` | Remove a previously registered root. |
| `otter_gc_enable()` / `otter_gc_disable()` / `otter_gc_is_enabled()` | Toggle collection globally. |

Example (Rust FFI):

```rust
#[no_mangle]
pub extern "C" fn hold_buffer(buf: *mut u8) {
    unsafe {
        otter_gc_add_root(buf);
        // store buf in a global structure known only to Rust
    }
}

#[no_mangle]
pub extern "C" fn release_buffer(buf: *mut u8) {
    unsafe {
        // drop from the Rust structure first, then inform the GC
        otter_gc_remove_root(buf);
    }
}
```

Always remove roots once you stop using them; otherwise the GC will keep the objects alive forever.

## 4. Arena allocators

For deterministic lifetimes you can allocate out of bump-pointer arenas. They never participate in GC and are freed when you reset or destroy the arena.

Available helpers:

| Symbol | Description |
|--------|-------------|
| `otter_arena_create(capacity)` | Returns an arena handle with the requested capacity (defaults to 64 KB). |
| `otter_arena_alloc(handle, size, align)` | Allocates raw bytes from the arena. The memory is uninitialised. |
| `otter_arena_reset(handle)` | Drops all allocations in the arena without freeing the arena itself. |
| `otter_arena_destroy(handle)` | Frees the arena handle and invalidates every pointer returned by it. |

Typical usage (Rust FFI):

```rust
let arena = unsafe { otter_arena_create(256 * 1024) }; // 256 KB
let ptr = unsafe { otter_arena_alloc(arena, 1024, 8) }; // 1 KB chunk
// ... use ptr while the arena lives ...
unsafe { otter_arena_reset(arena) }; // drop everything allocated in the arena
unsafe { otter_arena_destroy(arena) }; // final cleanup
```

Never read/write through an arena pointer after calling `reset` or `destroy`.

## 5. Best practices

1. **Prefer the default collector** unless you have workload-specific data showing that another strategy wins.
2. **Minimize the time GC is disabled.** Treat `otter_gc_disable()` as a scoped guard in FFI code.
3. **Register every external root** as soon as you store a GC pointer outside Otter’s heap.
4. **Reset arenas frequently** to keep their footprint bounded and to avoid aliasing freed memory.
5. **Monitor runtime metrics.** `runtime.stats()` and `runtime.memory()` help you confirm your collector settings.

These conventions prevent dangling pointers when interfacing with native code and keep long-lived REPL or server sessions from leaking memory.
