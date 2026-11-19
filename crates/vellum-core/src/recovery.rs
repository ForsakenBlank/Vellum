//! autosave and crash recovery.
//!
//! every running vellum owns a session folder under `recovery/`:
//!
//! ```text
//! recovery/session-1763480000000-4242/
//!   lock            held with an exclusive os file lock while the session runs
//!   manifest.json   the open documents, when they were saved, and whether vellum exited
//!   doc-3.json      the state of each open document
//! ```
//!
//! autosave rewrites the folder every few seconds when something changed. a clean exit
//! deletes it unless documents still have unsaved changes. on the next launch, any folder
//! whose lock is free belongs to a session that is gone: if it did not exit cleanly that
//! was a crash, otherwise the user quit with unsaved work. either way the ui offers to
//! restore it, and nothing is deleted until the user restores or discards.
//!
//! the os lock (not a pid file) is what tells a live session from a dead one, so two
//! vellum windows running at once never offer each other's work.

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::documents::{DocumentError, DocumentInfo, DocumentManager, DocumentMeta, MAIN_WINDOW};
use crate::files::Workspace;
use crate::recent::now_ms;
use crate::settings::{SettingsError, write_atomic};

const MANIFEST: &str = "manifest.json";
const LOCK: &str = "lock";
const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum RecoveryError {
    #[error("could not use the recovery folder {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Write(#[from] SettingsError),
    #[error("a recovery file could not be read: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Document(#[from] DocumentError),
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> RecoveryError + '_ {
    move |source| RecoveryError::Io {
        path: path.to_owned(),
        source,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestDocument {
    doc_type: String,
    title: String,
    meta: DocumentMeta,
    dirty: bool,
    state_file: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    version: u32,
    started_at_ms: u64,
    saved_at_ms: u64,
    clean_exit: bool,
    documents: Vec<ManifestDocument>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RecoveryReason {
    /// vellum stopped without closing properly
    Crash,
    /// vellum was closed while documents had unsaved changes
    UnsavedAtExit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecoverableDocument {
    pub title: String,
    pub path: Option<String>,
    pub workspace: Workspace,
    pub dirty: bool,
}

/// what the restore prompt shows
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecoveryOffer {
    pub reason: RecoveryReason,
    #[ts(type = "number")]
    pub saved_at_ms: u64,
    pub documents: Vec<RecoverableDocument>,
}

/// a session folder from an earlier run, locked by us so no other instance claims it
struct Pending {
    dir: PathBuf,
    manifest: Manifest,
    _lock: File,
}

struct Autosaved {
    revision: Option<u64>,
    files: Vec<String>,
}

pub struct Recovery {
    session_dir: PathBuf,
    started_at_ms: u64,
    /// taken and dropped on a clean exit, since windows cannot delete an open file
    lock: Mutex<Option<File>>,
    pending: Mutex<Vec<Pending>>,
    autosaved: Mutex<Autosaved>,
}

impl Recovery {
    /// claims a new session folder under `root` and finds what earlier sessions left behind
    pub fn start(root: &Path) -> Result<Self, RecoveryError> {
        fs::create_dir_all(root).map_err(io_err(root))?;
        let pending = find_pending(root);

        let started_at_ms = now_ms();
        let session_dir = root.join(format!("session-{started_at_ms}-{}", std::process::id()));
        fs::create_dir_all(&session_dir).map_err(io_err(&session_dir))?;
        let lock_path = session_dir.join(LOCK);
        let lock = File::create(&lock_path).map_err(io_err(&lock_path))?;
        lock.lock().map_err(io_err(&lock_path))?;

        Ok(Self {
            session_dir,
            started_at_ms,
            lock: Mutex::new(Some(lock)),
            pending: Mutex::new(pending),
            autosaved: Mutex::new(Autosaved {
                revision: None,
                files: Vec::new(),
            }),
        })
    }

    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }

    /// what earlier sessions left behind, if anything
    pub fn offer(&self) -> Option<RecoveryOffer> {
        let pending = self.pending.lock();
        if pending.is_empty() {
            return None;
        }
        let reason = if pending.iter().any(|p| !p.manifest.clean_exit) {
            RecoveryReason::Crash
        } else {
            RecoveryReason::UnsavedAtExit
        };
        let saved_at_ms = pending
            .iter()
            .map(|p| p.manifest.saved_at_ms)
            .max()
            .unwrap_or_default();
        let documents = pending
            .iter()
            .flat_map(|p| &p.manifest.documents)
            .map(|doc| RecoverableDocument {
                title: doc.title.clone(),
                path: doc.meta.path.as_ref().map(|p| p.display().to_string()),
                workspace: doc.meta.workspace,
                dirty: doc.dirty,
            })
            .collect();
        Some(RecoveryOffer {
            reason,
            saved_at_ms,
            documents,
        })
    }

    /// opens every document from earlier sessions, then deletes their folders. a document
    /// that cannot be loaded is skipped and its folder kept, so nothing is thrown away.
    pub fn restore(&self, documents: &DocumentManager) -> Vec<DocumentInfo> {
        let pending = std::mem::take(&mut *self.pending.lock());
        let mut restored = Vec::new();
        for session in pending {
            let mut all_ok = true;
            for doc in &session.manifest.documents {
                match load_document(&session.dir, doc, documents) {
                    Ok(info) => restored.push(info),
                    Err(err) => {
                        all_ok = false;
                        tracing::error!(dir = ?session.dir, title = doc.title, %err,
                            "could not restore a document, its recovery files are kept");
                    }
                }
            }
            if all_ok {
                remove_session(session);
            }
        }
        restored
    }

    /// deletes what earlier sessions left behind
    pub fn discard(&self) {
        for session in std::mem::take(&mut *self.pending.lock()) {
            remove_session(session);
        }
    }

    /// writes every open document and the manifest if anything changed since last time.
    /// returns true if it wrote.
    pub fn autosave(&self, documents: &DocumentManager) -> Result<bool, RecoveryError> {
        self.write(documents, false, false)
    }

    /// the last save of a clean exit. if no document has unsaved changes the folder is
    /// deleted, otherwise it stays for the next launch to offer.
    pub fn finish(&self, documents: &DocumentManager) -> Result<(), RecoveryError> {
        self.write(documents, true, true)?;
        let (_, docs) = documents.snapshot_all();
        if docs.iter().all(|d| !d.dirty) {
            drop(self.lock.lock().take());
            fs::remove_dir_all(&self.session_dir).map_err(io_err(&self.session_dir))?;
        }
        Ok(())
    }

    fn write(
        &self,
        documents: &DocumentManager,
        force: bool,
        clean_exit: bool,
    ) -> Result<bool, RecoveryError> {
        let mut autosaved = self.autosaved.lock();
        let (revision, docs) = documents.snapshot_all();
        if !force && autosaved.revision == Some(revision) {
            return Ok(false);
        }

        let mut files = Vec::new();
        let mut entries = Vec::new();
        for doc in docs {
            let state_file = format!("{}.json", doc.id);
            let bytes = serde_json::to_vec(&doc.state)?;
            write_atomic(&self.session_dir.join(&state_file), &bytes)?;
            files.push(state_file.clone());
            entries.push(ManifestDocument {
                doc_type: doc.doc_type,
                title: doc.title,
                meta: doc.meta,
                dirty: doc.dirty,
                state_file,
            });
        }
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            started_at_ms: self.started_at_ms,
            saved_at_ms: now_ms(),
            clean_exit,
            documents: entries,
        };
        let bytes = serde_json::to_vec_pretty(&manifest)?;
        write_atomic(&self.session_dir.join(MANIFEST), &bytes)?;

        // state files of documents closed since the last autosave
        for old in autosaved.files.iter().filter(|f| !files.contains(f)) {
            let _ = fs::remove_file(self.session_dir.join(old));
        }
        autosaved.files = files;
        autosaved.revision = Some(revision);
        Ok(true)
    }
}

fn find_pending(root: &Path) -> Vec<Pending> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut pending = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let lock_path = dir.join(LOCK);
        let lock = match File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
        {
            Ok(file) => file,
            Err(err) => {
                tracing::warn!(?dir, %err, "could not check a recovery folder");
                continue;
            }
        };
        // a held lock means that session is still running in another window
        if lock.try_lock().is_err() {
            continue;
        }
        let manifest = fs::read(dir.join(MANIFEST))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Manifest>(&bytes).ok());
        match manifest {
            Some(manifest) if !manifest.documents.is_empty() => pending.push(Pending {
                dir,
                manifest,
                _lock: lock,
            }),
            // nothing to recover, or a session that never got as far as an autosave
            _ => {
                drop(lock);
                let _ = fs::remove_dir_all(&dir);
            }
        }
    }
    pending.sort_by_key(|p| p.manifest.saved_at_ms);
    pending
}

fn load_document(
    dir: &Path,
    doc: &ManifestDocument,
    documents: &DocumentManager,
) -> Result<DocumentInfo, RecoveryError> {
    let path = dir.join(&doc.state_file);
    let bytes = fs::read(&path).map_err(io_err(&path))?;
    let state = serde_json::from_slice(&bytes)?;
    let mut meta = doc.meta.clone();
    // torn off windows do not come back, their tabs go to the main window
    meta.window = MAIN_WINDOW.to_owned();
    Ok(documents.open_state(&doc.doc_type, state, meta, doc.dirty)?)
}

fn remove_session(session: Pending) {
    let Pending { dir, _lock, .. } = session;
    drop(_lock);
    if let Err(err) = fs::remove_dir_all(&dir) {
        tracing::warn!(?dir, %err, "could not delete an old recovery folder");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::CommandRecord;
    use crate::documents::{EditNotes, PlaceholderDocument};
    use crate::events::EventBus;
    use crate::files::FileKind;
    use crate::history::HistoryLimits;
    use serde_json::json;

    fn manager() -> DocumentManager {
        DocumentManager::new(EventBus::new(), HistoryLimits::default())
    }

    fn open(documents: &DocumentManager, name: &str) -> String {
        let path = PathBuf::from(format!("/media/{name}"));
        let kind = FileKind::of_path(&path).unwrap();
        documents
            .open(
                PlaceholderDocument::default(),
                DocumentMeta::for_file(path, kind),
            )
            .unwrap()
            .id
    }

    fn edit(documents: &DocumentManager, id: &str, notes: &str) {
        documents
            .execute(
                id,
                &CommandRecord::new(EditNotes::KIND, json!({ "notes": notes })),
            )
            .unwrap();
    }

    fn sessions(root: &Path) -> usize {
        fs::read_dir(root).unwrap().count()
    }

    #[test]
    fn a_fresh_start_offers_nothing() {
        let root = tempfile::tempdir().unwrap();
        let recovery = Recovery::start(root.path()).unwrap();
        assert!(recovery.offer().is_none());
        assert!(recovery.session_dir().join(LOCK).exists());
    }

    #[test]
    fn autosave_only_writes_when_something_changed() {
        let root = tempfile::tempdir().unwrap();
        let recovery = Recovery::start(root.path()).unwrap();
        let docs = manager();
        let id = open(&docs, "clip.mp4");
        assert!(recovery.autosave(&docs).unwrap());
        assert!(!recovery.autosave(&docs).unwrap());
        edit(&docs, &id, "trim the intro");
        assert!(recovery.autosave(&docs).unwrap());
        assert!(recovery.session_dir().join(format!("{id}.json")).exists());

        docs.close(&id).unwrap();
        assert!(recovery.autosave(&docs).unwrap());
        assert!(!recovery.session_dir().join(format!("{id}.json")).exists());
    }

    #[test]
    fn a_crash_is_offered_and_restored() {
        let root = tempfile::tempdir().unwrap();
        {
            let crashed = Recovery::start(root.path()).unwrap();
            let docs = manager();
            let id = open(&docs, "clip.mp4");
            open(&docs, "photo.png");
            edit(&docs, &id, "trim the intro");
            crashed.autosave(&docs).unwrap();
            // dropping without finish() is what a crash looks like
        }

        let recovery = Recovery::start(root.path()).unwrap();
        let offer = recovery.offer().unwrap();
        assert_eq!(offer.reason, RecoveryReason::Crash);
        assert_eq!(offer.documents.len(), 2);
        assert_eq!(offer.documents[0].title, "clip.mp4");
        assert!(offer.documents[0].dirty);
        assert!(!offer.documents[1].dirty);

        let docs = manager();
        let restored = recovery.restore(&docs);
        assert_eq!(restored.len(), 2);
        assert!(restored[0].dirty);
        assert_eq!(
            docs.state(&restored[0].id).unwrap().state["notes"],
            "trim the intro"
        );
        assert!(recovery.offer().is_none());
        // only our own session folder is left
        assert_eq!(sessions(root.path()), 1);
    }

    #[test]
    fn discarding_deletes_the_old_session() {
        let root = tempfile::tempdir().unwrap();
        {
            let crashed = Recovery::start(root.path()).unwrap();
            let docs = manager();
            open(&docs, "clip.mp4");
            crashed.autosave(&docs).unwrap();
        }
        let recovery = Recovery::start(root.path()).unwrap();
        assert!(recovery.offer().is_some());
        recovery.discard();
        assert!(recovery.offer().is_none());
        assert_eq!(sessions(root.path()), 1);
    }

    #[test]
    fn a_clean_exit_without_changes_leaves_nothing() {
        let root = tempfile::tempdir().unwrap();
        {
            let recovery = Recovery::start(root.path()).unwrap();
            let docs = manager();
            open(&docs, "clip.mp4");
            recovery.autosave(&docs).unwrap();
            recovery.finish(&docs).unwrap();
        }
        assert_eq!(sessions(root.path()), 0);
        let recovery = Recovery::start(root.path()).unwrap();
        assert!(recovery.offer().is_none());
    }

    #[test]
    fn a_clean_exit_with_unsaved_work_is_offered() {
        let root = tempfile::tempdir().unwrap();
        {
            let recovery = Recovery::start(root.path()).unwrap();
            let docs = manager();
            let id = open(&docs, "song.wav");
            edit(&docs, &id, "louder");
            recovery.finish(&docs).unwrap();
        }
        let recovery = Recovery::start(root.path()).unwrap();
        let offer = recovery.offer().unwrap();
        assert_eq!(offer.reason, RecoveryReason::UnsavedAtExit);
        assert_eq!(offer.documents[0].workspace, Workspace::Audio);
    }

    #[test]
    fn a_running_session_is_never_offered() {
        let root = tempfile::tempdir().unwrap();
        let first = Recovery::start(root.path()).unwrap();
        let docs = manager();
        open(&docs, "clip.mp4");
        first.autosave(&docs).unwrap();

        let second = Recovery::start(root.path()).unwrap();
        assert!(second.offer().is_none());
        assert!(first.session_dir().exists());
    }

    #[test]
    fn empty_and_broken_sessions_are_cleaned_up() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("session-1-1")).unwrap();
        let broken = root.path().join("session-2-2");
        fs::create_dir_all(&broken).unwrap();
        fs::write(broken.join(MANIFEST), b"{ not json").unwrap();

        let recovery = Recovery::start(root.path()).unwrap();
        assert!(recovery.offer().is_none());
        assert_eq!(sessions(root.path()), 1);
    }

    #[test]
    fn torn_off_documents_come_back_in_the_main_window() {
        let root = tempfile::tempdir().unwrap();
        {
            let crashed = Recovery::start(root.path()).unwrap();
            let docs = manager();
            let id = open(&docs, "clip.mp4");
            docs.move_to_window(&id, "doc-1", None).unwrap();
            crashed.autosave(&docs).unwrap();
        }
        let recovery = Recovery::start(root.path()).unwrap();
        let restored = recovery.restore(&manager());
        assert_eq!(restored[0].window, MAIN_WINDOW);
    }

    #[test]
    fn unknown_document_types_keep_their_files() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("session-5-5");
        fs::create_dir_all(&old).unwrap();
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            started_at_ms: 1,
            saved_at_ms: 2,
            clean_exit: false,
            documents: vec![ManifestDocument {
                doc_type: "hologram".into(),
                title: "Future".into(),
                meta: DocumentMeta::for_tool("x", vec![]),
                dirty: true,
                state_file: "doc-1.json".into(),
            }],
        };
        fs::write(old.join(MANIFEST), serde_json::to_vec(&manifest).unwrap()).unwrap();
        fs::write(old.join("doc-1.json"), b"{}").unwrap();

        let recovery = Recovery::start(root.path()).unwrap();
        assert!(recovery.restore(&manager()).is_empty());
        assert!(old.join("doc-1.json").exists());
    }
}
