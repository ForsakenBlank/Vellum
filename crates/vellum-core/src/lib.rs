//! the foundations every other vellum crate builds on.
//!
//! - [`command`]: the one `Command` type behind every user action
//! - [`history`]: branching undo/redo with named snapshots
//! - [`event`]: the typed event bus that also feeds the ui
//! - [`settings`]: versioned json settings with migrations
//! - [`jobs`]: the queue all slow work runs on
//! - [`shell`]: open documents, windows, recent files, autosave and recovery

#![forbid(unsafe_code)]

pub mod command;
pub mod event;
pub mod history;
pub mod jobs;
pub mod settings;
pub mod shell;

mod fs;
mod time;
