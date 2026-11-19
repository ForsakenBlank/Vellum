//! the shared core of vellum.
//!
//! every other crate builds on the pieces in here:
//!
//! - [`command`]: the `Command` trait, the command registry and macro records
//! - [`history`]: branching undo/redo history with named snapshots
//! - [`events`]: the typed event bus that also feeds the ui
//! - [`settings`]: versioned json settings with migrations
//! - [`jobs`]: the job queue that runs all slow work
//! - [`ui`]: small types shared with the ui
//!
//! see `ARCHITECTURE.md` in the repo root for why things are shaped the way they are.

pub mod actions;
pub mod app;
pub mod command;
pub mod documents;
pub mod events;
pub mod files;
pub mod history;
pub mod jobs;
pub mod notifications;
pub mod recent;
pub mod recovery;
pub mod settings;
pub mod shortcuts;
pub mod ui;

pub use command::{Command, CommandError, CommandInfo, CommandRecord, CommandRegistry};
pub use events::{Event, EventBus, EventSink, Subscription};
pub use history::{ExecuteOutcome, History, HistoryError, HistoryLimits};
pub use jobs::{JobContext, JobError, JobId, JobQueue, JobResource, JobSpec, JobState};
pub use settings::{AppSettings, SettingsSchema, SettingsStore};
