//! autosave and crash recovery for the session.
//!
//! while the app runs, a marker file says so. a clean exit removes it along
//! with the autosave. if the marker is still there on the next start, the last
//! run did not exit cleanly, and whatever it autosaved is offered back.
//!
//! ```text
//! <dir>/running             the marker
//! <dir>/session.json        autosave of the current run
//! <dir>/session.recovered.json   what a crashed run left, until the user decides
//! ```

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::session::{OpenDocument, SessionSnapshot};
use crate::fs::{remove_if_present, write_atomically};
use crate::time::unix_ms;

const MARKER: &str = "running";
const AUTOSAVE: &str = "session.json";
const RECOVERED: &str = "session.recovered.json";

/// what the ui needs to ask "restore your session?"
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecoveryOffer {
    /// when the crashed session was last autosaved
    pub saved_ms: u64,
    pub documents: Vec<OpenDocument>,
}

pub struct SessionFiles {
    dir: PathBuf,
}

impl SessionFiles {
    /// marks the start of a run. if the last run never finished, its autosave is
    /// kept aside for recovery. a recovery the user never answered is still
    /// offered, so a second crash before deciding loses nothing
    pub fn begin(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let files = Self { dir: dir.into() };
        fs::create_dir_all(&files.dir)?;

        let crashed = files.path(MARKER).exists();
        if crashed {
            match read_snapshot(&files.path(AUTOSAVE)) {
                Ok(Some(snapshot)) if !snapshot.documents.is_empty() => {
                    fs::rename(files.path(AUTOSAVE), files.path(RECOVERED))?;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%error, "the autosave from the last run could not be read");
                }
            }
        }
        remove_if_present(&files.path(AUTOSAVE))?;
        write_atomically(&files.path(MARKER), unix_ms().to_string().as_bytes())?;
        Ok(files)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    pub fn save(&self, snapshot: &SessionSnapshot) -> io::Result<()> {
        let json = serde_json::to_vec(snapshot).map_err(io::Error::other)?;
        write_atomically(&self.path(AUTOSAVE), &json)
    }

    /// what a crashed run left behind, if the user has not decided about it yet
    pub fn recovery(&self) -> Option<SessionSnapshot> {
        match read_snapshot(&self.path(RECOVERED)) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(%error, "the recovered session could not be read");
                None
            }
        }
    }

    pub fn offer(&self) -> Option<RecoveryOffer> {
        self.recovery().map(|snapshot| RecoveryOffer {
            saved_ms: snapshot.saved_ms,
            documents: snapshot.documents,
        })
    }

    /// the user answered the recovery prompt, either way
    pub fn forget_recovery(&self) -> io::Result<()> {
        remove_if_present(&self.path(RECOVERED))
    }

    /// a clean exit. nothing will be offered next time
    pub fn finish(&self) -> io::Result<()> {
        remove_if_present(&self.path(AUTOSAVE))?;
        remove_if_present(&self.path(MARKER))
    }
}

fn read_snapshot(path: &Path) -> io::Result<Option<SessionSnapshot>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::shell::files::{OpenTarget, plan_open};
    use crate::shell::session::{MAIN_WINDOW, Session};

    fn session_with(paths: &[&str]) -> Session {
        let mut session = Session::new();
        let paths: Vec<String> = paths.iter().map(|p| (*p).to_owned()).collect();
        let plan = plan_open(&paths, &OpenTarget::Auto);
        session.open(&plan.opens, MAIN_WINDOW).unwrap();
        session
    }

    #[test]
    fn a_clean_exit_offers_nothing_next_time() {
        let dir = TempDir::new().unwrap();
        let files = SessionFiles::begin(dir.path()).unwrap();
        files.save(&session_with(&["/a.png"]).snapshot()).unwrap();
        files.finish().unwrap();

        let next = SessionFiles::begin(dir.path()).unwrap();
        assert!(next.offer().is_none());
    }

    #[test]
    fn a_crash_offers_the_last_autosave() {
        let dir = TempDir::new().unwrap();
        let files = SessionFiles::begin(dir.path()).unwrap();
        files
            .save(&session_with(&["/a.png", "/b.mp4"]).snapshot())
            .unwrap();
        drop(files); // no finish: the app died

        let next = SessionFiles::begin(dir.path()).unwrap();
        let offer = next.offer().unwrap();
        let names: Vec<_> = offer.documents.iter().map(|d| d.title.as_str()).collect();
        assert_eq!(names, ["a.png", "b.mp4"]);

        // autosaving the new run does not touch what is on offer
        next.save(&Session::new().snapshot()).unwrap();
        assert!(next.offer().is_some());
        next.forget_recovery().unwrap();
        assert!(next.offer().is_none());
    }

    #[test]
    fn a_crash_with_nothing_open_offers_nothing() {
        let dir = TempDir::new().unwrap();
        let files = SessionFiles::begin(dir.path()).unwrap();
        files.save(&Session::new().snapshot()).unwrap();
        drop(files);

        assert!(SessionFiles::begin(dir.path()).unwrap().offer().is_none());
    }

    #[test]
    fn an_unanswered_offer_survives_a_second_crash() {
        let dir = TempDir::new().unwrap();
        let files = SessionFiles::begin(dir.path()).unwrap();
        files.save(&session_with(&["/a.png"]).snapshot()).unwrap();
        drop(files);

        // the second run crashes before anything was open and before the user answered
        let second = SessionFiles::begin(dir.path()).unwrap();
        second.save(&Session::new().snapshot()).unwrap();
        drop(second);

        let third = SessionFiles::begin(dir.path()).unwrap();
        assert_eq!(third.offer().unwrap().documents[0].title, "a.png");
    }

    #[test]
    fn a_garbled_autosave_is_skipped_not_fatal() {
        let dir = TempDir::new().unwrap();
        let files = SessionFiles::begin(dir.path()).unwrap();
        fs::write(dir.path().join(AUTOSAVE), b"{ not json").unwrap();
        drop(files);

        let next = SessionFiles::begin(dir.path()).unwrap();
        assert!(next.offer().is_none());
    }
}
