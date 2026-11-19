//! the job queue for anything slow: exports, conversions, ai, proxies, thumbnails.
//!
//! jobs are cooperative. a job calls [`JobContext::checkpoint`] every so often, which is where
//! pause and cancel take effect, and reports progress with [`JobContext::set_progress`].

mod queue;

use std::sync::Arc;
use std::time::Instant;

use parking_lot::{Condvar, Mutex};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub use queue::{JobQueue, JobQueueError, QueueConfig};

pub type JobId = u64;

/// which pool of workers a job uses. each kind has its own parallel limit, so a long gpu export
/// never stops a quick cpu thumbnail from starting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum JobKind {
    Cpu,
    Gpu,
    Io,
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS,
)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Priority {
    Low,
    #[default]
    Normal,
    High,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum JobStatus {
    Queued,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// everything the ui shows about a job.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JobInfo {
    #[ts(type = "number")]
    pub id: JobId,
    pub label: String,
    pub kind: JobKind,
    pub priority: Priority,
    pub status: JobStatus,
    /// 0.0 to 1.0
    pub progress: f32,
    pub message: Option<String>,
    pub eta_secs: Option<f64>,
    pub error: Option<String>,
    pub attempts: u32,
    /// whatever the job returned, usually `{ "path": "..." }` so the ui can open the result
    pub result: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum JobEvent {
    /// sent when a job is added or its status, priority or result changes
    Changed { job: JobInfo },
    /// sent often while a job runs, throttled by [`QueueConfig::progress_interval`]
    Progress {
        #[ts(type = "number")]
        id: JobId,
        progress: f32,
        eta_secs: Option<f64>,
        message: Option<String>,
    },
    Removed {
        #[ts(type = "number")]
        id: JobId,
    },
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum JobError {
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Failed(String),
}

impl JobError {
    pub fn failed(msg: impl Into<String>) -> Self {
        Self::Failed(msg.into())
    }
}

pub type JobOutput = Option<serde_json::Value>;

/// a unit of slow work. `run` may be called more than once if the user retries.
pub trait Job: Send + 'static {
    fn run(&mut self, ctx: &JobContext) -> Result<JobOutput, JobError>;
}

impl<F> Job for F
where
    F: FnMut(&JobContext) -> Result<JobOutput, JobError> + Send + 'static,
{
    fn run(&mut self, ctx: &JobContext) -> Result<JobOutput, JobError> {
        self(ctx)
    }
}

/// how a job should be queued.
#[derive(Clone, Debug)]
pub struct JobSpec {
    pub label: String,
    pub kind: JobKind,
    pub priority: Priority,
    /// failed runs are retried automatically until this many attempts have been made
    pub max_attempts: u32,
}

impl JobSpec {
    pub fn new(label: impl Into<String>, kind: JobKind) -> Self {
        Self {
            label: label.into(),
            kind,
            priority: Priority::Normal,
            max_attempts: 1,
        }
    }

    pub fn priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    pub fn max_attempts(mut self, attempts: u32) -> Self {
        self.max_attempts = attempts.max(1);
        self
    }
}

#[derive(Default)]
struct ControlState {
    paused: bool,
    cancelled: bool,
}

/// pause and cancel flags shared between the queue and one running job.
#[derive(Default)]
pub(crate) struct Control {
    state: Mutex<ControlState>,
    wake: Condvar,
}

impl Control {
    pub(crate) fn set_paused(&self, paused: bool) {
        self.state.lock().paused = paused;
        self.wake.notify_all();
    }

    pub(crate) fn cancel(&self) {
        self.state.lock().cancelled = true;
        self.wake.notify_all();
    }

    pub(crate) fn reset(&self) {
        *self.state.lock() = ControlState::default();
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.state.lock().cancelled
    }
}

/// handed to a running job so it can report progress and honour pause and cancel.
pub struct JobContext {
    id: JobId,
    control: Arc<Control>,
    queue: Arc<queue::Shared>,
    started: Instant,
}

impl JobContext {
    pub fn id(&self) -> JobId {
        self.id
    }

    /// returns `Err(JobError::Cancelled)` if the job was cancelled, and blocks while it is paused.
    /// use it with `?` inside loops.
    pub fn checkpoint(&self) -> Result<(), JobError> {
        let mut state = self.control.state.lock();
        loop {
            if state.cancelled {
                return Err(JobError::Cancelled);
            }
            if !state.paused {
                return Ok(());
            }
            self.control.wake.wait(&mut state);
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.control.is_cancelled()
    }

    /// `progress` is clamped to 0.0..=1.0.
    pub fn set_progress(&self, progress: f32) {
        self.queue.report(self.id, Some(progress), None);
    }

    pub fn set_message(&self, message: impl Into<String>) {
        self.queue.report(self.id, None, Some(message.into()));
    }

    /// time since this attempt started, including any time spent paused.
    pub fn elapsed(&self) -> std::time::Duration {
        self.started.elapsed()
    }
}

#[cfg(test)]
mod tests;
