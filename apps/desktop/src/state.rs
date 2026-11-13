use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use vellum_core::event::{EventBus, Subscription};
use vellum_core::jobs::{JobLimits, JobQueue, JobQueueConfig};
use vellum_core::settings::{PerformanceSettings, SettingsChanged, SettingsSection, SettingsStore};
use vellum_core::shell::autosave::SessionFiles;
use vellum_core::shell::recent::RecentFiles;
use vellum_core::shell::session::Shell;
use vellum_core::shell::settings::GeneralSettings;

use crate::EVENT_CHANNEL;
use crate::autosave::Autosave;
use crate::bindings::register_settings;

/// set by ci to check that the window opens and the ui comes up, then quit
const SMOKE_TEST_ENV: &str = "VELLUM_SMOKE_TEST";
const SMOKE_TEST_TIMEOUT: Duration = Duration::from_secs(90);

/// the services that live as long as the app does
pub struct AppState {
    pub settings: SettingsStore,
    pub jobs: JobQueue,
    pub shell: Arc<Shell>,
    pub recent: RecentFiles,
    /// none when the data folder could not be used, autosave is off then
    pub session_files: Option<Arc<SessionFiles>>,
    autosave: Mutex<Option<Autosave>>,
    pub started: Instant,
    pub smoke_test: bool,
    _subscriptions: Vec<Subscription>,
}

impl AppState {
    pub fn start(app: &AppHandle) -> Result<Self, Box<dyn std::error::Error>> {
        let started = Instant::now();
        let bus = EventBus::new();

        let forward_to = app.clone();
        let forwarder = bus.tap(move |envelope| {
            if let Err(error) = forward_to.emit(EVENT_CHANNEL, envelope) {
                tracing::warn!(event = envelope.name, %error, "could not send an event to the ui");
            }
        });

        let settings = open_settings(app, &bus);
        register_settings(&settings);

        let limits = job_limits(&settings.get::<PerformanceSettings>());
        let jobs = JobQueue::new(
            bus.clone(),
            JobQueueConfig {
                limits,
                ..Default::default()
            },
        )?;

        let limits_follow_settings = bus.subscribe({
            let (settings, jobs) = (settings.clone(), jobs.clone());
            move |changed: &SettingsChanged| {
                if changed.section == PerformanceSettings::KEY {
                    jobs.set_limits(job_limits(&settings.get::<PerformanceSettings>()));
                }
            }
        });

        let data_dir = app.path().app_data_dir().ok();
        let shell = Arc::new(Shell::new(Some(bus.clone())));
        let recent = match &data_dir {
            Some(dir) => RecentFiles::open(dir.join("recent.json"), Some(bus.clone())),
            None => RecentFiles::in_memory(Some(bus.clone())),
        };
        recent.set_limit(settings.get::<GeneralSettings>().recent_limit as usize);

        let session_files = open_session_files(data_dir);
        let autosave = session_files.as_ref().and_then(|files| {
            Autosave::start(shell.clone(), files.clone(), settings.clone())
                .map_err(|error| tracing::error!(%error, "autosave could not start"))
                .ok()
        });

        let recent_follows_settings = bus.subscribe({
            let (settings, recent) = (settings.clone(), recent.clone());
            move |changed: &SettingsChanged| {
                if changed.section == GeneralSettings::KEY {
                    recent.set_limit(settings.get::<GeneralSettings>().recent_limit as usize);
                }
            }
        });

        let smoke_test = std::env::var_os(SMOKE_TEST_ENV).is_some();
        if smoke_test {
            std::thread::spawn(|| {
                std::thread::sleep(SMOKE_TEST_TIMEOUT);
                eprintln!("smoke test: the ui never reported ready");
                std::process::exit(1);
            });
        }

        Ok(Self {
            settings,
            jobs,
            shell,
            recent,
            session_files,
            autosave: Mutex::new(autosave),
            started,
            smoke_test,
            _subscriptions: vec![forwarder, limits_follow_settings, recent_follows_settings],
        })
    }

    /// a clean exit: stop the background work and tell the next run not to
    /// offer a recovery
    pub fn shut_down(&self) {
        self.jobs.shut_down();
        if let Some(mut autosave) = self.autosave.lock().take() {
            autosave.stop();
        }
        if let Some(files) = &self.session_files
            && let Err(error) = files.finish()
        {
            tracing::warn!(%error, "could not mark the session as closed cleanly");
        }
    }
}

fn open_session_files(data_dir: Option<PathBuf>) -> Option<Arc<SessionFiles>> {
    let dir = data_dir?.join("session");
    match SessionFiles::begin(&dir) {
        Ok(files) => Some(Arc::new(files)),
        Err(error) => {
            tracing::error!(dir = %dir.display(), %error, "autosave and crash recovery are off this session");
            None
        }
    }
}

/// settings that cannot be loaded must not stop the app from opening,
/// so the fallback is a store that lives in memory for this run
fn open_settings(app: &AppHandle, bus: &EventBus) -> SettingsStore {
    let opened = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())
        .and_then(|dir| {
            SettingsStore::open(dir.join("settings.json"), Some(bus.clone()))
                .map_err(|error| error.to_string())
        });
    match opened {
        Ok(store) => {
            if let Some(aside) = store.recovered_file() {
                tracing::warn!(moved_to = %aside.display(), "settings were reset, the old file was kept");
            }
            store
        }
        Err(error) => {
            tracing::error!(%error, "settings will not be saved this session");
            SettingsStore::in_memory(Some(bus.clone()))
        }
    }
}

fn job_limits(performance: &PerformanceSettings) -> JobLimits {
    let defaults = JobLimits::default();
    JobLimits {
        cpu: match performance.max_cpu_jobs {
            0 => defaults.cpu,
            chosen => chosen as usize,
        },
        gpu: performance.max_gpu_jobs.max(1) as usize,
        io: defaults.io,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_cpu_jobs_means_pick_for_me() {
        let auto = job_limits(&PerformanceSettings::default());
        assert_eq!(auto.cpu, JobLimits::default_cpu_jobs());
        assert_eq!(auto.gpu, 1);

        let chosen = PerformanceSettings {
            max_cpu_jobs: 3,
            max_gpu_jobs: 2,
            ..Default::default()
        };
        let limits = job_limits(&chosen);
        assert_eq!((limits.cpu, limits.gpu), (3, 2));
    }
}
