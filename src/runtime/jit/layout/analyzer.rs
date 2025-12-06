use std::collections::HashMap;

use super::optimizer::{CacheAnalysis, FieldAccessStats, FieldId, StructId};
use super::profiler::AccessPattern;

/// Analyzes cache locality and access patterns
pub struct CacheLocalityAnalyzer {
    cache_line_size: usize,
    l1_cache_size: usize,
    #[expect(dead_code, reason = "Work in progress")]
    l2_cache_size: usize,
}

impl CacheLocalityAnalyzer {
    pub fn new() -> Self {
        Self {
            cache_line_size: 64,       // Typical cache line size
            l1_cache_size: 32 * 1024,  // 32KB L1 cache
            l2_cache_size: 256 * 1024, // 256KB L2 cache
        }
    }

    pub fn with_cache_info(cache_line_size: usize, l1_size: usize, l2_size: usize) -> Self {
        Self {
            cache_line_size,
            l1_cache_size: l1_size,
            l2_cache_size: l2_size,
        }
    }

    pub fn analyze_patterns(
        &mut self,
        patterns: &[AccessPattern],
    ) -> Result<HashMap<StructId, CacheAnalysis>, String> {
        let mut results = HashMap::new();

        // Group patterns by struct
        let mut struct_patterns: HashMap<StructId, Vec<&AccessPattern>> = HashMap::new();
        for pattern in patterns {
            if let Some(struct_id) = pattern.struct_id {
                struct_patterns.entry(struct_id).or_default().push(pattern);
            }
        }

        // Analyze each struct
        for (struct_id, struct_accesses) in struct_patterns {
            let analysis = self.analyze_struct(struct_id, &struct_accesses)?;
            results.insert(struct_id, analysis);
        }

        Ok(results)
    }

    fn analyze_struct(
        &self,
        struct_id: StructId,
        patterns: &[&AccessPattern],
    ) -> Result<CacheAnalysis, String> {
        if patterns.is_empty() {
            return Ok(CacheAnalysis {
                struct_id,
                cache_locality_score: 0.0,
                cache_miss_rate: 1.0,
                field_accesses: HashMap::new(),
            });
        }

        // Analyze cache locality
        let cache_locality_score = self.calculate_cache_locality(patterns);
        let cache_miss_rate = self.estimate_cache_miss_rate(patterns);

        // Analyze field access patterns
        let mut field_accesses = HashMap::new();
        for pattern in patterns {
            if let Some(field_id) = pattern.field_id {
                let stats = field_accesses
                    .entry(field_id)
                    .or_insert_with(|| FieldAccessStats {
                        access_count: 0,
                        size: pattern.size,
                        cache_hits: 0,
                        cache_misses: 0,
                    });

                stats.access_count += 1;

                // Estimate cache hit/miss based on address locality
                if self.is_likely_cache_hit(pattern.address, patterns) {
                    stats.cache_hits += 1;
                } else {
                    stats.cache_misses += 1;
                }
            }
        }

        Ok(CacheAnalysis {
            struct_id,
            cache_locality_score,
            cache_miss_rate,
            field_accesses,
        })
    }

    fn calculate_cache_locality(&self, patterns: &[&AccessPattern]) -> f64 {
        if patterns.len() < 2 {
            return 0.0;
        }

        // Calculate spatial locality (how close addresses are)
        let mut spatial_scores = Vec::new();
        for i in 1..patterns.len() {
            let addr1 = patterns[i - 1].address;
            let addr2 = patterns[i].address;
            let distance = addr2.abs_diff(addr1);

            // Score based on cache line alignment
            if distance < self.cache_line_size {
                spatial_scores.push(1.0);
            } else if distance < self.cache_line_size * 4 {
                spatial_scores.push(0.5);
            } else {
                spatial_scores.push(0.0);
            }
        }

        // Calculate temporal locality (how frequently accessed)
        let time_span = if patterns.len() > 1 {
            patterns[patterns.len() - 1].timestamp - patterns[0].timestamp
        } else {
            1
        };
        let access_density = patterns.len() as f64 / time_span.max(1) as f64;

        // Combine scores
        let spatial_score = spatial_scores.iter().sum::<f64>() / spatial_scores.len() as f64;
        let temporal_score = (access_density.min(1.0) * 0.5).min(0.5);

        (spatial_score * 0.7 + temporal_score * 0.3).min(1.0)
    }

    fn estimate_cache_miss_rate(&self, patterns: &[&AccessPattern]) -> f64 {
        if patterns.is_empty() {
            return 1.0;
        }

        // Estimate based on address distribution
        let mut cache_lines_accessed = std::collections::HashSet::new();
        for pattern in patterns {
            let cache_line = pattern.address / self.cache_line_size;
            cache_lines_accessed.insert(cache_line);
        }

        // Estimate miss rate based on cache capacity
        let unique_cache_lines = cache_lines_accessed.len();
        let cache_lines_in_l1 = self.l1_cache_size / self.cache_line_size;

        if unique_cache_lines <= cache_lines_in_l1 {
            0.1 // Low miss rate
        } else if unique_cache_lines <= cache_lines_in_l1 * 2 {
            0.3 // Moderate miss rate
        } else {
            0.7 // High miss rate
        }
    }

    fn is_likely_cache_hit(&self, address: usize, patterns: &[&AccessPattern]) -> bool {
        // Check if this address was recently accessed
        let cache_line = address / self.cache_line_size;
        let recent_patterns = patterns.iter().rev().take(10);

        for pattern in recent_patterns {
            let pattern_cache_line = pattern.address / self.cache_line_size;
            if pattern_cache_line == cache_line {
                return true;
            }
        }

        false
    }

    pub fn suggest_field_order(&self, analysis: &CacheAnalysis) -> Vec<FieldId> {
        // Order fields by access frequency and size for optimal cache alignment
        let mut fields: Vec<_> = analysis.field_accesses.iter().collect();

        fields.sort_by(|(_, a), (_, b)| {
            // Prioritize frequently accessed fields
            b.access_count.cmp(&a.access_count).then_with(|| {
                // Then by size (larger fields first for alignment)
                b.size.cmp(&a.size)
            })
        });

        fields.into_iter().map(|(id, _)| *id).collect()
    }
}

impl Default for CacheLocalityAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}
