//! the job queue: the one place slow work runs.
//!
//! exports, ai, proxies, thumbnails and conversions are all a [`Job`]. the queue
//! gives each of them progress, an eta, cancel, pause and retry, keeps the
//! number running at once within a limit per [`JobKind`], and reports
//! everything to the ui as [`JobEvent`]s.
//!
//! jobs are blocking code on their own thread. they cooperate by calling
//! [`JobContext::checkpoint`] often, which is where cancel and pause take effect.

mod eta;
mod queue;
#[cfg(test)]
mod tests;

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

pub use eta::EtaEstimator;
pub use queue::{CancelToken, JobContext, JobHandle, JobQueue, JobQueueConfig, JobsOverview};

use crate::event::Event;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobId(u64);

/// the resource a job mostly waits on. each kind has its own limit on how many run at once
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum JobKind {
    /// encoding, decoding, filters on the cpu
    Cpu,
    /// anything that fills gpu memory: ai models, gpu renders
    Gpu,
    /// downloads and file copies, which barely use the cpu
    Io,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize, TS,
)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Priority {
    /// housekeeping like thumbnails and proxies
    Low,
    #[default]
    Normal,
    High,
    /// what the user is looking at and waiting for right now
    Urgent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum JobState {
    Queued,
    Running,
    /// asked to pause, still working up to its next checkpoint
    Pausing,
    Paused,
    /// asked to cancel, still unwinding
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

/// how often to try again when a job fails with a retryable error
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// total tries, so 1 means no retry
    pub max_attempts: u32,
    /// wait before the second try. it doubles for each one after that
    pub backoff: Duration,
}

impl RetryPolicy {
    pub const NONE: Self = Self {
        max_attempts: 1,
        backoff: Duration::ZERO,
    };

    pub fn attempts(max_attempts: u32) -> Self {
        Self {
            max_attempts: max_attempts.max(1),
            backoff: Duration::from_secs(1),
        }
    }

    pub fn backoff(mut self, backoff: Duration) -> Self {
        self.backoff = backoff;
        self
    }

    /// the wait after try number `attempt` has failed
    fn delay_after(&self, attempt: u32) -> Duration {
        let doublings = attempt.saturating_sub(1).min(16);
        self.backoff
            .saturating_mul(1 << doublings)
            .min(Duration::from_secs(60))
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::NONE
    }
}

/// everything the queue needs to know about a job before it runs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobSpec {
    /// shown in the queue panel, such as "Export holiday.mp4"
    pub name: String,
    pub kind: JobKind,
    pub priority: Priority,
    pub retry: RetryPolicy,
    /// what sort of work this is ("export", "thumbnail"), for icons and filters
    pub category: Option<String>,
    /// housekeeping the user did not ask for. the ui keeps these out of the way
    pub background: bool,
}

impl JobSpec {
    pub fn new(name: impl Into<String>, kind: JobKind) -> Self {
        Self {
            name: name.into(),
            kind,
            priority: Priority::Normal,
            retry: RetryPolicy::NONE,
            category: None,
            background: false,
        }
    }

    pub fn cpu(name: impl Into<String>) -> Self {
        Self::new(name, JobKind::Cpu)
    }

    pub fn gpu(name: impl Into<String>) -> Self {
        Self::new(name, JobKind::Gpu)
    }

    pub fn io(name: impl Into<String>) -> Self {
        Self::new(name, JobKind::Io)
    }

    pub fn priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    pub fn retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn background(mut self) -> Self {
        self.background = true;
        self.priority = self.priority.min(Priority::Low);
        self
    }
}

/// how many jobs of each kind may run at the same time
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobLimits {
    pub cpu: usize,
    pub gpu: usize,
    pub io: usize,
}

impl JobLimits {
    /// encoders and filters already spread over every core, so a couple of
    /// them side by side is plenty
    pub fn default_cpu_jobs() -> usize {
        let cores = std::thread::available_parallelism().map_or(2, |cores| cores.get());
        (cores / 2).clamp(1, 4)
    }

    fn of(&self, kind: JobKind) -> usize {
        match kind {
            JobKind::Cpu => self.cpu,
            JobKind::Gpu => self.gpu,
            JobKind::Io => self.io,
        }
    }
}

impl Default for JobLimits {
    fn default() -> Self {
        Self {
            cpu: Self::default_cpu_jobs(),
            gpu: 1,
            io: 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JobError {
    #[error("cancelled")]
    Cancelled,
    #[error("{message}")]
    Failed {
        message: String,
        /// true when trying again might work, like a dropped connection
        retryable: bool,
    },
}

impl JobError {
    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed {
            message: message.into(),
            retryable: false,
        }
    }

    pub fn retryable(message: impl Into<String>) -> Self {
        Self::Failed {
            message: message.into(),
            retryable: true,
        }
    }
}

impl From<std::io::Error> for JobError {
    fn from(error: std::io::Error) -> Self {
        Self::failed(error.to_string())
    }
}

/// what a job hands back. the value travels to the ui with the finished job,
/// typically the path of the file it made
pub type JobResult = Result<Value, JobError>;

/// a unit of slow work. closures taking a [`JobContext`] are jobs too.
///
/// `run` is called again on the same object for every retry, so anything it
/// needs for a second try has to survive the first.
pub trait Job: Send + 'static {
    fn run(&mut self, ctx: &JobContext) -> JobResult;
}

impl<F> Job for F
where
    F: FnMut(&JobContext) -> JobResult + Send + 'static,
{
    fn run(&mut self, ctx: &JobContext) -> JobResult {
        self(ctx)
    }
}

/// the state of one job as the ui sees it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JobSnapshot {
    pub id: JobId,
    pub name: String,
    pub kind: JobKind,
    pub category: Option<String>,
    pub background: bool,
    pub priority: Priority,
    pub state: JobState,
    /// 0 to 1, or none while the job cannot tell how far along it is
    pub progress: Option<f32>,
    /// what the job is doing right now, such as "Pass 1 of 2"
    pub message: Option<String>,
    pub eta_ms: Option<u64>,
    /// time spent running, not counting time queued or paused
    pub elapsed_ms: u64,
    /// which try this is, starting at 1 once the job has started
    pub attempt: u32,
    pub max_attempts: u32,
    pub can_retry: bool,
    pub error: Option<String>,
    pub output: Option<Value>,
    pub created_ms: u64,
    pub started_ms: Option<u64>,
    pub finished_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum JobEvent {
    /// a job is new or changed. progress updates are rate limited
    Updated {
        job: JobSnapshot,
    },
    Removed {
        id: JobId,
    },
    /// the waiting jobs in the order they will start. only sent when a job
    /// lands somewhere other than the end of the queue
    Reordered {
        pending: Vec<JobId>,
    },
}

impl Event for JobEvent {
    const NAME: &'static str = "jobs";
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JobQueueError {
    #[error("that job no longer exists")]
    NotFound(JobId),
    #[error("a job that is {state:?} cannot {action}")]
    NotAllowed {
        id: JobId,
        action: &'static str,
        state: JobState,
    },
}
