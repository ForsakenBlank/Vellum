//! the job queue. anything slow (exports, ai, proxies, conversions, downloads) runs here so
//! the ui never blocks.
//!
//! jobs are plain functions that get a [`JobContext`]. they report progress through it and
//! call [`JobContext::checkpoint`] in their loops, which is where pause and cancel take
//! effect. both are cooperative: a job that never checks just runs to the end.
//!
//! each job needs one slot of a [`JobResource`] (cpu, gpu or io) and every resource has its
//! own limit, so a long gpu render does not stop a quick cpu conversion from starting. a
//! paused job gives its slot back while it waits.

mod eta;
mod queue;

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::events::Event;
use crate::settings::JobSettings;

pub use queue::{JobContext, JobQueue};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobId(#[ts(type = "number")] pub u64);

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "job {}", self.0)
    }
}

/// which pool of slots a job runs in
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum JobResource {
    /// encoding, decoding, image maths
    Cpu,
    /// gpu renders and ai inference
    Gpu,
    /// downloads and big file copies
    Io,
}

impl JobResource {
    pub const ALL: [JobResource; 3] = [Self::Cpu, Self::Gpu, Self::Io];

    fn index(self) -> usize {
        match self {
            Self::Cpu => 0,
            Self::Gpu => 1,
            Self::Io => 2,
        }
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS,
)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum JobPriority {
    Low,
    #[default]
    Normal,
    High,
    /// for work the user is actively waiting on, like a preview
    Urgent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum JobState {
    Queued,
    Running,
    /// pause was asked for, the job stops at its next checkpoint
    Pausing,
    Paused,
    /// cancel was asked for, the job stops at its next checkpoint
    Cancelling,
    /// failed and will run again after a short wait
    WaitingToRetry,
    Completed,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

impl fmt::Display for JobState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Pausing => "pausing",
            Self::Paused => "paused",
            Self::Cancelling => "cancelling",
            Self::WaitingToRetry => "waiting to retry",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        };
        f.write_str(text)
    }
}

/// what to run and how
#[derive(Debug, Clone, PartialEq)]
pub struct JobSpec {
    /// shown in the job panel, eg "Export holiday.mp4 to 10 MB"
    pub title: String,
    pub resource: JobResource,
    pub priority: JobPriority,
    /// automatic retries after a retryable failure
    pub max_retries: u32,
    /// wait before the first retry, doubled for each one after
    pub retry_delay: Duration,
    /// housekeeping the user did not ask for directly, like thumbnails. the jobs panel hides
    /// it, it never notifies, and it leaves the queue as soon as it finishes.
    pub background: bool,
    /// the document that started it, eg a quick tool session, so that view can list its own
    /// runs. none for jobs nobody owns.
    pub owner: Option<String>,
}

impl JobSpec {
    pub fn new(title: impl Into<String>, resource: JobResource) -> Self {
        Self {
            title: title.into(),
            resource,
            priority: JobPriority::Normal,
            max_retries: 0,
            retry_delay: Duration::from_secs(2),
            background: false,
            owner: None,
        }
    }

    pub fn cpu(title: impl Into<String>) -> Self {
        Self::new(title, JobResource::Cpu)
    }

    pub fn gpu(title: impl Into<String>) -> Self {
        Self::new(title, JobResource::Gpu)
    }

    pub fn io(title: impl Into<String>) -> Self {
        Self::new(title, JobResource::Io)
    }

    pub fn with_priority(mut self, priority: JobPriority) -> Self {
        self.priority = priority;
        self
    }

    /// see [`JobSpec::background`]
    pub fn background(mut self) -> Self {
        self.background = true;
        self
    }

    /// see [`JobSpec::owner`]
    pub fn owned_by(mut self, document: impl Into<String>) -> Self {
        self.owner = Some(document.into());
        self
    }

    pub fn with_retries(mut self, max_retries: u32, delay: Duration) -> Self {
        self.max_retries = max_retries;
        self.retry_delay = delay;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JobError {
    #[error("cancelled")]
    Cancelled,
    #[error("{message}")]
    Failed { message: String, retryable: bool },
}

impl JobError {
    /// a failure that might go away if tried again, like a network error
    pub fn failed(message: impl fmt::Display) -> Self {
        Self::Failed {
            message: message.to_string(),
            retryable: true,
        }
    }

    /// a failure that retrying cannot fix, like a missing input file
    pub fn fatal(message: impl fmt::Display) -> Self {
        Self::Failed {
            message: message.to_string(),
            retryable: false,
        }
    }
}

impl From<std::io::Error> for JobError {
    fn from(err: std::io::Error) -> Self {
        Self::failed(err)
    }
}

impl From<serde_json::Error> for JobError {
    fn from(err: serde_json::Error) -> Self {
        Self::fatal(err)
    }
}

/// what a job hands back when it finishes, eg `{ "path": "out.mp4" }` for "open when done"
pub type JobOutput = serde_json::Value;

/// the work itself. it is `FnMut` rather than `FnOnce` so the same task can run again on retry.
pub trait JobTask: Send + 'static {
    fn run(&mut self, ctx: &JobContext) -> Result<JobOutput, JobError>;
}

impl<F> JobTask for F
where
    F: FnMut(&JobContext) -> Result<JobOutput, JobError> + Send + 'static,
{
    fn run(&mut self, ctx: &JobContext) -> Result<JobOutput, JobError> {
        self(ctx)
    }
}

/// the state of a job as the ui sees it
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JobSnapshot {
    pub id: JobId,
    pub title: String,
    pub resource: JobResource,
    pub priority: JobPriority,
    pub state: JobState,
    /// 0 to 1, or none when the job cannot tell
    pub progress: Option<f32>,
    pub message: Option<String>,
    pub eta_secs: Option<f64>,
    /// time spent running, not counting pauses or time in the queue
    pub elapsed_secs: f64,
    /// 0 on the first run, then counts automatic retries
    pub attempt: u32,
    pub max_retries: u32,
    /// housekeeping like thumbnails, hidden from the jobs panel and never notified
    pub background: bool,
    /// the document that started it, see `JobSpec::owner`
    pub owner: Option<String>,
    pub error: Option<String>,
    #[ts(type = "unknown")]
    pub result: Option<JobOutput>,
    #[ts(type = "number")]
    pub created_at_ms: u64,
    /// increases with every update across all jobs. events can arrive out of order across
    /// threads, so the ui drops any snapshot older than the one it already has.
    #[ts(type = "number")]
    pub revision: u64,
}

/// the whole queue at one moment, for the ui to start from
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct JobsOverview {
    /// oldest first
    pub jobs: Vec<JobSnapshot>,
    /// waiting jobs in start order
    pub waiting: Vec<JobId>,
    /// every event after this moment has a higher revision
    #[ts(type = "number")]
    pub revision: u64,
}

/// sent on the bus whenever the queue changes
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum JobEvent {
    Updated {
        job: JobSnapshot,
    },
    Removed {
        id: JobId,
        #[ts(type = "number")]
        revision: u64,
    },
    /// the jobs waiting to start, in the order they will start
    Order {
        waiting: Vec<JobId>,
        #[ts(type = "number")]
        revision: u64,
    },
}

impl Event for JobEvent {
    const NAME: &'static str = "job:changed";
}

/// how many jobs of each resource may run at once
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobLimits {
    pub cpu: usize,
    pub gpu: usize,
    pub io: usize,
}

impl JobLimits {
    pub fn get(&self, resource: JobResource) -> usize {
        let limit = match resource {
            JobResource::Cpu => self.cpu,
            JobResource::Gpu => self.gpu,
            JobResource::Io => self.io,
        };
        // a limit of zero would stall the queue forever
        limit.max(1)
    }
}

impl Default for JobLimits {
    fn default() -> Self {
        Self::from(&JobSettings::default())
    }
}

impl From<&JobSettings> for JobLimits {
    fn from(settings: &JobSettings) -> Self {
        let to_usize = |n: u32| usize::try_from(n).unwrap_or(1);
        Self {
            cpu: to_usize(settings.max_cpu_jobs),
            gpu: to_usize(settings.max_gpu_jobs),
            io: to_usize(settings.max_io_jobs),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JobQueueError {
    #[error("{0} does not exist")]
    UnknownJob(JobId),
    #[error("cannot {action} {id} while it is {state}")]
    NotAllowed {
        id: JobId,
        action: &'static str,
        state: JobState,
    },
}
