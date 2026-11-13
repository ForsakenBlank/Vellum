//! the autosave timer. every few seconds it checks whether the interval from
//! the general settings has passed and the session changed since the last
//! save, and writes it if so.
//!
//! this is a plain thread rather than a job: a save of the session is a few
//! kilobytes, and a job every thirty seconds would push real jobs out of the
//! list of finished ones. when documents with real content arrive, their saves
//! will be jobs started from here.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use vellum_core::settings::SettingsStore;
use vellum_core::shell::autosave::SessionFiles;
use vellum_core::shell::session::Shell;
use vellum_core::shell::settings::GeneralSettings;

const TICK: Duration = Duration::from_millis(500);

pub struct Autosave {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Autosave {
    pub fn start(
        shell: Arc<Shell>,
        files: Arc<SessionFiles>,
        settings: SettingsStore,
    ) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = thread::Builder::new()
            .name("vellum-autosave".into())
            .spawn(move || {
                let mut saved_revision = shell.revision();
                let mut last_save = Instant::now();
                while !stopped.load(Ordering::Acquire) {
                    thread::sleep(TICK);
                    let seconds = settings.get::<GeneralSettings>().autosave_seconds;
                    if seconds == 0 || last_save.elapsed() < Duration::from_secs(seconds.into()) {
                        continue;
                    }
                    let revision = shell.revision();
                    if revision == saved_revision {
                        continue;
                    }
                    match files.save(&shell.snapshot()) {
                        Ok(()) => saved_revision = revision,
                        Err(error) => tracing::warn!(%error, "autosave failed, will try again"),
                    }
                    last_save = Instant::now();
                }
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Autosave {
    fn drop(&mut self) {
        self.stop();
    }
}
