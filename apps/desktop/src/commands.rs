//! tauri commands. errors cross into the ui as plain strings that are safe to show the user.

use std::time::Duration;

use serde::Serialize;
use tauri::State;
use vellum_core::jobs::{JobContext, JobKind, JobSpec};
use vellum_core::{JobId, JobInfo, Settings};

use crate::state::AppState;

type CmdResult<T> = Result<T, String>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: &'static str,
    settings_path: String,
    settings_notice: Option<String>,
    settings_read_only: bool,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    let settings = state.settings.lock();
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        settings_path: settings.path().display().to_string(),
        settings_notice: state.settings_notice.clone(),
        settings_read_only: settings.is_read_only(),
    }
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().get().clone()
}

/// deep merges `patch` into the settings and returns the result.
#[tauri::command]
pub fn update_settings(
    state: State<'_, AppState>,
    patch: serde_json::Value,
) -> CmdResult<Settings> {
    let mut store = state.settings.lock();
    store.patch(patch).map_err(|e| e.to_string())?;
    let limits = &store.get().jobs;
    for (kind, max) in [
        (JobKind::Cpu, limits.max_cpu_jobs),
        (JobKind::Gpu, limits.max_gpu_jobs),
        (JobKind::Io, limits.max_io_jobs),
    ] {
        state.jobs.set_limit(kind, max as usize);
    }
    Ok(store.get().clone())
}

#[tauri::command]
pub fn list_jobs(state: State<'_, AppState>) -> Vec<JobInfo> {
    state.jobs.list()
}

#[tauri::command]
pub fn cancel_job(state: State<'_, AppState>, id: JobId) -> CmdResult<()> {
    state.jobs.cancel(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn pause_job(state: State<'_, AppState>, id: JobId) -> CmdResult<()> {
    state.jobs.pause(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn resume_job(state: State<'_, AppState>, id: JobId) -> CmdResult<()> {
    state.jobs.resume(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn retry_job(state: State<'_, AppState>, id: JobId) -> CmdResult<()> {
    state.jobs.retry(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_job(state: State<'_, AppState>, id: JobId) -> CmdResult<()> {
    state.jobs.remove(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clear_finished_jobs(state: State<'_, AppState>) {
    state.jobs.clear_finished();
}

/// queues a job that just counts up, so the job panel can be tried before real tools exist.
/// phase 2 replaces this with real media jobs.
#[tauri::command]
pub fn start_sample_job(state: State<'_, AppState>, seconds: Option<u32>) -> JobId {
    let steps = seconds.unwrap_or(5).clamp(1, 600) * 20;
    let spec = JobSpec::new("Sample job", JobKind::Cpu);
    state.jobs.submit(spec, move |ctx: &JobContext| {
        for step in 0..steps {
            ctx.checkpoint()?;
            std::thread::sleep(Duration::from_millis(50));
            ctx.set_progress((step + 1) as f32 / steps as f32);
        }
        Ok(None)
    })
}
