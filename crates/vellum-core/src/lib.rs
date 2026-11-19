//! vellum-core: the pieces every other crate builds on.
//!
//! - [`command`]: the one command type all actions go through
//! - [`history`]: branching undo/redo with named snapshots and macro recording
//! - [`events`]: the typed event bus shared with the ui
//! - [`settings`]: versioned json settings with migrations
//! - [`jobs`]: the background job queue
//!
//! see ARCHITECTURE.md in the repo root for why things are shaped this way.

pub mod command;
pub mod events;
pub mod history;
pub mod jobs;
pub mod settings;

pub use command::{Command, CommandError, CommandInfo, CommandRecord, CommandRegistry, Macro};
pub use events::{EventBus, VellumEvent, UI_EVENT_CHANNEL};
pub use history::{History, HistoryEntry, HistoryState};
pub use jobs::{
    Job, JobContext, JobError, JobEvent, JobId, JobInfo, JobKind, JobQueue, JobSpec, JobStatus,
    Priority, QueueConfig,
};
pub use settings::{Settings, SettingsStore};
