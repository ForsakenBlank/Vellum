//! how media work reports progress and hears about cancel and pause.
//!
//! the functions in this crate take a `&dyn Control` instead of a job context, so they run
//! the same inside a job, in the cli and in tests.

use vellum_core::{JobContext, JobError};

use crate::error::MediaError;

pub trait Control {
    /// returns `Err(Cancelled)` when the work should stop, and blocks while paused
    fn checkpoint(&self) -> Result<(), MediaError>;
    /// 0 to 1
    fn progress(&self, fraction: f32);
    fn message(&self, _text: &str) {}
}

impl Control for JobContext {
    fn checkpoint(&self) -> Result<(), MediaError> {
        JobContext::checkpoint(self).map_err(|err| match err {
            JobError::Cancelled => MediaError::Cancelled,
            JobError::Failed { message, .. } => MediaError::Unsupported(message),
        })
    }

    fn progress(&self, fraction: f32) {
        self.set_progress(fraction);
    }

    fn message(&self, text: &str) {
        self.set_message(text);
    }
}

/// for work nobody can cancel, like a probe the ui is waiting on
pub struct Unattended;

impl Control for Unattended {
    fn checkpoint(&self) -> Result<(), MediaError> {
        Ok(())
    }

    fn progress(&self, _fraction: f32) {}
}

/// maps progress from one step onto a slice of the whole, eg pass 2 of 2 is 0.5 to 1
pub struct Span<'a> {
    pub inner: &'a dyn Control,
    pub start: f32,
    pub end: f32,
}

impl Control for Span<'_> {
    fn checkpoint(&self) -> Result<(), MediaError> {
        self.inner.checkpoint()
    }

    fn progress(&self, fraction: f32) {
        let fraction = fraction.clamp(0.0, 1.0);
        self.inner
            .progress(self.start + (self.end - self.start) * fraction);
    }

    fn message(&self, text: &str) {
        self.inner.message(text);
    }
}
