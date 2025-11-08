use crate::runtime::symbol_registry::SymbolRegistry;
use anyhow::Result;
use ast::nodes::Program;

use super::engine::JitEngine;

/// JIT executor that coordinates program execution
pub struct JitExecutor {
    engine: JitEngine,
    #[allow(dead_code)]
    program: Program,
}

impl JitExecutor {
    pub fn new(program: Program, symbol_registry: &'static SymbolRegistry) -> Result<Self> {
        let mut engine = JitEngine::new(symbol_registry)?;
        engine.compile_program(&program)?;

        Ok(Self { engine, program })
    }

    /// Execute the main function
    pub fn execute_main(&mut self) -> Result<()> {
        // Execute main function
        self.engine.execute_function("main", &[])?;
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
