use std::collections::HashMap;
use std::time::Duration;

/// Sampling profiler for lightweight profiling
pub struct Sampler {
    sample_interval: u64,
    call_counter: u64,
    samples: HashMap<String, u64>,
}

impl Sampler {
    pub fn new() -> Self {
        Self {
            sample_interval: 100, // Sample every 100 calls
            call_counter: 0,
            samples: HashMap::new(),
        }
    }

    pub fn with_interval(interval: u64) -> Self {
        Self {
            sample_interval: interval,
            call_counter: 0,
            samples: HashMap::new(),
        }
    }

    pub fn record_call(&mut self, function_name: &str, _duration: Duration) {
        self.call_counter += 1;

        if self.call_counter.is_multiple_of(self.sample_interval) {
            *self.samples.entry(function_name.to_string()).or_insert(0) += 1;
        }
    }

    pub fn get_samples(&self) -> &HashMap<String, u64> {
        &self.samples
    }

    pub fn reset(&mut self) {
        self.call_counter = 0;
        self.samples.clear();
    }
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}
