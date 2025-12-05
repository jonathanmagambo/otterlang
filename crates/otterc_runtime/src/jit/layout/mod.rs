// Runtime Data Layout Optimizer
pub mod analyzer;
pub mod optimizer;
pub mod profiler;
pub mod simd;
pub mod transformer;
pub mod validator;

pub use analyzer::CacheLocalityAnalyzer;
pub use optimizer::DataLayoutOptimizer;
pub use profiler::MemoryProfiler;
pub use simd::SimdOpportunityDetector;
pub use transformer::LayoutTransformer;
pub use validator::LayoutValidator;

pub use optimizer::{
    AccessType, CacheAnalysis, FieldAccessStats, FieldId, LayoutOptimization, MemoryLayout,
    OptimizerStats, SimdOpportunity, StructId,
};
pub use profiler::AccessPattern;
