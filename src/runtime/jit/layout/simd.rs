use super::optimizer::{SimdOpportunity, StructId};
use super::profiler::AccessPattern;

/// Detects SIMD vectorization opportunities
pub struct SimdOpportunityDetector {
    simd_width: usize, // Typically 128, 256, or 512 bits
}

impl SimdOpportunityDetector {
    pub fn new() -> Self {
        Self {
            simd_width: 256, // Assume AVX2 (256-bit)
        }
    }

    pub fn with_simd_width(width: usize) -> Self {
        Self { simd_width: width }
    }

    pub fn detect_opportunities(
        &mut self,
        patterns: &[AccessPattern],
    ) -> Result<Vec<SimdOpportunity>, String> {
        // Group patterns by struct
        let mut struct_patterns: std::collections::HashMap<StructId, Vec<&AccessPattern>> =
            std::collections::HashMap::new();

        for pattern in patterns {
            if let Some(struct_id) = pattern.struct_id {
                struct_patterns
                    .entry(struct_id)
                    .or_default()
                    .push(pattern);
            }
        }

        let mut opportunities = Vec::new();

        for (struct_id, struct_accesses) in struct_patterns {
            if let Some(opportunity) = self.analyze_struct(struct_id, &struct_accesses)? {
                opportunities.push(opportunity);
            }
        }

        Ok(opportunities)
    }

    fn analyze_struct(
        &self,
        struct_id: StructId,
        patterns: &[&AccessPattern],
    ) -> Result<Option<SimdOpportunity>, String> {
        if patterns.is_empty() {
            return Ok(None);
        }

        // Look for sequential access patterns that could benefit from SIMD
        let vectorizable_fields = self.identify_vectorizable_fields(patterns);
        let simd_score = self.calculate_simd_score(patterns, &vectorizable_fields);

        if simd_score > 0.5 {
            Ok(Some(SimdOpportunity {
                struct_id,
                simd_utilization_score: simd_score,
                vectorizable_fields,
            }))
        } else {
            Ok(None)
        }
    }

    fn identify_vectorizable_fields(&self, patterns: &[&AccessPattern]) -> Vec<super::FieldId> {
        // Identify fields that are accessed sequentially and have compatible sizes
        let mut field_access_counts: std::collections::HashMap<super::FieldId, usize> =
            std::collections::HashMap::new();

        for pattern in patterns {
            if let Some(field_id) = pattern.field_id {
                *field_access_counts.entry(field_id).or_insert(0) += 1;
            }
        }

        // Filter fields that are accessed frequently and have size compatible with SIMD
        field_access_counts
            .iter()
            .filter(|(_, count)| **count > 10) // Minimum access threshold
            .filter_map(|(field_id, _)| {
                // Check if field size is compatible with SIMD
                if let Some(pattern) = patterns.iter().find(|p| p.field_id == Some(*field_id)) {
                    if pattern.size >= 4 && pattern.size <= 16 {
                        // Typical SIMD-compatible sizes (4, 8, 16 bytes)
                        Some(*field_id)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    fn calculate_simd_score(
        &self,
        patterns: &[&AccessPattern],
        vectorizable_fields: &[super::FieldId],
    ) -> f64 {
        if vectorizable_fields.is_empty() {
            return 0.0;
        }

        // Calculate sequential access ratio
        let sequential_accesses = self.count_sequential_accesses(patterns);
        let total_accesses = patterns.len();

        if total_accesses == 0 {
            return 0.0;
        }

        let sequential_ratio = sequential_accesses as f64 / total_accesses as f64;

        // Calculate field alignment score
        let alignment_score = self.calculate_alignment_score(patterns, vectorizable_fields);

        // Combine scores
        (sequential_ratio * 0.6 + alignment_score * 0.4).min(1.0)
    }

    fn count_sequential_accesses(&self, patterns: &[&AccessPattern]) -> usize {
        if patterns.len() < 2 {
            return 0;
        }

        let mut count = 0;
        for i in 1..patterns.len() {
            let addr1 = patterns[i - 1].address;
            let addr2 = patterns[i].address;
            let distance = addr2.abs_diff(addr1);

            // Sequential if addresses are close and increasing
            if distance <= self.simd_width / 8 && addr2 > addr1 {
                count += 1;
            }
        }

        count
    }

    fn calculate_alignment_score(
        &self,
        patterns: &[&AccessPattern],
        vectorizable_fields: &[super::FieldId],
    ) -> f64 {
        if vectorizable_fields.is_empty() {
            return 0.0;
        }

        let mut aligned_accesses = 0;
        let mut total_field_accesses = 0;

        for pattern in patterns {
            if let Some(field_id) = pattern.field_id
                && vectorizable_fields.contains(&field_id) {
                    total_field_accesses += 1;
                    // Check if address is aligned to SIMD width
                    if pattern.address % (self.simd_width / 8) == 0 {
                        aligned_accesses += 1;
                    }
                }
        }

        if total_field_accesses == 0 {
            return 0.0;
        }

        aligned_accesses as f64 / total_field_accesses as f64
    }
}

impl Default for SimdOpportunityDetector {
    fn default() -> Self {
        Self::new()
    }
}
