use crate::runtime::jit::engine::JitEngine;
use crate::runtime::symbol_registry::SymbolRegistry;
use anyhow::Result;
use otterc_ast::nodes::Program;
use std::collections::HashMap;

/// Simplified JIT executor for running programs
pub struct JitExecutor {
    engine: JitEngine,
    hotness_counters: HashMap<String, usize>,
    optimization_threshold: usize,
}

impl JitExecutor {
    /// Create a new JIT executor with default LLVM backend
    pub fn new(
        program: &Program,
        symbol_registry: &'static SymbolRegistry,
    ) -> anyhow::Result<Self> {
        Self::new_with_backend(program, symbol_registry)
    }

    /// Create a new JIT executor with specified backend
    pub fn new_with_backend(
        program: &Program,
        symbol_registry: &'static SymbolRegistry,
    ) -> anyhow::Result<Self> {
        let mut engine = JitEngine::new_with_backend(symbol_registry)?;
        engine.compile_program(program)?;

        Ok(Self {
            engine,
            hotness_counters: HashMap::new(),
            optimization_threshold: 100, // Default threshold
        })
    }

    /// Recompile the current program without rebuilding the entire engine
    pub fn recompile(&mut self, program: &Program) -> Result<()> {
        self.engine.compile_program(program)
    }

    /// Execute the main function
    pub fn execute_main(&mut self) -> Result<()> {
        self.execute_with_profiling("main", &[])
    }

    /// Execute a function with profiling and hotness tracking
    pub fn execute_with_profiling(&mut self, name: &str, args: &[u64]) -> Result<()> {
        // Update hotness counter
        let count = {
            let counter = self.hotness_counters.entry(name.to_string()).or_insert(0);
            *counter += 1;
            *counter
        };

        // Check for optimization
        if count >= self.optimization_threshold {
            self.optimize_function(name)?;
            // Reset counter after optimization to avoid repeated optimization
            self.hotness_counters.insert(name.to_string(), 0);
        }

        // Execute
        self.engine.execute_function(name, args)?;
        Ok(())
    }

    /// Trigger optimization for a hot function
    fn optimize_function(&mut self, _name: &str) -> Result<()> {
        // In a real JIT, this would trigger re-compilation with higher optimization levels (O3)
        // For now, we just log it or simulate it
        // println!("Optimizing hot function: {}", name);

        // Example: self.engine.recompile(name, OptLevel::Aggressive)?;
        Ok(())
    }

    /// Get performance statistics
    pub fn get_stats(&self) -> ExecutorStats {
        ExecutorStats {
            profiler_metrics: self.engine.get_profiler_stats(),
            cache_stats: self.engine.get_cache_stats(),
        }
    }
}

#[derive(Debug)]
pub struct ExecutorStats {
    pub profiler_metrics: Vec<super::profiler::FunctionMetrics>,
    pub cache_stats: super::cache::function_cache::CacheStats,
}
