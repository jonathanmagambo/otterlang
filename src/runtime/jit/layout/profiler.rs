use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use super::optimizer::{AccessType, FieldId, StructId};
/// Memory access pattern for profiling
#[derive(Debug, Clone)]
pub struct AccessPattern {
    pub address: usize,
    pub size: usize,
    pub access_type: AccessType,
    pub timestamp: u64,
    pub struct_id: Option<StructId>,
    pub field_id: Option<FieldId>,
}

/// Profiles memory access patterns to identify optimization opportunities
pub struct MemoryProfiler {
    accesses: RwLock<Vec<AccessPattern>>,
    struct_accesses: RwLock<HashMap<StructId, Vec<AccessPattern>>>,
    field_accesses: RwLock<HashMap<FieldId, Vec<AccessPattern>>>,
    total_access_count: AtomicU64,
    max_samples: usize,
}

impl MemoryProfiler {
    pub fn new() -> Self {
        Self {
            accesses: RwLock::new(Vec::new()),
            struct_accesses: RwLock::new(HashMap::new()),
            field_accesses: RwLock::new(HashMap::new()),
            total_access_count: AtomicU64::new(0),
            max_samples: 100_000, // Limit sample size
        }
    }

    pub fn with_max_samples(max_samples: usize) -> Self {
        Self {
            accesses: RwLock::new(Vec::new()),
            struct_accesses: RwLock::new(HashMap::new()),
            field_accesses: RwLock::new(HashMap::new()),
            total_access_count: AtomicU64::new(0),
            max_samples,
        }
    }

    pub fn record_access(&self, address: usize, size: usize, access_type: AccessType) {
        self.record_access_with_context(address, size, access_type, None, None);
    }

    pub fn record_access_with_context(
        &self,
        address: usize,
        size: usize,
        access_type: AccessType,
        struct_id: Option<StructId>,
        field_id: Option<FieldId>,
    ) {
        let count = self.total_access_count.fetch_add(1, Ordering::SeqCst);

        // Sample periodically to avoid excessive memory usage
        if count.is_multiple_of(10) {
            let mut accesses = self.accesses.write();
            if accesses.len() >= self.max_samples {
                // Keep only recent samples (simple FIFO)
                accesses.drain(0..self.max_samples / 10);
            }

            let pattern = AccessPattern {
                address,
                size,
                access_type,
                timestamp: count,
                struct_id,
                field_id,
            };

            accesses.push(pattern.clone());

            // Track by struct
            if let Some(sid) = struct_id {
                self.struct_accesses
                    .write()
                    .entry(sid)
                    .or_default()
                    .push(pattern.clone());
            }

            // Track by field
            if let Some(fid) = field_id {
                self.field_accesses
                    .write()
                    .entry(fid)
                    .or_default()
                    .push(pattern);
            }
        }
    }

    pub fn get_access_patterns(&self) -> Vec<AccessPattern> {
        self.accesses.read().clone()
    }

    pub fn get_struct_patterns(&self, struct_id: StructId) -> Vec<AccessPattern> {
        self.struct_accesses
            .read()
            .get(&struct_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn get_field_patterns(&self, field_id: FieldId) -> Vec<AccessPattern> {
        self.field_accesses
            .read()
            .get(&field_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn analyze_access_frequency(&self) -> HashMap<StructId, u64> {
        let mut frequencies = HashMap::new();
        for (struct_id, patterns) in self.struct_accesses.read().iter() {
            frequencies.insert(*struct_id, patterns.len() as u64);
        }
        frequencies
    }

    pub fn analyze_field_frequency(&self) -> HashMap<FieldId, u64> {
        let mut frequencies = HashMap::new();
        for (field_id, patterns) in self.field_accesses.read().iter() {
            frequencies.insert(*field_id, patterns.len() as u64);
        }
        frequencies
    }

    pub fn detect_temporal_locality(&self, struct_id: StructId) -> f64 {
        let patterns = self.get_struct_patterns(struct_id);
        if patterns.len() < 2 {
            return 0.0;
        }

        // Calculate average time between accesses
        let mut time_diffs = Vec::new();
        for i in 1..patterns.len() {
            let diff = patterns[i].timestamp - patterns[i - 1].timestamp;
            time_diffs.push(diff);
        }

        if time_diffs.is_empty() {
            return 0.0;
        }

        let avg_diff = time_diffs.iter().sum::<u64>() as f64 / time_diffs.len() as f64;

        // Lower average difference = better temporal locality
        // Normalize to 0-1 scale (assuming max diff of 1000)
        (1.0 - (avg_diff.min(1000.0) / 1000.0)).max(0.0)
    }

    pub fn detect_spatial_locality(&self, struct_id: StructId) -> f64 {
        let patterns = self.get_struct_patterns(struct_id);
        if patterns.is_empty() {
            return 0.0;
        }

        // Calculate average distance between consecutive accesses
        let mut distances = Vec::new();
        for i in 1..patterns.len() {
            let addr1 = patterns[i - 1].address;
            let addr2 = patterns[i].address;
            let distance = addr2.abs_diff(addr1);
            distances.push(distance);
        }

        if distances.is_empty() {
            return 0.0;
        }

        let avg_distance = distances.iter().sum::<usize>() as f64 / distances.len() as f64;

        // Lower average distance = better spatial locality
        // Normalize to 0-1 scale (assuming cache line size of 64 bytes)
        let cache_line_size = 64.0;
        (1.0 - (avg_distance.min(cache_line_size * 4.0) / (cache_line_size * 4.0))).max(0.0)
    }

    pub fn total_accesses(&self) -> u64 {
        self.total_access_count.load(Ordering::SeqCst)
    }

    pub fn structures_tracked(&self) -> usize {
        self.struct_accesses.read().len()
    }
}

impl Default for MemoryProfiler {
    fn default() -> Self {
        Self::new()
    }
}
