//! Separate deadline clocks for Recon turns (spec §7).
//!
//! Queue/backoff time counts toward wall lifetime but not active provider
//! execution allowance. Foreground expiry can continue work in the background
//! under the same job identity.

use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct ClockSet {
    pub started: Instant,
    pub job_lifetime: Duration,
    pub foreground: Duration,
    pub attempt_execution: Duration,
    pub stream_inactivity: Duration,
    /// Accumulated active provider execution time.
    pub active_spent: Duration,
    /// Accumulated queue/backoff waiting time.
    pub queue_spent: Duration,
}

impl ClockSet {
    pub fn new(job_lifetime: Duration, foreground: Duration) -> Self {
        Self {
            started: Instant::now(),
            job_lifetime,
            foreground,
            attempt_execution: Duration::from_secs(120),
            stream_inactivity: Duration::from_secs(60),
            active_spent: Duration::ZERO,
            queue_spent: Duration::ZERO,
        }
    }

    pub fn wall_elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn remaining_lifetime(&self) -> Duration {
        self.job_lifetime.saturating_sub(self.wall_elapsed())
    }

    pub fn remaining_foreground(&self) -> Duration {
        self.foreground.saturating_sub(self.wall_elapsed())
    }

    pub fn foreground_expired(&self) -> bool {
        self.remaining_foreground().is_zero()
    }

    pub fn job_expired(&self) -> bool {
        self.remaining_lifetime().is_zero()
    }

    /// Active attempt budget cannot exceed remaining hard lifetime.
    pub fn attempt_budget(&self) -> Duration {
        self.attempt_execution.min(self.remaining_lifetime())
    }

    pub fn record_active(&mut self, spent: Duration) {
        self.active_spent = self.active_spent.saturating_add(spent);
    }

    pub fn record_queue(&mut self, spent: Duration) {
        self.queue_spent = self.queue_spent.saturating_add(spent);
    }

    /// Sequential tool steps accumulate; independent steps may overlap up to capacity.
    pub fn sequential_tool_budget(timeouts: &[Duration]) -> Duration {
        timeouts
            .iter()
            .fold(Duration::ZERO, |acc, d| acc.saturating_add(*d))
    }

    pub fn overlapping_tool_budget(timeouts: &[Duration], capacity: usize) -> Duration {
        if timeouts.is_empty() {
            return Duration::ZERO;
        }
        let capacity = capacity.max(1) as u64;
        let mut sorted = timeouts.to_vec();
        sorted.sort();
        let longest = *sorted.last().unwrap_or(&Duration::ZERO);
        let sum: Duration = sorted.iter().copied().sum();
        Duration::from_secs(sum.as_secs() / capacity).max(longest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_time_does_not_spend_active_but_consumes_wall() {
        let mut clocks = ClockSet::new(Duration::from_secs(100), Duration::from_secs(30));
        clocks.record_queue(Duration::from_secs(5));
        clocks.record_active(Duration::from_secs(2));
        assert_eq!(clocks.active_spent, Duration::from_secs(2));
        assert_eq!(clocks.queue_spent, Duration::from_secs(5));
        let remaining = clocks.remaining_lifetime();
        assert!(clocks.attempt_execution.min(remaining) <= remaining);
        assert!(clocks.attempt_budget() <= clocks.job_lifetime);
    }

    #[test]
    fn sequential_vs_overlapping_tool_budgets() {
        let t = [
            Duration::from_secs(40),
            Duration::from_secs(40),
            Duration::from_secs(40),
            Duration::from_secs(40),
        ];
        assert_eq!(
            ClockSet::sequential_tool_budget(&t),
            Duration::from_secs(160)
        );
        let overlap = ClockSet::overlapping_tool_budget(&t, 4);
        assert!(overlap <= Duration::from_secs(160));
        assert!(overlap >= Duration::from_secs(40));
    }
}
