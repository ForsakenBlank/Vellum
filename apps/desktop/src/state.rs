use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};
use vellum_core::event::{EventBus, Subscription};
use vellum_core::jobs::{JobLimits, JobQueue, JobQueueConfig};
use vellum_core::settings::{PerformanceSettings, SettingsChanged, SettingsSection, SettingsStore};

use crate::EVENT_CHANNEL;
use crate::bindings::register_settings;

/// set by ci to check that the window opens and the ui comes up, then quit
const SMOKE_TEST_ENV: &str = "VELLUM_SMOKE_TEST";
const SMOKE_TEST_TIMEOUT: Duration = Duration::from_secs(90);

/// the services that live as long as the app does
pub struct AppState {
    pub settings: SettingsStore,
    pub jobs: JobQueue,
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
            started,
            smoke_test,
            _subscriptions: vec![forwarder, limits_follow_settings],
        })
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
