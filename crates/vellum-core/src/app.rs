//! everything a running vellum needs, started and stopped in one place.
//!
//! [`AppServices`] owns the event bus, settings, recent files, job queue, documents,
//! notifications, autosave and the app command registry, plus the background threads that
//! keep them in step with the settings. the desktop shell creates one and forwards ipc calls
//! to it; tests and the future cli can do exactly the same without a window.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use ts_rs::TS;

use crate::actions::{self, AppContext};
use crate::command::{CommandError, CommandInfo, CommandRecord, CommandRegistry};
use crate::documents::{DocumentInfo, DocumentManager, DocumentMeta, PlaceholderDocument};
use crate::events::{Event, EventBus};
use crate::files::{FileKind, display_name};
use crate::history::HistoryLimits;
use crate::jobs::{JobLimits, JobQueue};
use crate::notifications::{NewNotification, NotificationCenter, NotificationLevel};
use crate::recent;
use crate::recent::{RecentFiles, RecentStore, now_ms};
use crate::recovery::Recovery;
use crate::settings::{AppSettings, LoadOutcome, SettingsChanged, SettingsError, SettingsStore};
use crate::shortcuts::Keymap;

/// where vellum keeps its files
#[derive(Debug, Clone)]
pub struct AppPaths {
    /// settings.json and recent.json
    pub config_dir: PathBuf,
    /// autosave and recovery folders
    pub data_dir: PathBuf,
}

/// what a command asks the ui to do after it ran
#[derive(Debug, Clone, PartialEq, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommandOutcome {
    /// ui actions to perform, like opening the palette
    pub ui_actions: Vec<CommandRecord>,
    /// a short note for the user, eg "Nothing to undo"
    pub message: Option<String>,
}

/// sent after every autosave attempt
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AutosaveStatus {
    #[ts(type = "number | null")]
    pub saved_at_ms: Option<u64>,
    pub error: Option<String>,
}

impl Event for AutosaveStatus {
    const NAME: &'static str = "autosave:changed";
}

/// a file that was not opened, and why
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SkippedFile {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OpenFilesResult {
    /// the documents now showing the files, including ones that were already open
    pub opened: Vec<DocumentInfo>,
    pub skipped: Vec<SkippedFile>,
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error(transparent)]
    Settings(#[from] SettingsError),
}

/// cheap to clone, every clone shares the same services
#[derive(Clone)]
pub struct AppServices {
    pub bus: EventBus,
    pub settings: Arc<SettingsStore<AppSettings>>,
    pub recent: Arc<RecentStore>,
    pub jobs: JobQueue,
    pub documents: DocumentManager,
    pub notifications: NotificationCenter,
    /// none if the recovery folder could not be created, autosave is off then
    pub recovery: Option<Arc<Recovery>>,
    pub commands: Arc<CommandRegistry<AppContext>>,
    pub paths: AppPaths,
    autosave: Option<Autosaver>,
    autosave_thread: Arc<Mutex<Option<AutosaveThread>>>,
}

/// how to stop the autosave thread, and its handle to wait for it
type AutosaveThread = (Sender<()>, JoinHandle<()>);

/// the parts autosave needs. the autosave thread holds only these, so dropping the
/// services releases the recovery lock once the thread stops.
#[derive(Clone)]
struct Autosaver {
    recovery: Arc<Recovery>,
    documents: DocumentManager,
    bus: EventBus,
    last: Arc<Mutex<AutosaveStatus>>,
}

impl Autosaver {
    fn run(&self) -> AutosaveStatus {
        let status = match self.recovery.autosave(&self.documents) {
            Ok(true) => AutosaveStatus {
                saved_at_ms: Some(now_ms()),
                error: None,
            },
            Ok(false) => return self.last.lock().clone(),
            Err(err) => {
                tracing::warn!(%err, "autosave failed");
                AutosaveStatus {
                    saved_at_ms: self.last.lock().saved_at_ms,
                    error: Some(format!("{err}. Vellum keeps trying.")),
                }
            }
        };
        *self.last.lock() = status.clone();
        self.bus.emit(status.clone());
        status
    }
}

impl AppServices {
    /// opens every store and starts the background threads. only an unusable settings
    /// folder stops start-up; everything else degrades and tells the user.
    pub fn start(paths: AppPaths) -> Result<Self, StartError> {
        let bus = EventBus::new();
        let (settings, outcome) = SettingsStore::<AppSettings>::open(
            paths.config_dir.join("settings.json"),
            Some(bus.clone()),
        )?;
        let (recent, _) =
            RecentStore::open(paths.config_dir.join("recent.json"), Some(bus.clone()))?;
        let current = settings.get();

        let notifications = NotificationCenter::new(bus.clone());
        notifications.follow_jobs();
        report_settings_outcome(&notifications, &outcome);

        let jobs = JobQueue::new(JobLimits::from(&current.jobs), bus.clone());
        let documents = DocumentManager::new(bus.clone(), HistoryLimits::from(&current.history));

        let recovery = match Recovery::start(&paths.data_dir.join("recovery")) {
            Ok(recovery) => Some(Arc::new(recovery)),
            Err(err) => {
                tracing::error!(%err, "autosave is off");
                notifications.push(
                    NewNotification::new(NotificationLevel::Error, "Autosave is off").body(
                        format!(
                            "{err}. Check that the folder can be written to, then restart Vellum."
                        ),
                    ),
                );
                None
            }
        };

        let commands = Arc::new(
            actions::registry()
                .map_err(|err| {
                    // a duplicate kind is a programming mistake caught by the tests
                    tracing::error!(%err, "app commands could not be registered");
                    err
                })
                .unwrap_or_default(),
        );

        let autosave = recovery.as_ref().map(|recovery| Autosaver {
            recovery: recovery.clone(),
            documents: documents.clone(),
            bus: bus.clone(),
            last: Arc::new(Mutex::new(AutosaveStatus {
                saved_at_ms: None,
                error: None,
            })),
        });
        let services = Self {
            bus,
            settings: Arc::new(settings),
            recent: Arc::new(recent),
            jobs,
            documents,
            notifications,
            recovery,
            commands,
            paths,
            autosave,
            autosave_thread: Arc::new(Mutex::new(None)),
        };
        services.follow_settings();
        services.start_autosave();
        Ok(services)
    }

    /// stops autosave, cancels jobs and writes the last autosave. call once on exit.
    pub fn shutdown(&self) {
        self.stop_autosave();
        self.jobs.cancel_all();
        if let Some(recovery) = &self.recovery
            && let Err(err) = recovery.finish(&self.documents)
        {
            tracing::error!(%err, "the last autosave failed");
        }
    }

    pub fn command_infos(&self) -> Vec<CommandInfo> {
        self.commands.infos().cloned().collect()
    }

    /// runs an app command. `document` is the document it is about, usually the active tab.
    pub fn run_command(
        &self,
        record: &CommandRecord,
        document: Option<String>,
    ) -> Result<CommandOutcome, CommandError> {
        let mut command = self.commands.instantiate(record)?;
        let mut context = AppContext {
            services: self.clone(),
            document,
            outcome: CommandOutcome::default(),
        };
        command.apply(&mut context)?;
        Ok(context.outcome)
    }

    /// opens files in the workspace that fits each one, or all of them in one quick tool.
    /// a file that is already open is not opened twice. every opened file goes to the top
    /// of the recent list.
    pub fn open_files(
        &self,
        paths: &[PathBuf],
        tool: Option<&str>,
        window: &str,
    ) -> OpenFilesResult {
        let mut result = OpenFilesResult::default();
        if let Some(tool) = tool {
            let title = self.tool_title(tool);
            let title = match paths {
                [one] => format!("{title}: {}", display_name(one)),
                _ => title,
            };
            let mut meta = DocumentMeta::for_tool(tool, paths.to_vec());
            meta.window = window.to_owned();
            match self
                .documents
                .open(PlaceholderDocument::titled(title), meta)
            {
                Ok(info) => result.opened.push(info),
                Err(err) => result.skipped.extend(paths.iter().map(|p| SkippedFile {
                    path: p.display().to_string(),
                    reason: err.to_string(),
                })),
            }
            self.remember(paths);
            return result;
        }

        let open = self.documents.list();
        for path in paths {
            let shown = path.display().to_string();
            let Some(kind) = FileKind::of_path(path) else {
                let ext = path
                    .extension()
                    .map(|e| format!(".{} files", e.to_string_lossy()))
                    .unwrap_or_else(|| "files without an extension".into());
                result.skipped.push(SkippedFile {
                    path: shown,
                    reason: format!("Vellum cannot open {ext} yet."),
                });
                continue;
            };
            if let Some(existing) = open
                .iter()
                .find(|d| d.tool.is_none() && d.path.as_deref() == Some(shown.as_str()))
            {
                result.opened.push(existing.clone());
                continue;
            }
            let mut meta = DocumentMeta::for_file(path.clone(), kind);
            meta.window = window.to_owned();
            let doc = PlaceholderDocument::titled(display_name(path));
            match self.documents.open(doc, meta) {
                Ok(info) => result.opened.push(info),
                Err(err) => result.skipped.push(SkippedFile {
                    path: shown,
                    reason: err.to_string(),
                }),
            }
        }
        self.remember(paths);
        result
    }

    fn remember(&self, paths: &[PathBuf]) {
        for path in paths.iter().filter(|p| FileKind::of_path(p).is_some()) {
            if let Err(err) = recent::record_open(&self.recent, path) {
                tracing::warn!(%err, "could not update recent files");
            }
        }
    }

    /// "Video to GIF" for `video_to_gif`, from the tool's command
    pub fn tool_title(&self, tool: &str) -> String {
        self.commands
            .info(&format!("tool.{tool}"))
            .map_or_else(|| tool.replace('_', " "), |info| info.title.to_owned())
    }

    /// true if a job produced this file, so the ui may ask to open it
    pub fn is_job_result(&self, path: &str) -> bool {
        self.jobs
            .list()
            .iter()
            .any(|job| crate::notifications::result_path(job).as_deref() == Some(path))
    }

    pub fn keymap(&self) -> Keymap {
        self.settings.read(|s| Keymap::from_settings(&s.shortcuts))
    }

    pub fn recent_files(&self) -> RecentFiles {
        self.recent.get()
    }

    pub fn autosave_status(&self) -> AutosaveStatus {
        match &self.autosave {
            Some(autosave) => autosave.last.lock().clone(),
            None => AutosaveStatus {
                saved_at_ms: None,
                error: Some("Autosave is off because its folder could not be created.".into()),
            },
        }
    }

    /// saves now if anything changed, and reports the result
    pub fn autosave_now(&self) -> AutosaveStatus {
        match &self.autosave {
            Some(autosave) => autosave.run(),
            None => self.autosave_status(),
        }
    }

    /// keeps job limits, history limits and the keymap in step with the settings. the
    /// thread lives as long as the process, so it holds only what it needs.
    fn follow_settings(&self) {
        let changes = self.bus.subscribe::<SettingsChanged<AppSettings>>();
        let jobs = self.jobs.clone();
        let documents = self.documents.clone();
        let bus = self.bus.clone();
        let mut shortcuts = self.settings.read(|s| s.shortcuts.clone());
        let spawned = thread::Builder::new()
            .name("vellum-settings".into())
            .spawn(move || {
                while let Some(change) = changes.recv() {
                    let settings = change.settings;
                    jobs.set_limits(JobLimits::from(&settings.jobs));
                    documents.set_limits(HistoryLimits::from(&settings.history));
                    if settings.shortcuts != shortcuts {
                        shortcuts = settings.shortcuts.clone();
                        bus.emit(Keymap::from_settings(&shortcuts));
                    }
                }
            });
        if let Err(err) = spawned {
            tracing::error!(%err, "settings changes will only apply after a restart");
        }
    }

    fn start_autosave(&self) {
        let Some(autosave) = self.autosave.clone() else {
            return;
        };
        let settings = self.settings.clone();
        let (stop, stopped) = mpsc::channel::<()>();
        let spawned = thread::Builder::new()
            .name("vellum-autosave".into())
            .spawn(move || {
                loop {
                    let (enabled, secs) =
                        settings.read(|s| (s.autosave.enabled, s.autosave.interval_secs));
                    match stopped.recv_timeout(Duration::from_secs(u64::from(secs))) {
                        Err(RecvTimeoutError::Timeout) if enabled => {
                            autosave.run();
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            });
        match spawned {
            Ok(handle) => *self.autosave_thread.lock() = Some((stop, handle)),
            Err(err) => tracing::error!(%err, "autosave could not start"),
        }
    }

    fn stop_autosave(&self) {
        if let Some((stop, handle)) = self.autosave_thread.lock().take() {
            let _ = stop.send(());
            if handle.join().is_err() {
                tracing::error!("the autosave thread panicked");
            }
        }
    }
}

fn report_settings_outcome(notifications: &NotificationCenter, outcome: &LoadOutcome) {
    let note = |path: &Path| path.display().to_string();
    let new = match outcome {
        LoadOutcome::Fresh | LoadOutcome::Loaded | LoadOutcome::Migrated { .. } => return,
        LoadOutcome::FromNewerVersion { backup, .. } => {
            NewNotification::new(NotificationLevel::Warning, "Settings came from a newer Vellum")
                .body(format!(
                    "Settings Vellum does not know yet were kept in {}.",
                    note(backup)
                ))
        }
        LoadOutcome::Recovered { moved_to, .. } => {
            NewNotification::new(NotificationLevel::Warning, "Settings were reset")
                .body(format!(
                    "The settings file could not be read, so defaults are in use. The old file was kept as {}.",
                    note(moved_to)
                ))
        }
    };
    notifications.push(new);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documents::{DocumentMeta, EditNotes, PlaceholderDocument};
    use crate::files::FileKind;
    use crate::settings::Theme;
    use serde_json::json;

    fn start(dir: &Path) -> AppServices {
        AppServices::start(AppPaths {
            config_dir: dir.join("config"),
            data_dir: dir.join("data"),
        })
        .unwrap()
    }

    fn open_clip(services: &AppServices) -> String {
        let path = PathBuf::from("/m/clip.mp4");
        services
            .documents
            .open(
                PlaceholderDocument::default(),
                DocumentMeta::for_file(path, FileKind::Video),
            )
            .unwrap()
            .id
    }

    fn wait_until(mut check: impl FnMut() -> bool) {
        for _ in 0..200 {
            if check() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("condition never became true");
    }

    #[test]
    fn starts_with_commands_and_a_keymap() {
        let dir = tempfile::tempdir().unwrap();
        let services = start(dir.path());
        assert!(services.command_infos().len() > 20);
        assert_eq!(services.keymap().bindings["edit.undo"], ["Ctrl+Z"]);
        assert!(services.recovery.is_some());
        services.shutdown();
    }

    #[test]
    fn settings_changes_reach_documents_and_the_keymap() {
        let dir = tempfile::tempdir().unwrap();
        let services = start(dir.path());
        let keymaps = services.bus.subscribe::<Keymap>();
        services
            .settings
            .apply_patch(&json!({
                "history": { "maxSteps": 20 },
                "shortcuts": { "preset": "photoshop" }
            }))
            .unwrap();
        wait_until(|| services.documents.limits().max_steps == 20);
        let keymap = keymaps.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(keymap.bindings["app.settings"], ["Ctrl+K"]);

        // a change that leaves shortcuts alone sends no keymap
        services
            .settings
            .apply_patch(&json!({ "appearance": { "theme": "light" } }))
            .unwrap();
        assert!(keymaps.recv_timeout(Duration::from_millis(200)).is_none());
        services.shutdown();
    }

    #[test]
    fn autosave_reports_and_shutdown_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let services = start(dir.path());
        let statuses = services.bus.subscribe::<AutosaveStatus>();
        let id = open_clip(&services);
        let status = services.autosave_now();
        assert!(status.saved_at_ms.is_some());
        assert!(statuses.try_recv().is_some());
        // nothing changed, nothing written, nothing sent
        services.autosave_now();
        assert!(statuses.try_recv().is_none());

        services
            .documents
            .execute(
                &id,
                &CommandRecord::new(EditNotes::KIND, json!({ "notes": "x" })),
            )
            .unwrap();
        services.documents.mark_saved(&id).unwrap();
        let session = services.recovery.as_ref().unwrap().session_dir().to_owned();
        services.shutdown();
        assert!(!session.exists());
    }

    #[test]
    fn a_crashed_session_is_offered_on_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        {
            let services = start(dir.path());
            let id = open_clip(&services);
            services
                .documents
                .execute(
                    &id,
                    &CommandRecord::new(EditNotes::KIND, json!({ "notes": "x" })),
                )
                .unwrap();
            services.autosave_now();
            // no shutdown: this is the crash. the recovery lock goes when the last clone of
            // it drops, so stop the autosave thread that holds one.
            services.stop_autosave();
        }
        let services = start(dir.path());
        let offer = services.recovery.as_ref().unwrap().offer().unwrap();
        assert_eq!(offer.documents.len(), 1);
        services.shutdown();
    }

    #[test]
    fn opening_files_routes_dedupes_and_remembers() {
        let dir = tempfile::tempdir().unwrap();
        let services = start(dir.path());
        let paths = [
            PathBuf::from("/m/clip.mp4"),
            PathBuf::from("/m/notes.txt"),
            PathBuf::from("/m/README"),
        ];
        let result = services.open_files(&paths, None, "main");
        assert_eq!(result.opened.len(), 1);
        assert_eq!(result.opened[0].title, "clip.mp4");
        assert_eq!(
            result.skipped[0].reason,
            "Vellum cannot open .txt files yet."
        );
        assert_eq!(
            result.skipped[1].reason,
            "Vellum cannot open files without an extension yet."
        );

        let again = services.open_files(&paths[..1], None, "main");
        assert_eq!(again.opened[0].id, result.opened[0].id);
        assert_eq!(services.documents.list().len(), 1);
        assert_eq!(services.recent_files().files[0].name, "clip.mp4");
        services.shutdown();
    }

    #[test]
    fn opening_files_in_a_tool_makes_one_session() {
        let dir = tempfile::tempdir().unwrap();
        let services = start(dir.path());
        let result = services.open_files(
            &[PathBuf::from("/m/clip.mp4")],
            Some("video_to_gif"),
            "doc-2",
        );
        let doc = &result.opened[0];
        assert_eq!(doc.title, "Video to GIF: clip.mp4");
        assert_eq!(doc.tool.as_deref(), Some("video_to_gif"));
        assert_eq!(doc.window, "doc-2");
        assert_eq!(doc.inputs, ["/m/clip.mp4"]);

        let empty = services.open_files(&[], Some("screen_record"), "main");
        assert_eq!(empty.opened[0].title, "Screen record");
        services.shutdown();
    }

    #[test]
    fn broken_settings_are_reported_as_a_notification() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::write(dir.path().join("config/settings.json"), b"{ broken").unwrap();
        let services = start(dir.path());
        let list = services.notifications.list();
        assert_eq!(list.items[0].title, "Settings were reset");
        assert_eq!(services.settings.get().appearance.theme, Theme::Dark);
        services.shutdown();
    }
}
