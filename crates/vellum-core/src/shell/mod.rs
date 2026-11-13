//! the state of the app shell that is not tied to any one document type:
//! which files are open and where, the recent list, autosave and crash
//! recovery, and the settings for shortcuts and panel layouts.
//!
//! nothing here knows about tauri, so it is tested like any other crate code.

pub mod autosave;
pub mod files;
pub mod recent;
pub mod session;
pub mod settings;
