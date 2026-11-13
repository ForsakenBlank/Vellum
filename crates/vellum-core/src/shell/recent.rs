//! recently opened files and pinned ones, for the home screen and the palette.
//!
//! kept in its own json file next to the settings. it is history rather than
//! a preference, it changes every time a file is opened, and losing it is an
//! annoyance rather than a problem, so a broken file is simply started over.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::files::{MediaKind, classify, file_name};
use crate::event::{Event, EventBus};
use crate::fs::write_atomically;
use crate::time::unix_ms;

pub const DEFAULT_RECENT_LIMIT: usize = 30;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecentEntry {
    pub path: String,
    pub name: String,
    pub kind: MediaKind,
    pub opened_ms: u64,
    pub pinned: bool,
    /// false when the file was moved or deleted since. worked out when listed
    #[serde(default = "yes")]
    pub exists: bool,
}

fn yes() -> bool {
    true
}

/// published with the whole list whenever it changes
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RecentChanged {
    pub entries: Vec<RecentEntry>,
}

impl Event for RecentChanged {
    const NAME: &'static str = "recent.changed";
}

#[derive(Default, Serialize, Deserialize)]
struct RecentFile {
    entries: Vec<RecentEntry>,
}

struct Inner {
    path: Option<PathBuf>,
    bus: Option<EventBus>,
    state: Mutex<State>,
}

struct State {
    /// newest first
    entries: Vec<RecentEntry>,
    limit: usize,
}

#[derive(Clone)]
pub struct RecentFiles {
    inner: Arc<Inner>,
}

impl RecentFiles {
    pub fn open(path: impl Into<PathBuf>, bus: Option<EventBus>) -> Self {
        let path = path.into();
        let entries = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<RecentFile>(&bytes) {
                Ok(file) => file.entries,
                Err(error) => {
                    tracing::warn!(%error, "the recent files list could not be read, starting over");
                    Vec::new()
                }
            },
            Err(error) => {
                if error.kind() != io::ErrorKind::NotFound {
                    tracing::warn!(%error, "the recent files list could not be read");
                }
                Vec::new()
            }
        };
        Self::build(Some(path), bus, entries)
    }

    pub fn in_memory(bus: Option<EventBus>) -> Self {
        Self::build(None, bus, Vec::new())
    }

    fn build(path: Option<PathBuf>, bus: Option<EventBus>, entries: Vec<RecentEntry>) -> Self {
        Self {
            inner: Arc::new(Inner {
                path,
                bus,
                state: Mutex::new(State {
                    entries,
                    limit: DEFAULT_RECENT_LIMIT,
                }),
            }),
        }
    }

    /// every entry, newest first, with `exists` checked against the disk
    pub fn list(&self) -> Vec<RecentEntry> {
        let mut entries = self.inner.state.lock().entries.clone();
        for entry in &mut entries {
            entry.exists = Path::new(&entry.path).exists();
        }
        entries
    }

    /// how many unpinned entries to keep. pinned ones never count
    pub fn set_limit(&self, limit: usize) {
        self.change(|state| {
            state.limit = limit;
            trim(state);
            true
        });
    }

    /// records that a file was opened just now
    pub fn touch(&self, path: &str) {
        self.touch_all(&[path.to_owned()]);
    }

    /// records several opened files in one write. the last one ends up on top
    pub fn touch_all(&self, paths: &[String]) {
        if paths.is_empty() {
            return;
        }
        self.change(|state| {
            let now = unix_ms();
            for path in paths {
                let pinned = state
                    .entries
                    .iter()
                    .find(|entry| &entry.path == path)
                    .is_some_and(|entry| entry.pinned);
                state.entries.retain(|entry| &entry.path != path);
                state.entries.insert(
                    0,
                    RecentEntry {
                        path: path.clone(),
                        name: file_name(path),
                        kind: classify(Path::new(path)),
                        opened_ms: now,
                        pinned,
                        exists: true,
                    },
                );
            }
            trim(state);
            true
        });
    }

    /// pins or unpins a path, adding it to the list if it was not there
    pub fn set_pinned(&self, path: &str, pinned: bool) {
        self.change(|state| {
            match state.entries.iter_mut().find(|entry| entry.path == path) {
                Some(entry) if entry.pinned == pinned => return false,
                Some(entry) => entry.pinned = pinned,
                None if !pinned => return false,
                None => state.entries.push(RecentEntry {
                    path: path.to_owned(),
                    name: file_name(path),
                    kind: classify(Path::new(path)),
                    opened_ms: unix_ms(),
                    pinned: true,
                    exists: true,
                }),
            }
            trim(state);
            true
        });
    }

    pub fn remove(&self, path: &str) {
        self.change(|state| {
            let before = state.entries.len();
            state.entries.retain(|entry| entry.path != path);
            state.entries.len() != before
        });
    }

    /// forgets everything that is not pinned
    pub fn clear(&self) {
        self.change(|state| {
            let before = state.entries.len();
            state.entries.retain(|entry| entry.pinned);
            state.entries.len() != before
        });
    }

    /// applies a change, then saves and announces it if it did anything
    fn change(&self, change: impl FnOnce(&mut State) -> bool) {
        let entries = {
            let mut state = self.inner.state.lock();
            if !change(&mut state) {
                return;
            }
            if let Some(path) = &self.inner.path {
                let file = RecentFile {
                    entries: state.entries.clone(),
                };
                let saved = serde_json::to_vec_pretty(&file)
                    .map_err(io::Error::other)
                    .and_then(|json| write_atomically(path, &json));
                if let Err(error) = saved {
                    // the list still works for this run, it just will not be remembered
                    tracing::warn!(%error, "the recent files list could not be saved");
                }
            }
            state.entries.clone()
        };
        if let Some(bus) = &self.inner.bus {
            bus.publish(RecentChanged {
                entries: self.with_exists(entries),
            });
        }
    }

    fn with_exists(&self, mut entries: Vec<RecentEntry>) -> Vec<RecentEntry> {
        for entry in &mut entries {
            entry.exists = Path::new(&entry.path).exists();
        }
        entries
    }
}

/// drops the oldest unpinned entries past the limit
fn trim(state: &mut State) {
    let mut unpinned = 0;
    let limit = state.limit;
    state.entries.retain(|entry| {
        if entry.pinned {
            return true;
        }
        unpinned += 1;
        unpinned <= limit
    });
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn paths(recent: &RecentFiles) -> Vec<String> {
        recent.list().into_iter().map(|entry| entry.name).collect()
    }

    #[test]
    fn newest_first_and_no_duplicates() {
        let recent = RecentFiles::in_memory(None);
        recent.touch("/a.png");
        recent.touch("/b.mp4");
        recent.touch("/a.png");
        assert_eq!(paths(&recent), ["a.png", "b.mp4"]);
        assert_eq!(recent.list()[1].kind, MediaKind::Video);
    }

    #[test]
    fn the_limit_only_counts_unpinned_files() {
        let recent = RecentFiles::in_memory(None);
        recent.touch("/keep.png");
        recent.set_pinned("/keep.png", true);
        recent.set_limit(2);
        for name in ["/1.png", "/2.png", "/3.png"] {
            recent.touch(name);
        }
        assert_eq!(paths(&recent), ["3.png", "2.png", "keep.png"]);
    }

    #[test]
    fn pinning_survives_being_opened_again_and_clearing() {
        let recent = RecentFiles::in_memory(None);
        recent.touch("/a.png");
        recent.touch("/b.png");
        recent.set_pinned("/a.png", true);
        recent.touch("/a.png");
        assert!(recent.list()[0].pinned);

        recent.clear();
        assert_eq!(paths(&recent), ["a.png"]);
        recent.set_pinned("/project.vellum", true);
        assert_eq!(recent.list()[1].kind, MediaKind::Project);
        recent.remove("/a.png");
        assert_eq!(paths(&recent), ["project.vellum"]);
    }

    #[test]
    fn the_list_survives_a_restart_and_notices_missing_files() {
        let dir = TempDir::new().unwrap();
        let real = dir.path().join("real.png");
        fs::write(&real, b"png").unwrap();
        let real = real.to_string_lossy().into_owned();
        let file = dir.path().join("recent.json");

        let recent = RecentFiles::open(&file, None);
        recent.touch("/gone/nowhere.png");
        recent.touch(&real);

        let reopened = RecentFiles::open(&file, None);
        let entries = reopened.list();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].exists);
        assert!(!entries[1].exists);
    }

    #[test]
    fn a_broken_file_starts_over() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("recent.json");
        fs::write(&file, b"nope").unwrap();
        assert!(RecentFiles::open(&file, None).list().is_empty());
    }

    #[test]
    fn changes_are_announced_once() {
        let bus = EventBus::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let _sub = bus.subscribe(move |event: &RecentChanged| sink.lock().push(event.clone()));
        let recent = RecentFiles::in_memory(Some(bus));

        recent.touch("/a.png");
        recent.remove("/missing.png");
        recent.set_pinned("/a.png", false);
        assert_eq!(seen.lock().len(), 1);
    }
}
