use parking_lot::RwLock;
use std::sync::Arc;

use super::analyzer::CacheLocalityAnalyzer;
use super::profiler::MemoryProfiler;
use super::simd::SimdOpportunityDetector;
use super::transformer::LayoutTransformer;
use super::validator::LayoutValidator;

/// Type definitions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessType {
    Read,
    Write,
    ReadWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StructId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FieldId(pub u64);

#[derive(Debug, Clone)]
pub enum LayoutOptimization {
    StructReordering {
        struct_id: StructId,
        new_field_order: Vec<FieldId>,
    },
    ArrayTransposition {
        array_id: u64,
        new_dimensions: Vec<usize>,
    },
    MemoryBlockReorganization {
        block_id: u64,
        new_layout: MemoryLayout,
    },
}

#[derive(Debug, Clone)]
pub struct MemoryLayout {
    pub alignment: usize,
    pub field_order: Vec<FieldId>,
    pub padding: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct CacheAnalysis {
    pub struct_id: StructId,
    pub cache_locality_score: f64,
    pub cache_miss_rate: f64,
    pub field_accesses: std::collections::HashMap<FieldId, FieldAccessStats>,
}

#[derive(Debug, Clone)]
pub struct FieldAccessStats {
    pub access_count: u64,
    pub size: usize,
    pub cache_hits: u64,
    pub cache_misses: u64,
}

#[derive(Debug, Clone)]
pub struct SimdOpportunity {
    pub struct_id: StructId,
    pub simd_utilization_score: f64,
    pub vectorizable_fields: Vec<FieldId>,
}

#[derive(Debug, Clone)]
pub struct OptimizerStats {
    pub total_accesses: u64,
    pub structures_tracked: usize,
    pub optimizations_applied: usize,
}

/// Main data layout optimizer that coordinates all subsystems
pub struct DataLayoutOptimizer {
    profiler: Arc<RwLock<MemoryProfiler>>,
    analyzer: Arc<RwLock<CacheLocalityAnalyzer>>,
    simd_detector: Arc<RwLock<SimdOpportunityDetector>>,
    transformer: Arc<RwLock<LayoutTransformer>>,
    validator: Arc<RwLock<LayoutValidator>>,
    enabled: bool,
}

impl DataLayoutOptimizer {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            profiler: Arc::new(RwLock::new(MemoryProfiler::new())),
            analyzer: Arc::new(RwLock::new(CacheLocalityAnalyzer::new())),
            simd_detector: Arc::new(RwLock::new(SimdOpportunityDetector::new())),
            transformer: Arc::new(RwLock::new(LayoutTransformer::new())),
            validator: Arc::new(RwLock::new(LayoutValidator::new())),
            enabled: true,
        })
    }

    pub fn record_access(&self, address: usize, size: usize, access_type: AccessType) {
        if !self.enabled {
            return;
        }
        self.profiler
            .write()
            .record_access(address, size, access_type);
    }

    pub fn analyze_and_optimize(&self) -> Result<Vec<LayoutOptimization>, String> {
        if !self.enabled {
            return Ok(Vec::new());
        }

        // Analyze access patterns
        let patterns = self.profiler.read().get_access_patterns();
        let cache_analysis = self.analyzer.write().analyze_patterns(&patterns)?;
        let simd_opportunities = self.simd_detector.write().detect_opportunities(&patterns)?;

        // Generate optimization suggestions
        let mut optimizations = Vec::new();

        for (struct_id, analysis) in cache_analysis {
            if let Some(optimization) =
                self.generate_optimization(struct_id, &analysis, &simd_opportunities)?
            {
                optimizations.push(optimization);
            }
        }

        Ok(optimizations)
    }

    pub fn apply_optimization(&self, optimization: &LayoutOptimization) -> Result<(), String> {
        // Validate that optimization is safe
        if !self.validator.read().is_safe(optimization)? {
            return Err("Optimization failed safety check".to_string());
        }

        // Apply transformation
        self.transformer.write().apply(optimization)?;

        Ok(())
    }

    fn generate_optimization(
        &self,
        struct_id: StructId,
        analysis: &CacheAnalysis,
        simd_opportunities: &[SimdOpportunity],
    ) -> Result<Option<LayoutOptimization>, String> {
        // Determine if reorganization would be beneficial
        let cache_score = analysis.cache_locality_score;
        let simd_score = simd_opportunities
            .iter()
            .find(|o| o.struct_id == struct_id)
            .map(|o| o.simd_utilization_score)
            .unwrap_or(0.0);

        if cache_score < 0.5 || simd_score > 0.7 {
            // Suggest field reordering
            let reordered_fields = self.suggest_field_order(struct_id, analysis)?;
            return Ok(Some(LayoutOptimization::StructReordering {
                struct_id,
                new_field_order: reordered_fields,
            }));
        }

        Ok(None)
    }

    fn suggest_field_order(
        &self,
        _struct_id: StructId,
        analysis: &CacheAnalysis,
    ) -> Result<Vec<FieldId>, String> {
        // Order fields by access frequency and size for better cache alignment
        let mut fields: Vec<_> = analysis.field_accesses.iter().collect();
        fields.sort_by(|a, b| {
            // Prioritize frequently accessed fields
            b.1.access_count.cmp(&a.1.access_count).then_with(|| {
                // Then by size (larger fields first for alignment)
                b.1.size.cmp(&a.1.size)
            })
        });

        Ok(fields.into_iter().map(|(id, _)| *id).collect())
    }

    pub fn get_stats(&self) -> OptimizerStats {
        OptimizerStats {
            total_accesses: self.profiler.read().total_accesses(),
            structures_tracked: self.profiler.read().structures_tracked(),
            optimizations_applied: self.transformer.read().optimizations_applied(),
        }
    }
}
