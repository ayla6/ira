//! Estimates the controller's report rate from physical event inter-arrival
//! gaps so the mapping pipeline (gyro sampling, output ticks) runs at the
//! controller's native cadence — a 1000 Hz pad gets a 1000 Hz pipeline —
//! instead of a fixed 250 Hz that discards three of every four reports.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Candidate intervals matching common controller report rates, from 8000 Hz
/// overclocked pads down to 125 Hz Bluetooth mode.
const COMMON_INTERVALS_MS: [f32; 7] = [0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0];
/// Gaps longer than this mean the controller went quiet (many pads only
/// report changes); they say nothing about the report rate.
const IDLE_GAP: Duration = Duration::from_millis(100);
/// Gaps needed before the first estimate replaces the default.
const MIN_SAMPLES: usize = 16;

#[derive(Debug)]
pub struct ReportRateEstimator {
    gaps: VecDeque<f32>,
    last_event: Option<Instant>,
    interval: Duration,
}

impl Default for ReportRateEstimator {
    fn default() -> Self {
        Self {
            gaps: VecDeque::with_capacity(64),
            last_event: None,
            // Matches the previous fixed sample interval; replaced by the
            // measured rate once enough events have arrived.
            interval: Duration::from_millis(4),
        }
    }
}

impl ReportRateEstimator {
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Record a burst of physical events arriving now.
    pub fn observe(&mut self, now: Instant) {
        if let Some(last) = self.last_event.replace(now) {
            let gap = now.saturating_duration_since(last);
            if gap >= IDLE_GAP {
                // Long silence: restart gap collection, keep the estimate.
                self.gaps.clear();
                return;
            }
            if self.gaps.len() == 64 {
                self.gaps.pop_front();
            }
            self.gaps.push_back(gap.as_secs_f32());
            if self.gaps.len() >= MIN_SAMPLES {
                self.update_interval();
            }
        }
    }

    /// Forget everything learned; the next controller starts fresh.
    pub fn reset(&mut self) {
        self.gaps.clear();
        self.last_event = None;
        self.interval = Duration::from_millis(4);
    }

    fn update_interval(&mut self) {
        let mut sorted = self.gaps.iter().copied().collect::<Vec<_>>();
        sorted.sort_by(f32::total_cmp);
        let median_seconds = sorted[sorted.len() / 2];
        let median_ms = median_seconds * 1000.0;
        // Nearest candidate by ratio so the spacing in the table (powers of
        // two around 1 ms) doesn't bias toward either end.
        let nearest_ms = COMMON_INTERVALS_MS
            .iter()
            .copied()
            .min_by(|left, right| {
                let left_ratio = (median_ms / left).max(left / median_ms);
                let right_ratio = (median_ms / right).max(right / median_ms);
                left_ratio
                    .partial_cmp(&right_ratio)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(4.0);
        let interval = Duration::from_secs_f32(nearest_ms / 1000.0);
        if interval != self.interval {
            self.interval = interval;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn advance(estimator: &mut ReportRateEstimator, gap: Duration) {
        let now = estimator
            .last_event
            .unwrap_or_else(Instant::now)
            .checked_add(gap)
            .unwrap_or_else(Instant::now);
        estimator.observe(now);
    }

    fn settle(estimator: &mut ReportRateEstimator, gap: Duration, count: usize) {
        for _ in 0..count {
            advance(estimator, gap);
        }
    }

    #[test]
    fn test_1000hz_controller_gets_1ms_pipeline() {
        let mut estimator = ReportRateEstimator::default();
        settle(&mut estimator, Duration::from_micros(1000), 32);
        assert_eq!(estimator.interval(), Duration::from_millis(1));
    }

    #[test]
    fn test_125hz_controller_gets_8ms_pipeline() {
        let mut estimator = ReportRateEstimator::default();
        settle(&mut estimator, Duration::from_millis(8), 32);
        assert_eq!(estimator.interval(), Duration::from_millis(8));
    }

    #[test]
    fn test_8000hz_controller_gets_sub_millisecond_pipeline() {
        let mut estimator = ReportRateEstimator::default();
        settle(&mut estimator, Duration::from_micros(125), 32);
        assert_eq!(estimator.interval(), Duration::from_micros(125));
    }

    #[test]
    fn test_jitter_does_not_skew_estimate() {
        // Real loops deliver gaps with scheduling jitter; the median must
        // still land on the true 2 ms period.
        let mut estimator = ReportRateEstimator::default();
        for _ in 0..16 {
            advance(&mut estimator, Duration::from_micros(1400));
            advance(&mut estimator, Duration::from_micros(2600));
        }
        assert_eq!(estimator.interval(), Duration::from_millis(2));
    }

    #[test]
    fn test_idle_gap_keeps_estimate_and_restarts_collection() {
        let mut estimator = ReportRateEstimator::default();
        settle(&mut estimator, Duration::from_millis(1), 32);
        assert_eq!(estimator.interval(), Duration::from_millis(1));
        // Controller goes quiet mid-hold, then resumes at the same rate.
        advance(&mut estimator, Duration::from_millis(500));
        assert_eq!(
            estimator.interval(),
            Duration::from_millis(1),
            "idle must not change the estimate"
        );
        settle(&mut estimator, Duration::from_millis(1), 32);
        assert_eq!(estimator.interval(), Duration::from_millis(1));
    }

    #[test]
    fn test_reset_restores_default_interval() {
        let mut estimator = ReportRateEstimator::default();
        settle(&mut estimator, Duration::from_millis(1), 32);
        estimator.reset();
        assert_eq!(estimator.interval(), Duration::from_millis(4));
    }
}
