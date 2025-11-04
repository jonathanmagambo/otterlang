//! Memory management system for OtterLang
//!
//! Provides reference counting, garbage collection, and memory profiling

pub mod config;
pub mod gc;
pub mod object;
pub mod profiler;
pub mod rc;

pub use config::{GcConfig, GcStrategy};
pub use gc::{GcStrategyTrait, MarkSweepGC, RcGC};
pub use object::OtterObject;
pub use profiler::{AllocationInfo, MemoryProfiler};
pub use rc::{RcOtter, WeakOtter};
