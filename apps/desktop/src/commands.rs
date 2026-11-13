//! the commands the ui can call. each one is a thin wrapper: check the input,
//! call into a service, turn errors into something the ui can show.
//!
//! all of them are `async` in the tauri sense, which only means they run off
//! the main thread so a slow disk can never freeze the window.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, State};
use ts_rs::TS;
use vellum_core::jobs::{JobContext, JobError, JobId, JobSpec, JobsOverview, Priority};

use crate::error::IpcError;
use crate::state::AppState;

type Ipc<T> = Result<T, IpcError>;

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    /// `windows`, `macos` or `linux`
    pub os: String,
    pub arch: String,
    pub debug_build: bool,
}

#[tauri::command(async)]
pub fn app_info(app: AppHandle) -> AppInfo {
    let package = app.package_info();
    AppInfo {
        name: package.name.clone(),
        version: package.version.to_string(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        debug_build: cfg!(debug_assertions),
    }
}

/// the ui calls this once it has loaded its state and drawn its first frame
#[tauri::command(async)]
pub fn ui_ready(app: AppHandle, state: State<'_, AppState>) {
    tracing::info!(
        startup_ms = state.started.elapsed().as_millis() as u64,
        "ui ready"
    );
    if state.smoke_test {
        app.exit(0);
    }
}

#[tauri::command(async)]
pub fn settings_all(state: State<'_, AppState>) -> BTreeMap<String, Value> {
    state.settings.all()
}

#[tauri::command(async)]
pub fn settings_set(state: State<'_, AppState>, section: String, value: Value) -> Ipc<Value> {
    Ok(state.settings.set_raw(&section, value)?)
}

#[tauri::command(async)]
pub fn jobs_overview(state: State<'_, AppState>) -> JobsOverview {
    state.jobs.overview()
}

#[tauri::command(async)]
pub fn job_pause(state: State<'_, AppState>, id: JobId) -> Ipc<()> {
    Ok(state.jobs.pause(id)?)
}

#[tauri::command(async)]
pub fn job_resume(state: State<'_, AppState>, id: JobId) -> Ipc<()> {
    Ok(state.jobs.resume(id)?)
}

#[tauri::command(async)]
pub fn job_cancel(state: State<'_, AppState>, id: JobId) -> Ipc<()> {
    Ok(state.jobs.cancel(id)?)
}

#[tauri::command(async)]
pub fn job_retry(state: State<'_, AppState>, id: JobId) -> Ipc<()> {
    Ok(state.jobs.retry(id)?)
}

#[tauri::command(async)]
pub fn job_remove(state: State<'_, AppState>, id: JobId) -> Ipc<()> {
    Ok(state.jobs.remove(id)?)
}

#[tauri::command(async)]
pub fn job_set_priority(state: State<'_, AppState>, id: JobId, priority: Priority) -> Ipc<()> {
    Ok(state.jobs.set_priority(id, priority)?)
}

#[tauri::command(async)]
pub fn job_move(state: State<'_, AppState>, id: JobId, index: usize) -> Ipc<()> {
    Ok(state.jobs.move_to(id, index)?)
}

#[tauri::command(async)]
pub fn jobs_clear_finished(state: State<'_, AppState>) {
    state.jobs.clear_finished();
}

/// a pretend job for checking the queue by hand until real ones exist.
/// it counts to `seconds` and then succeeds, or fails if asked to
#[tauri::command(async)]
pub fn dev_sample_job(state: State<'_, AppState>, seconds: f32, fail: bool) -> JobId {
    let seconds = if seconds.is_finite() {
        seconds.clamp(0.1, 600.0)
    } else {
        5.0
    };
    let name = if fail {
        "Sample job that fails"
    } else {
        "Sample job"
    };
    let spec = JobSpec::cpu(name).category("sample");

    let handle = state.jobs.submit(spec, move |ctx: &JobContext| {
        let steps = (seconds * 20.0).ceil() as u32;
        for step in 0..steps {
            ctx.sleep(Duration::from_millis(50))?;
            ctx.progress((step + 1) as f32 / steps as f32);
            if fail && step >= steps / 2 {
                return Err(JobError::failed(
                    "This sample job is meant to fail half way. Press retry to run it again.",
                ));
            }
        }
        Ok(json!({ "seconds": seconds }))
    });
    handle.id()
}
