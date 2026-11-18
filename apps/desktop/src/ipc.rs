//! commands the ui can invoke. they stay thin: parse, call into the core, map errors.

use std::thread;
use std::time::Duration;

use serde::{Serialize, Serializer};
use tauri::State;
use vellum_core::AppSettings;
use vellum_core::jobs::{
    JobContext, JobError, JobId, JobPriority, JobQueueError, JobResource, JobSpec, JobsOverview,
};
use vellum_core::settings::SettingsError;
use vellum_core::ui::AppInfo;

use crate::state::AppState;

/// errors reach the ui as a plain message
#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error(transparent)]
    Jobs(#[from] JobQueueError),
    #[error(transparent)]
    Event(#[from] vellum_core::events::EventError),
    #[error("{0}")]
    Invalid(String),
}

impl Serialize for IpcError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

type IpcResult<T> = Result<T, IpcError>;

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        settings_path: state.settings_path.display().to_string(),
        os: std::env::consts::OS.to_owned(),
    }
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> AppSettings {
    state.settings.get()
}

/// takes a json merge patch, eg `{ "appearance": { "theme": "light" } }`
#[tauri::command]
pub fn update_settings(
    state: State<'_, AppState>,
    patch: serde_json::Value,
) -> IpcResult<AppSettings> {
    Ok(state.settings.apply_patch(&patch)?)
}

#[tauri::command]
pub fn reset_settings(state: State<'_, AppState>) -> IpcResult<AppSettings> {
    Ok(state.settings.reset()?)
}

#[tauri::command]
pub fn list_jobs(state: State<'_, AppState>) -> JobsOverview {
    state.jobs.overview()
}

#[tauri::command]
pub fn cancel_job(state: State<'_, AppState>, id: JobId) -> IpcResult<()> {
    Ok(state.jobs.cancel(id)?)
}

#[tauri::command]
pub fn pause_job(state: State<'_, AppState>, id: JobId) -> IpcResult<()> {
    Ok(state.jobs.pause(id)?)
}

#[tauri::command]
pub fn resume_job(state: State<'_, AppState>, id: JobId) -> IpcResult<()> {
    Ok(state.jobs.resume(id)?)
}

#[tauri::command]
pub fn retry_job(state: State<'_, AppState>, id: JobId) -> IpcResult<()> {
    Ok(state.jobs.retry(id)?)
}

#[tauri::command]
pub fn remove_job(state: State<'_, AppState>, id: JobId) -> IpcResult<()> {
    Ok(state.jobs.remove(id)?)
}

#[tauri::command]
pub fn clear_finished_jobs(state: State<'_, AppState>) {
    state.jobs.clear_finished();
}

#[tauri::command]
pub fn set_job_priority(
    state: State<'_, AppState>,
    id: JobId,
    priority: JobPriority,
) -> IpcResult<()> {
    Ok(state.jobs.set_priority(id, priority)?)
}

#[tauri::command]
pub fn move_job(state: State<'_, AppState>, id: JobId, before: Option<JobId>) -> IpcResult<()> {
    Ok(state.jobs.move_before(id, before)?)
}

/// a job that does nothing but tick for a while. it lets you try the queue by hand until
/// real jobs arrive in phase 2. `fail_first` makes the first attempt fail to show retries.
#[tauri::command]
pub fn start_sample_job(
    state: State<'_, AppState>,
    seconds: u32,
    resource: JobResource,
    fail_first: bool,
) -> IpcResult<JobId> {
    if !(1..=600).contains(&seconds) {
        return Err(IpcError::Invalid(
            "a sample job runs for 1 to 600 seconds".into(),
        ));
    }
    let steps = u64::from(seconds) * 20;
    let spec = JobSpec::new(format!("Sample job ({seconds} s)"), resource)
        .with_retries(u32::from(fail_first), Duration::from_secs(1));
    let id = state
        .jobs
        .submit(spec, move |ctx: &JobContext| -> Result<_, JobError> {
            for step in 0..steps {
                ctx.checkpoint()?;
                ctx.set_progress_steps(step, steps);
                if fail_first && ctx.attempt() == 0 && step == steps / 2 {
                    return Err(JobError::failed("the sample job failed on purpose"));
                }
                thread::sleep(Duration::from_millis(50));
            }
            Ok(serde_json::json!({ "steps": steps }))
        });
    Ok(id)
}

/// the way the ui sends events to rust. only events registered with
/// `EventBus::accept_from_ui` get through.
#[tauri::command]
pub fn ui_event(
    state: State<'_, AppState>,
    name: String,
    payload: serde_json::Value,
) -> IpcResult<()> {
    Ok(state.bus.dispatch_from_ui(&name, payload)?)
}
