use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use tauri::{AppHandle, Manager};
use vellum_core::jobs::JobLimits;
use vellum_core::settings::{LoadOutcome, SettingsChanged};
use vellum_core::{AppSettings, EventBus, JobQueue, SettingsStore};

use vellum_core::ui::UiReady;

use crate::bridge::TauriSink;

/// everything the ipc commands need, managed by tauri
pub struct AppState {
    pub bus: EventBus,
    pub settings: Arc<SettingsStore<AppSettings>>,
    pub jobs: JobQueue,
    pub settings_path: PathBuf,
}

impl AppState {
    pub fn init(app: &AppHandle) -> Result<Self, Box<dyn std::error::Error>> {
        let bus = EventBus::new();
        bus.add_sink(Arc::new(TauriSink::new(app.clone())));
        bus.accept_from_ui::<UiReady>();

        let settings_path = app.path().app_config_dir()?.join("settings.json");
        let (settings, outcome) =
            SettingsStore::<AppSettings>::open(&settings_path, Some(bus.clone()))?;
        log_outcome(&outcome);
        let settings = Arc::new(settings);

        let jobs = JobQueue::new(JobLimits::from(&settings.get().jobs), bus.clone());
        follow_job_limits(&bus, jobs.clone());
        log_ui_ready(&bus);

        Ok(Self {
            bus,
            settings,
            jobs,
            settings_path,
        })
    }
}

/// keeps the queue limits in step with the settings, however the settings get changed
fn follow_job_limits(bus: &EventBus, jobs: JobQueue) {
    let changes = bus.subscribe::<SettingsChanged<AppSettings>>();
    let spawned = thread::Builder::new()
        .name("vellum-settings-jobs".into())
        .spawn(move || {
            while let Some(change) = changes.recv() {
                jobs.set_limits(JobLimits::from(&change.settings.jobs));
            }
        });
    if let Err(err) = spawned {
        tracing::error!(%err, "job limits will not follow settings changes");
    }
}

fn log_ui_ready(bus: &EventBus) {
    let ready = bus.subscribe::<UiReady>();
    let spawned = thread::Builder::new()
        .name("vellum-ui-ready".into())
        .spawn(move || {
            if let Some(event) = ready.recv() {
                tracing::info!(version = %event.ui_version, "ui is ready");
            }
        });
    if let Err(err) = spawned {
        tracing::warn!(%err, "could not watch for the ui");
    }
}

fn log_outcome(outcome: &LoadOutcome) {
    match outcome {
        LoadOutcome::Fresh => tracing::info!("no settings file yet, using defaults"),
        LoadOutcome::Loaded => tracing::debug!("settings loaded"),
        LoadOutcome::Migrated { from, backup } => {
            tracing::info!(from, ?backup, "settings upgraded, old file backed up");
        }
        LoadOutcome::FromNewerVersion { version, backup } => {
            tracing::warn!(version, ?backup, "settings came from a newer vellum");
        }
        LoadOutcome::Recovered { moved_to, reason } => {
            tracing::warn!(?moved_to, %reason, "settings were unreadable and were reset");
        }
    }
}
