use serde::Serialize;
use ts_rs::TS;
use vellum_core::jobs::JobQueueError;
use vellum_core::settings::SettingsError;

/// what a failed command hands to the ui. `message` is written for the user
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct IpcError {
    /// stable and machine readable, such as `job.not_found`
    pub code: String,
    pub message: String,
}

impl IpcError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

impl From<JobQueueError> for IpcError {
    fn from(error: JobQueueError) -> Self {
        match error {
            JobQueueError::NotFound(_) => {
                Self::new("job.not_found", "That job is no longer in the queue.")
            }
            JobQueueError::NotAllowed { action, state, .. } => Self::new(
                "job.not_allowed",
                format!(
                    "This job is {} and cannot {action} right now.",
                    describe(state)
                ),
            ),
        }
    }
}

fn describe(state: vellum_core::jobs::JobState) -> &'static str {
    use vellum_core::jobs::JobState::*;
    match state {
        Queued => "waiting to start",
        Running => "running",
        Pausing => "pausing",
        Paused => "paused",
        Cancelling => "being cancelled",
        Succeeded => "already finished",
        Failed => "already stopped with an error",
        Cancelled => "already cancelled",
    }
}

impl From<SettingsError> for IpcError {
    fn from(error: SettingsError) -> Self {
        let code = match &error {
            SettingsError::Io { .. } => "settings.io",
            SettingsError::UnknownSection(_) => "settings.unknown_section",
            SettingsError::Invalid { .. } => "settings.invalid",
            SettingsError::Migration { .. } => "settings.migration",
        };
        let message = match &error {
            SettingsError::Io { .. } => format!(
                "Your settings could not be saved. Check that the disk is not full or read only. ({error})"
            ),
            _ => format!("That setting was not accepted: {error}."),
        };
        Self::new(code, message)
    }
}

#[cfg(test)]
mod tests {
    use vellum_core::event::EventBus;
    use vellum_core::jobs::{JobContext, JobQueue, JobQueueConfig, JobSpec};

    use super::*;

    #[test]
    fn job_errors_read_as_sentences() {
        let queue = JobQueue::new(EventBus::new(), JobQueueConfig::default()).unwrap();
        let handle = queue.submit(JobSpec::cpu("Done"), |_: &JobContext| Ok(().into()));
        handle.wait().unwrap();

        let error = IpcError::from(queue.pause(handle.id()).unwrap_err());
        assert_eq!(error.code, "job.not_allowed");
        assert_eq!(
            error.message,
            "This job is already finished and cannot be paused right now."
        );

        queue.remove(handle.id()).unwrap();
        let error = IpcError::from(queue.pause(handle.id()).unwrap_err());
        assert_eq!(error.code, "job.not_found");
    }
}
