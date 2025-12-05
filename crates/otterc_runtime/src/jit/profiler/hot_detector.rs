use std::collections::HashMap;
use std::time::Duration;

use super::FunctionMetrics;

/// Configuration for hot function detection
#[derive(Debug, Clone)]
pub struct HotDetectorConfig {
    /// Minimum number of calls before considering a function hot
    pub call_threshold: u64,
    /// Minimum percentage of total execution time before considering hot
    pub time_threshold_percent: f64,
}

impl Default for HotDetectorConfig {
    fn default() -> Self {
        Self {
            call_threshold: 1000,
            time_threshold_percent: 5.0,
        }
    }
}

/// Represents a function that has been detected as "hot"
#[derive(Debug, Clone)]
pub struct HotFunction {
    pub name: String,
    pub metrics: FunctionMetrics,
    pub reason: HotReason,
}

#[derive(Debug, Clone)]
pub enum HotReason {
    HighCallCount,
    HighTimePercentage,
    Both,
}

/// Detects hot functions based on profiling metrics
pub struct HotDetector {
    config: HotDetectorConfig,
}

impl HotDetector {
    pub fn new(config: HotDetectorConfig) -> Self {
        Self { config }
    }

    pub fn detect_hot_functions(
        &mut self,
        metrics: &HashMap<String, FunctionMetrics>,
        total_time: Duration,
    ) -> Vec<HotFunction> {
        let mut hot_functions = Vec::new();

        for (name, func_metrics) in metrics {
            let is_high_call_count = func_metrics.call_count >= self.config.call_threshold;
            let time_percent = func_metrics.time_percentage(total_time);
            let is_high_time = time_percent >= self.config.time_threshold_percent;

            if is_high_call_count || is_high_time {
                let reason = match (is_high_call_count, is_high_time) {
                    (true, true) => HotReason::Both,
                    (true, false) => HotReason::HighCallCount,
                    (false, true) => HotReason::HighTimePercentage,
                    (false, false) => unreachable!(),
                };

                hot_functions.push(HotFunction {
                    name: name.clone(),
                    metrics: func_metrics.clone(),
                    reason,
                });
            }
        }

        hot_functions.sort_by(|a, b| {
            b.metrics
                .call_count
                .cmp(&a.metrics.call_count)
                .then_with(|| b.metrics.total_time.cmp(&a.metrics.total_time))
        });

        hot_functions
    }
}

impl Default for HotDetector {
    fn default() -> Self {
        Self::new(HotDetectorConfig::default())
    }
}
