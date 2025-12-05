//! Memory management system for OtterLang
//!
//! Provides reference counting, garbage collection, and memory profiling

pub mod allocator;
pub mod arena;
pub mod config;
pub mod gc;
pub mod object;
pub mod profiler;
pub mod rc;

pub use config::{GcConfig, GcStrategy};
pub use gc::{GcStats, GcStrategyTrait, GenerationalGC, MarkSweepGC, RcGC, get_gc};
pub use object::OtterObject;
pub use profiler::{AllocationInfo, MemoryProfiler};
pub use rc::{RcOtter, WeakOtter};
