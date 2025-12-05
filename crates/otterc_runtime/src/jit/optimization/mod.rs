// In-memory Optimization System
pub mod call_graph;
pub mod inliner;
pub mod reoptimizer;

pub use call_graph::CallGraph;
pub use inliner::Inliner;
pub use reoptimizer::Reoptimizer;

/// Optimization context for hot functions
pub struct OptimizationContext {
    pub hot_functions: Vec<String>,
    pub call_graph: CallGraph,
}

impl OptimizationContext {
    pub fn new(hot_functions: Vec<String>) -> Self {
        Self {
            hot_functions,
            call_graph: CallGraph::new(),
        }
    }
}
