//! time remaining estimates from progress samples.
//!
//! uses a smoothed rate (exponential moving average of progress per second) so a job that
//! speeds up or slows down settles on a sensible number instead of jumping around. time
//! spent paused does not count.

use std::time::{Duration, Instant};

/// samples closer together than this are skipped, they are mostly noise
const MIN_SAMPLE_GAP: Duration = Duration::from_millis(200);
/// no estimate until the job has run this long
const MIN_ELAPSED: Duration = Duration::from_secs(1);
/// weight of the newest rate sample
const SMOOTHING: f64 = 0.3;

#[derive(Debug, Clone, Default)]
pub(crate) struct EtaEstimator {
    /// running time from earlier segments, before the last pause
    active: Duration,
    /// start of the current running segment
    since: Option<Instant>,
    /// last sample as (running time, progress)
    last: Option<(Duration, f32)>,
    /// smoothed progress per second
    rate: Option<f64>,
}

impl EtaEstimator {
    pub fn start(&mut self, now: Instant) {
        if self.since.is_none() {
            self.since = Some(now);
        }
    }

    pub fn stop(&mut self, now: Instant) {
        if let Some(since) = self.since.take() {
            self.active += now.saturating_duration_since(since);
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn elapsed(&self, now: Instant) -> Duration {
        self.active
            + self
                .since
                .map_or(Duration::ZERO, |since| now.saturating_duration_since(since))
    }

    pub fn sample(&mut self, now: Instant, progress: f32) {
        let t = self.elapsed(now);
        let Some((t0, p0)) = self.last else {
            self.last = Some((t, progress));
            if !t.is_zero() && progress > 0.0 {
                self.rate = Some(f64::from(progress) / t.as_secs_f64());
            }
            return;
        };
        if progress < p0 {
            // progress went backwards (a second pass, say), start measuring again
            self.last = Some((t, progress));
            self.rate = None;
            return;
        }
        let gap = t.saturating_sub(t0);
        if gap < MIN_SAMPLE_GAP {
            return;
        }
        let instant_rate = f64::from(progress - p0) / gap.as_secs_f64();
        self.rate = Some(match self.rate {
            Some(rate) => rate + SMOOTHING * (instant_rate - rate),
            None => instant_rate,
        });
        self.last = Some((t, progress));
    }

    pub fn eta(&self, now: Instant, progress: Option<f32>) -> Option<Duration> {
        let progress = progress?;
        if progress >= 1.0 {
            return Some(Duration::ZERO);
        }
        if self.elapsed(now) < MIN_ELAPSED {
            return None;
        }
        let rate = self.rate.filter(|rate| *rate > 1e-9)?;
        Duration::try_from_secs_f64(f64::from(1.0 - progress) / rate).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: f64) -> Duration {
        Duration::from_secs_f64(s)
    }

    #[test]
    fn steady_progress_gives_the_obvious_estimate() {
        let base = Instant::now();
        let mut eta = EtaEstimator::default();
        eta.start(base);
        for i in 1..=5 {
            eta.sample(base + secs(f64::from(i)), i as f32 * 0.1);
        }
        let left = eta.eta(base + secs(5.0), Some(0.5)).unwrap();
        assert!((left.as_secs_f64() - 5.0).abs() < 0.01, "{left:?}");
    }

    #[test]
    fn no_estimate_too_early_or_without_progress() {
        let base = Instant::now();
        let mut eta = EtaEstimator::default();
        eta.start(base);
        eta.sample(base + secs(0.5), 0.2);
        assert_eq!(eta.eta(base + secs(0.5), Some(0.2)), None);
        assert_eq!(eta.eta(base + secs(3.0), None), None);
        assert_eq!(eta.eta(base + secs(3.0), Some(1.0)), Some(Duration::ZERO));
    }

    #[test]
    fn paused_time_does_not_count() {
        let base = Instant::now();
        let mut eta = EtaEstimator::default();
        eta.start(base);
        eta.sample(base + secs(2.0), 0.2);
        eta.stop(base + secs(2.0));
        // ten seconds paused
        eta.start(base + secs(12.0));
        assert_eq!(eta.elapsed(base + secs(14.0)), secs(4.0));
        eta.sample(base + secs(14.0), 0.4);
        let left = eta.eta(base + secs(14.0), Some(0.4)).unwrap();
        assert!((left.as_secs_f64() - 6.0).abs() < 0.01, "{left:?}");
    }

    #[test]
    fn smoothing_follows_a_change_in_speed() {
        let base = Instant::now();
        let mut eta = EtaEstimator::default();
        eta.start(base);
        let mut progress = 0.0;
        for i in 1..=4 {
            progress += 0.1;
            eta.sample(base + secs(f64::from(i)), progress);
        }
        // the job slows down to a quarter of the speed
        for i in 5..=20 {
            progress += 0.025;
            eta.sample(base + secs(f64::from(i)), progress);
        }
        let left = eta.eta(base + secs(20.0), Some(progress)).unwrap();
        let expected = f64::from(1.0 - progress) / 0.025;
        assert!(
            (left.as_secs_f64() - expected).abs() / expected < 0.05,
            "{left:?} vs {expected}"
        );
    }

    #[test]
    fn going_backwards_restarts_the_measurement() {
        let base = Instant::now();
        let mut eta = EtaEstimator::default();
        eta.start(base);
        eta.sample(base + secs(2.0), 0.9);
        eta.sample(base + secs(3.0), 0.1);
        assert_eq!(eta.eta(base + secs(3.0), Some(0.1)), None);
        eta.sample(base + secs(4.0), 0.2);
        let left = eta.eta(base + secs(4.0), Some(0.2)).unwrap();
        assert!((left.as_secs_f64() - 8.0).abs() < 0.01, "{left:?}");
    }

    #[test]
    fn tiny_gaps_are_ignored() {
        let base = Instant::now();
        let mut eta = EtaEstimator::default();
        eta.start(base);
        eta.sample(base + secs(1.0), 0.1);
        eta.sample(base + secs(1.01), 0.5);
        let left = eta.eta(base + secs(1.01), Some(0.5)).unwrap();
        // still based on the first sample's rate of 0.1 per second
        assert!((left.as_secs_f64() - 5.0).abs() < 0.01, "{left:?}");
    }
}
