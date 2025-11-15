use super::{CallSiteContext, SpecializationKey};
use std::collections::HashMap;

/// JIT Function Specializer
pub struct Specializer {
    cache: HashMap<SpecializationKey, usize>,
}

impl Default for Specializer {
    fn default() -> Self {
        Self::new()
    }
}

impl Specializer {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Check if a call site can be specialized
    pub fn can_specialize(&self, context: &CallSiteContext) -> bool {
        // Check if we have constants to specialize on
        context.arg_constants.iter().any(|c| c.is_some())
    }

    /// Get or create specialization key
    pub fn get_specialization_key(
        &mut self,
        context: &CallSiteContext,
    ) -> Option<SpecializationKey> {
        if !self.can_specialize(context) {
            return None;
        }

        let key = context.specialization_key();
        let entry = self.cache.entry(key.clone()).or_insert(0);
        *entry += 1;
        Some(key)
    }

    /// Get specialization statistics
    pub fn stats(&self) -> HashMap<String, usize> {
        let mut stats = HashMap::new();
        for (key, count) in &self.cache {
            stats.insert(key.function_name.clone(), *count);
        }
        stats
    }
}
