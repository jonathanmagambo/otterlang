use std::collections::HashMap;
use std::time::Duration;

/// Profiles individual function calls
pub struct CallProfiler {
    start_times: HashMap<String, std::time::Instant>,
}

impl CallProfiler {
    pub fn new() -> Self {
        Self {
            start_times: HashMap::new(),
        }
    }

    pub fn start_call(&mut self, function_name: &str) {
        self.start_times
            .insert(function_name.to_string(), std::time::Instant::now());
    }

    pub fn end_call(&mut self, function_name: &str) -> Option<Duration> {
        self.start_times
            .remove(function_name)
            .map(|start| start.elapsed())
    }

    pub fn is_active(&self, function_name: &str) -> bool {
        self.start_times.contains_key(function_name)
    }
}

impl Default for CallProfiler {
    fn default() -> Self {
        Self::new()
    }
}
