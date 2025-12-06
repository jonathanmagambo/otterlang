//! Runtime introspection API for querying runtime state
//!
//! This module provides a comprehensive API for inspecting the state of the
//! OtterLang runtime, including compiled functions, profiling data, memory usage,
//! GC statistics, and task scheduler state.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

use crate::runtime::jit::profiler::{CompilationProfiler, FunctionMetrics, MemoryProfiler};
use crate::runtime::jit::tiered_compiler::{CompilationTier, TieredStats};
use crate::runtime::memory::GcStats;

/// Metadata about a compiled function
#[derive(Debug, Clone)]
pub struct CompiledFunctionInfo {
    /// Function name
    pub name: String,

    /// Current compilation tier
    pub tier: CompilationTier,

    /// Number of times called
    pub call_count: u64,

    /// Number of times recompiled
    pub recompilation_count: u32,

    /// Code size in bytes
    pub code_size: usize,

    /// Whether function is currently cached
    pub is_cached: bool,

    /// Execution metrics
    pub metrics: Option<FunctionMetrics>,
}

/// Runtime statistics snapshot
#[derive(Debug, Clone)]
pub struct RuntimeSnapshot {
    /// Timestamp of snapshot (milliseconds since epoch)
    pub timestamp_ms: u64,

    /// Total number of compiled functions
    pub total_functions: usize,

    /// Functions per tier
    pub functions_per_tier: HashMap<CompilationTier, usize>,

    /// Total memory usage (bytes)
    pub total_memory_bytes: usize,

    /// Total compilation time (microseconds)
    pub total_compilation_time_us: u64,

    /// Total execution time (nanoseconds)
    pub total_execution_time_ns: u128,

    /// GC statistics
    pub gc_stats: Option<GcStats>,

    /// Number of active tasks
    pub active_tasks: usize,

    /// Cache hit rate (0.0 to 1.0)
    pub cache_hit_rate: f64,
}

/// Runtime query filters
#[derive(Debug, Clone, Default)]
pub struct QueryFilter {
    /// Filter by minimum call count
    pub min_call_count: Option<u64>,

    /// Filter by tier
    pub tier: Option<CompilationTier>,

    /// Filter by name pattern (simple substring match)
    pub name_pattern: Option<String>,

    /// Sort by field
    pub sort_by: Option<SortField>,

    /// Limit number of results
    pub limit: Option<usize>,
}

/// Fields to sort by
#[derive(Debug, Clone, Copy)]
pub enum SortField {
    Name,
    CallCount,
    Tier,
    CodeSize,
    ExecutionTime,
    MemoryUsage,
}

/// Runtime introspection interface
pub struct RuntimeIntrospector {
    /// Reference to tiered compiler (for tier info)
    tiered_compiler: Option<Arc<RwLock<crate::runtime::jit::tiered_compiler::TieredCompiler>>>,

    /// Reference to compilation profiler
    compilation_profiler: Option<Arc<RwLock<CompilationProfiler>>>,

    /// Reference to memory profiler
    memory_profiler: Option<Arc<RwLock<MemoryProfiler>>>,
}

impl RuntimeIntrospector {
    pub fn new() -> Self {
        Self {
            tiered_compiler: None,
            compilation_profiler: None,
            memory_profiler: None,
        }
    }

    /// Set the tiered compiler reference
    pub fn set_tiered_compiler(
        &mut self,
        compiler: Arc<RwLock<crate::runtime::jit::tiered_compiler::TieredCompiler>>,
    ) {
        self.tiered_compiler = Some(compiler);
    }

    /// Set the compilation profiler reference
    pub fn set_compilation_profiler(&mut self, profiler: Arc<RwLock<CompilationProfiler>>) {
        self.compilation_profiler = Some(profiler);
    }

    /// Set the memory profiler reference
    pub fn set_memory_profiler(&mut self, profiler: Arc<RwLock<MemoryProfiler>>) {
        self.memory_profiler = Some(profiler);
    }

    /// Get information about a specific function
    pub fn get_function_info(&self, function_name: &str) -> Option<CompiledFunctionInfo> {
        let mut info = CompiledFunctionInfo {
            name: function_name.to_string(),
            tier: CompilationTier::Quick,
            call_count: 0,
            recompilation_count: 0,
            code_size: 0,
            is_cached: false,
            metrics: None,
        };

        if let Some(tier_info) = self
            .tiered_compiler
            .as_ref()
            .and_then(|compiler| compiler.read().get_function_info(function_name))
        {
            info.tier = tier_info.tier;
            info.call_count = tier_info.call_count;
            info.recompilation_count = tier_info.recompilation_count;
        }

        if let Some(size) = self.compilation_profiler.as_ref().and_then(|profiler| {
            let guard = profiler.read();
            guard
                .get_function_stats(function_name)
                .and_then(|stats| stats.code_size_per_tier.get(&info.tier).copied())
        }) {
            info.code_size = size;
        }

        Some(info)
    }

    /// List all compiled functions with optional filtering
    pub fn list_functions(&self, filter: Option<QueryFilter>) -> Vec<CompiledFunctionInfo> {
        let mut functions = Vec::new();

        // Collect all function names from tiered compiler
        if let Some(ref compiler) = self.tiered_compiler {
            let all_info = compiler.read().get_all_function_info();

            for (name, tier_info) in all_info {
                let mut info = CompiledFunctionInfo {
                    name: name.clone(),
                    tier: tier_info.tier,
                    call_count: tier_info.call_count,
                    recompilation_count: tier_info.recompilation_count,
                    code_size: 0,
                    is_cached: false,
                    metrics: None,
                };

                if let Some(size) = self.compilation_profiler.as_ref().and_then(|profiler| {
                    let guard = profiler.read();
                    guard
                        .get_function_stats(&name)
                        .and_then(|stats| stats.code_size_per_tier.get(&info.tier).copied())
                }) {
                    info.code_size = size;
                }

                functions.push(info);
            }
        }

        // Apply filters
        if let Some(filter) = filter {
            functions = self.apply_filter(functions, filter);
        }

        functions
    }

    /// Get a snapshot of current runtime state
    pub fn get_snapshot(&self) -> RuntimeSnapshot {
        let mut snapshot = RuntimeSnapshot {
            timestamp_ms: current_time_ms(),
            total_functions: 0,
            functions_per_tier: HashMap::new(),
            total_memory_bytes: 0,
            total_compilation_time_us: 0,
            total_execution_time_ns: 0,
            gc_stats: None,
            active_tasks: 0,
            cache_hit_rate: 0.0,
        };

        // Get tiered compilation stats
        if let Some(ref compiler) = self.tiered_compiler {
            let stats = compiler.read().get_stats();
            snapshot.total_functions = stats.total_functions();
            snapshot.functions_per_tier = stats.functions_per_tier.clone();
        }

        // Get compilation time
        if let Some(ref profiler) = self.compilation_profiler {
            snapshot.total_compilation_time_us =
                profiler.read().total_compilation_time().as_micros() as u64;
        }

        // Get memory usage
        if let Some(ref profiler) = self.memory_profiler {
            snapshot.total_memory_bytes = profiler.read().total_memory_usage();
        }

        // Get GC stats
        snapshot.gc_stats = Some(crate::runtime::memory::gc::get_gc().collect());

        snapshot
    }

    /// Get top N functions by call count
    pub fn get_hot_functions(&self, limit: usize) -> Vec<CompiledFunctionInfo> {
        let filter = QueryFilter {
            sort_by: Some(SortField::CallCount),
            limit: Some(limit),
            ..Default::default()
        };
        self.list_functions(Some(filter))
    }

    /// Get functions with potential memory leaks
    pub fn get_leaky_functions(&self) -> Vec<String> {
        if let Some(ref profiler) = self.memory_profiler {
            profiler
                .read()
                .get_potential_leaks()
                .into_iter()
                .map(|s| s.function_name)
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get tiered compilation statistics
    pub fn get_tiered_stats(&self) -> Option<TieredStats> {
        self.tiered_compiler.as_ref().map(|c| c.read().get_stats())
    }

    /// Get total memory usage
    pub fn get_total_memory_usage(&self) -> usize {
        self.memory_profiler
            .as_ref()
            .map(|p| p.read().total_memory_usage())
            .unwrap_or(0)
    }

    /// Apply query filter to function list
    fn apply_filter(
        &self,
        mut functions: Vec<CompiledFunctionInfo>,
        filter: QueryFilter,
    ) -> Vec<CompiledFunctionInfo> {
        // Filter by call count
        if let Some(min_calls) = filter.min_call_count {
            functions.retain(|f| f.call_count >= min_calls);
        }

        // Filter by tier
        if let Some(tier) = filter.tier {
            functions.retain(|f| f.tier == tier);
        }

        // Filter by name pattern
        if let Some(ref pattern) = filter.name_pattern {
            functions.retain(|f| f.name.contains(pattern));
        }

        // Sort
        if let Some(sort_by) = filter.sort_by {
            match sort_by {
                SortField::Name => functions.sort_by(|a, b| a.name.cmp(&b.name)),
                SortField::CallCount => functions.sort_by(|a, b| b.call_count.cmp(&a.call_count)),
                SortField::Tier => functions.sort_by(|a, b| b.tier.cmp(&a.tier)),
                SortField::CodeSize => functions.sort_by(|a, b| b.code_size.cmp(&a.code_size)),
                SortField::ExecutionTime => {
                    // Would need execution time in metrics
                    functions.sort_by(|a, b| b.call_count.cmp(&a.call_count));
                }
                SortField::MemoryUsage => {
                    // Would need memory usage per function
                    functions.sort_by(|a, b| a.name.cmp(&b.name));
                }
            }
        }

        // Limit
        if let Some(limit) = filter.limit {
            functions.truncate(limit);
        }

        functions
    }

    /// Export runtime state to JSON-compatible structure
    pub fn export_state(&self) -> serde_json::Value {
        let snapshot = self.get_snapshot();
        let functions = self.list_functions(None);

        serde_json::json!({
            "timestamp_ms": snapshot.timestamp_ms,
            "total_functions": snapshot.total_functions,
            "functions_per_tier": snapshot.functions_per_tier,
            "total_memory_bytes": snapshot.total_memory_bytes,
            "total_compilation_time_us": snapshot.total_compilation_time_us,
            "functions": functions.iter().map(|f| {
                serde_json::json!({
                    "name": f.name,
                    "tier": format!("{:?}", f.tier),
                    "call_count": f.call_count,
                    "recompilation_count": f.recompilation_count,
                    "code_size": f.code_size,
                })
            }).collect::<Vec<_>>(),
        })
    }
}

impl Default for RuntimeIntrospector {
    fn default() -> Self {
        Self::new()
    }
}

/// Get current time in milliseconds
fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Global introspector instance
static GLOBAL_INTROSPECTOR: once_cell::sync::Lazy<Arc<RwLock<RuntimeIntrospector>>> =
    once_cell::sync::Lazy::new(|| Arc::new(RwLock::new(RuntimeIntrospector::new())));

/// Get the global runtime introspector
pub fn get_introspector() -> Arc<RwLock<RuntimeIntrospector>> {
    GLOBAL_INTROSPECTOR.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_filter() {
        let introspector = RuntimeIntrospector::new();

        let filter = QueryFilter {
            min_call_count: Some(100),
            tier: Some(CompilationTier::Aggressive),
            ..Default::default()
        };

        // Would need to set up test data
        let _results = introspector.list_functions(Some(filter));
    }

    #[test]
    fn test_snapshot() {
        let introspector = RuntimeIntrospector::new();
        let snapshot = introspector.get_snapshot();

        assert!(snapshot.timestamp_ms > 0);
    }
}
