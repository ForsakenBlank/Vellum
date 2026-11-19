//! recently opened files and pinned projects for the home screen.
//!
//! stored with the same [`SettingsStore`] as the settings (atomic writes, backups, recovery
//! from bad files), just in its own `recent.json`, so a long list never bloats settings.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::files::{FileKind, display_name};
use crate::settings::{SettingsError, SettingsSchema, SettingsStore};

/// unpinned entries beyond this are dropped, oldest first
pub const MAX_UNPINNED: usize = 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecentFile {
    pub path: String,
    pub name: String,
    pub kind: Option<FileKind>,
    #[ts(type = "number")]
    pub opened_at_ms: u64,
    #[ts(type = "number | null")]
    pub size_bytes: Option<u64>,
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct RecentFiles {
    /// newest first, pinned and unpinned mixed
    pub files: Vec<RecentFile>,
}

impl SettingsSchema for RecentFiles {
    const VERSION: u32 = 1;
    const CHANGED_EVENT: &'static str = "recent:changed";

    fn sanitize(&mut self) {
        let mut seen = Vec::new();
        self.files.retain(|file| {
            let keep = !file.path.is_empty() && !seen.contains(&file.path);
            seen.push(file.path.clone());
            keep
        });
        self.files
            .sort_by_key(|file| std::cmp::Reverse(file.opened_at_ms));
        let mut unpinned = 0;
        self.files.retain(|file| {
            if file.pinned {
                return true;
            }
            unpinned += 1;
            unpinned <= MAX_UNPINNED
        });
    }
}

pub type RecentStore = SettingsStore<RecentFiles>;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

/// adds or refreshes a file at the top of the list, keeping its pin
pub fn record_open(store: &RecentStore, path: &Path) -> Result<RecentFiles, SettingsError> {
    let key = path.display().to_string();
    let size_bytes = std::fs::metadata(path).ok().map(|m| m.len());
    store.update(|recent| {
        let pinned = recent.files.iter().any(|f| f.path == key && f.pinned);
        recent.files.retain(|f| f.path != key);
        recent.files.insert(
            0,
            RecentFile {
                path: key.clone(),
                name: display_name(path),
                kind: FileKind::of_path(path),
                opened_at_ms: now_ms(),
                size_bytes,
                pinned,
            },
        );
    })
}

pub fn set_pinned(
    store: &RecentStore,
    path: &str,
    pinned: bool,
) -> Result<RecentFiles, SettingsError> {
    store.update(|recent| {
        for file in recent.files.iter_mut().filter(|f| f.path == path) {
            file.pinned = pinned;
        }
    })
}

pub fn remove(store: &RecentStore, path: &str) -> Result<RecentFiles, SettingsError> {
    store.update(|recent| recent.files.retain(|f| f.path != path))
}

/// forgets every unpinned file. pins stay, since the user chose them.
pub fn clear_unpinned(store: &RecentStore) -> Result<RecentFiles, SettingsError> {
    store.update(|recent| recent.files.retain(|f| f.pinned))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBus;
    use crate::settings::SettingsChanged;
    use std::path::PathBuf;

    fn store() -> (tempfile::TempDir, RecentStore, EventBus) {
        let dir = tempfile::tempdir().unwrap();
        let bus = EventBus::new();
        let (store, _) =
            RecentStore::open(dir.path().join("recent.json"), Some(bus.clone())).unwrap();
        (dir, store, bus)
    }

    #[test]
    fn opening_moves_a_file_to_the_top() {
        let (dir, store, _) = store();
        let clip = dir.path().join("clip.mp4");
        std::fs::write(&clip, b"12345").unwrap();
        record_open(&store, &clip).unwrap();
        record_open(&store, Path::new("/x/photo.png")).unwrap();
        let recent = record_open(&store, &clip).unwrap();
        let names: Vec<_> = recent.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["clip.mp4", "photo.png"]);
        assert_eq!(recent.files[0].kind, Some(FileKind::Video));
        assert_eq!(recent.files[0].size_bytes, Some(5));
        assert_eq!(recent.files[1].size_bytes, None);
    }

    #[test]
    fn pins_survive_reopening_and_clearing() {
        let (_dir, store, _) = store();
        record_open(&store, Path::new("/a/trip.vellum")).unwrap();
        record_open(&store, Path::new("/a/b.png")).unwrap();
        set_pinned(&store, "/a/trip.vellum", true).unwrap();
        let recent = record_open(&store, Path::new("/a/trip.vellum")).unwrap();
        assert!(recent.files[0].pinned);

        let recent = clear_unpinned(&store).unwrap();
        assert_eq!(recent.files.len(), 1);
        assert_eq!(recent.files[0].path, "/a/trip.vellum");

        let recent = remove(&store, "/a/trip.vellum").unwrap();
        assert!(recent.files.is_empty());
    }

    #[test]
    fn the_list_is_capped_but_pins_are_kept() {
        let (_dir, store, _) = store();
        record_open(&store, Path::new("/pinned.png")).unwrap();
        set_pinned(&store, "/pinned.png", true).unwrap();
        for n in 0..(MAX_UNPINNED + 5) {
            record_open(&store, &PathBuf::from(format!("/f{n}.png"))).unwrap();
        }
        let files = store.get().files;
        assert_eq!(files.iter().filter(|f| !f.pinned).count(), MAX_UNPINNED);
        assert!(files.iter().any(|f| f.pinned));
    }

    #[test]
    fn changes_are_saved_and_announced() {
        let (dir, store, bus) = store();
        let changes = bus.subscribe::<SettingsChanged<RecentFiles>>();
        record_open(&store, Path::new("/a.png")).unwrap();
        assert_eq!(changes.try_recv().unwrap().settings.files.len(), 1);
        let (reopened, _) = RecentStore::open(dir.path().join("recent.json"), None).unwrap();
        assert_eq!(reopened.get().files[0].path, "/a.png");
    }
}
