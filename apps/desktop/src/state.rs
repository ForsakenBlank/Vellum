use std::path::PathBuf;

use parking_lot::Mutex;
use vellum_core::jobs::QueueConfig;
use vellum_core::settings::LoadOutcome;
use vellum_core::{EventBus, JobQueue, SettingsStore, VellumEvent};

/// the long lived services shared by every tauri command.
pub struct AppState {
    pub bus: EventBus<VellumEvent>,
    pub settings: Mutex<SettingsStore>,
    pub jobs: JobQueue,
    /// set when settings could not be loaded cleanly, shown once by the ui
    pub settings_notice: Option<String>,
}

impl AppState {
    pub fn load(settings_path: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let bus = EventBus::new();
        let (store, outcome) = SettingsStore::load(settings_path)?;
        let settings_notice = match outcome {
            LoadOutcome::RecoveredFromCorrupt { backup, .. } => Some(format!(
                "Your settings file could not be read, so defaults are in use. The old file was kept at {}.",
                backup.display()
            )),
            LoadOutcome::NewerVersion { .. } => Some(
                "Your settings were saved by a newer version of Vellum. Changes made here will not be saved."
                    .to_string(),
            ),
            _ => None,
        };
        let jobs = JobQueue::new(QueueConfig::from_settings(&store.get().jobs), bus.clone());
        Ok(Self {
            settings: Mutex::new(store.with_events(bus.clone())),
            bus,
            jobs,
            settings_notice,
        })
    }
}
