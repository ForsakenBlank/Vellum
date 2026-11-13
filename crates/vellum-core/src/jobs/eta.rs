use std::collections::VecDeque;
use std::time::Duration;

/// only progress from the last while counts, so the estimate follows a job
/// that speeds up or slows down
const WINDOW: Duration = Duration::from_secs(30);
/// samples closer together than this add noise, not information
const MIN_GAP: Duration = Duration::from_millis(200);
/// below this much observed time the rate is a guess
const MIN_SPAN: Duration = Duration::from_secs(1);

/// estimates time remaining from how fast progress has been moving lately.
///
/// times are a job's active running time, so pauses do not count.
#[derive(Debug, Clone, Default)]
pub struct EtaEstimator {
    samples: VecDeque<(Duration, f32)>,
    latest: Option<(Duration, f32)>,
}

impl EtaEstimator {
    pub fn record(&mut self, at: Duration, fraction: f32) {
        if self
            .latest
            .is_some_and(|(last_at, last)| fraction < last || at < last_at)
        {
            // progress went backwards: a new pass or a retry, so start over
            self.reset();
        }
        self.latest = Some((at, fraction));

        let spaced = self
            .samples
            .back()
            .is_none_or(|(last_at, _)| at - *last_at >= MIN_GAP);
        if spaced {
            self.samples.push_back((at, fraction));
        }
        while self.samples.len() > 2 && self.samples.front().is_some_and(|(t, _)| at - *t > WINDOW)
        {
            self.samples.pop_front();
        }
    }

    pub fn reset(&mut self) {
        self.samples.clear();
        self.latest = None;
    }

    /// time left as of `now`, or none until there is enough to go on
    pub fn estimate(&self, now: Duration) -> Option<Duration> {
        let (oldest_at, oldest) = *self.samples.front()?;
        let (latest_at, latest) = self.latest?;
        let span = latest_at.checked_sub(oldest_at)?;
        let gained = latest - oldest;
        if span < MIN_SPAN || gained <= 0.0 {
            return None;
        }

        let rate = gained / span.as_secs_f32();
        let remaining = Duration::try_from_secs_f32((1.0 - latest).max(0.0) / rate).ok()?;
        // time has passed since the last report, count that off
        Some(remaining.saturating_sub(now.saturating_sub(latest_at)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    fn close(actual: Option<Duration>, expected_ms: u64) -> bool {
        actual.is_some_and(|actual| actual.abs_diff(ms(expected_ms)) <= ms(50))
    }

    #[test]
    fn steady_progress_gives_a_steady_estimate() {
        let mut eta = EtaEstimator::default();
        for step in 0..=50u64 {
            eta.record(ms(step * 100), step as f32 / 100.0);
        }
        // half way after 5 seconds, so 5 more to go
        assert!(
            close(eta.estimate(ms(5000)), 5000),
            "{:?}",
            eta.estimate(ms(5000))
        );
    }

    #[test]
    fn there_is_no_estimate_before_enough_was_seen() {
        let mut eta = EtaEstimator::default();
        assert_eq!(eta.estimate(ms(0)), None);
        eta.record(ms(0), 0.0);
        eta.record(ms(300), 0.1);
        assert_eq!(eta.estimate(ms(300)), None);
    }

    #[test]
    fn a_stalled_job_has_no_estimate() {
        let mut eta = EtaEstimator::default();
        eta.record(ms(0), 0.2);
        eta.record(ms(3000), 0.2);
        assert_eq!(eta.estimate(ms(3000)), None);
    }

    #[test]
    fn the_estimate_counts_down_between_reports() {
        let mut eta = EtaEstimator::default();
        eta.record(ms(0), 0.0);
        eta.record(ms(2000), 0.5);
        assert!(close(eta.estimate(ms(2000)), 2000));
        assert!(close(eta.estimate(ms(3000)), 1000));
        assert!(close(eta.estimate(ms(9000)), 0));
    }

    #[test]
    fn a_slowdown_shows_up_once_old_samples_leave_the_window() {
        let mut eta = EtaEstimator::default();
        // fast for 10 seconds: 4% a second
        for second in 0..=10u64 {
            eta.record(ms(second * 1000), second as f32 * 0.04);
        }
        // then slow for a minute: 0.2% a second
        for second in 1..=60u64 {
            eta.record(ms(10_000 + second * 1000), 0.4 + second as f32 * 0.002);
        }
        // 48% left at 0.2% a second is 4 minutes
        let estimate = eta.estimate(ms(70_000)).unwrap();
        assert!(
            estimate.abs_diff(Duration::from_secs(240)) < Duration::from_secs(10),
            "{estimate:?}"
        );
    }

    #[test]
    fn progress_going_backwards_starts_over() {
        let mut eta = EtaEstimator::default();
        eta.record(ms(0), 0.0);
        eta.record(ms(2000), 0.9);
        eta.record(ms(2100), 0.0);
        assert_eq!(eta.estimate(ms(2100)), None);
        eta.record(ms(4100), 0.5);
        assert!(close(eta.estimate(ms(4100)), 2000));
    }

    #[test]
    fn bursts_of_reports_do_not_grow_the_sample_list() {
        let mut eta = EtaEstimator::default();
        for step in 0..10_000u64 {
            eta.record(ms(step), step as f32 / 20_000.0);
        }
        assert!(eta.samples.len() <= 60, "{}", eta.samples.len());
        assert!(close(eta.estimate(ms(10_000)), 10_000));
    }
}
