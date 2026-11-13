//! the commands the ui can call. each one is a thin wrapper: check the input,
//! call into a service, turn errors into something the ui can show.
//!
//! all of them are `async` in the tauri sense, which only means they run off
//! the main thread so a slow disk can never freeze the window.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use ts_rs::TS;
use vellum_core::command::{CommandRecord, CommandSpec};
use vellum_core::jobs::{JobContext, JobError, JobId, JobSpec, JobsOverview, Priority};
use vellum_core::shell::autosave::RecoveryOffer;
use vellum_core::shell::files::{MediaKind, OpenPlan, OpenTarget, classify, plan_open};
use vellum_core::shell::recent::RecentEntry;
use vellum_core::shell::session::SessionView;

use crate::error::IpcError;
use crate::host;
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

/// every command the backend registers, for the palette and the shortcut editor
#[tauri::command(async)]
pub fn commands_list(state: State<'_, AppState>) -> Vec<CommandSpec> {
    state.shell.specs()
}

#[tauri::command(async)]
pub fn shell_view(state: State<'_, AppState>) -> SessionView {
    state.shell.view()
}

/// runs a shell command (open, close, switch tab, tear off) and returns the
/// session as it ended up. the same view also goes out as `session.changed`
#[tauri::command(async)]
pub fn shell_run(
    app: AppHandle,
    state: State<'_, AppState>,
    record: CommandRecord,
) -> Ipc<SessionView> {
    let update = state.shell.run(&record)?;
    host::apply(&app, &state, &update);
    Ok(update.view)
}

/// what opening these files would do, for the drop overlay
#[tauri::command(async)]
pub fn files_plan(paths: Vec<String>, target: OpenTarget) -> OpenPlan {
    plan_open(&paths, &target)
}

/// the system open dialog. returns nothing when the user cancels
#[tauri::command(async)]
pub fn files_pick(app: AppHandle, window: WebviewWindow) -> Vec<String> {
    app.dialog()
        .file()
        .set_parent(&window)
        .set_title("Open")
        .blocking_pick_files()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|file| file.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

#[tauri::command(async)]
pub fn recent_list(app: AppHandle, state: State<'_, AppState>) -> Vec<RecentEntry> {
    let entries = state.recent.list();
    let paths: Vec<String> = entries.iter().map(|entry| entry.path.clone()).collect();
    host::allow_previews(&app, &paths);
    entries
}

#[tauri::command(async)]
pub fn recent_pin(state: State<'_, AppState>, path: String, pinned: bool) {
    state.recent.set_pinned(&path, pinned);
}

#[tauri::command(async)]
pub fn recent_remove(state: State<'_, AppState>, path: String) {
    state.recent.remove(&path);
}

#[tauri::command(async)]
pub fn recent_clear(state: State<'_, AppState>) {
    state.recent.clear();
}

/// what a crashed run left behind, if the user has not answered for it yet
#[tauri::command(async)]
pub fn recovery_offer(state: State<'_, AppState>) -> Option<RecoveryOffer> {
    state.session_files.as_ref().and_then(|files| files.offer())
}

#[tauri::command(async)]
pub fn recovery_restore(app: AppHandle, state: State<'_, AppState>) -> Ipc<SessionView> {
    let files = state
        .session_files
        .as_ref()
        .ok_or_else(IpcError::no_recovery)?;
    let snapshot = files.recovery().ok_or_else(IpcError::no_recovery)?;
    let update = state.shell.restore(&snapshot);
    let paths: Vec<String> = snapshot
        .documents
        .iter()
        .filter_map(|doc| doc.path.clone())
        .collect();
    host::allow_previews(&app, &paths);
    host::apply(&app, &state, &update);
    files.forget_recovery().map_err(IpcError::recovery_io)?;
    Ok(update.view)
}

#[tauri::command(async)]
pub fn recovery_discard(state: State<'_, AppState>) -> Ipc<()> {
    if let Some(files) = &state.session_files {
        files.forget_recovery().map_err(IpcError::recovery_io)?;
    }
    Ok(())
}

/// opens a file in the app the system picks for it, such as a finished export.
/// only media files: the ui must never be able to start a program this way
#[tauri::command(async)]
pub fn path_open(app: AppHandle, path: String) -> Ipc<()> {
    let file = Path::new(&path);
    if !file.is_file() {
        return Err(IpcError::missing_file(&path));
    }
    if matches!(classify(file), MediaKind::Unknown | MediaKind::Project) {
        return Err(IpcError::not_media(&path));
    }
    app.opener()
        .open_path(path.clone(), None::<&str>)
        .map_err(|error| IpcError::open_failed(&path, &error))
}

/// shows a file in explorer, finder or the linux file manager
#[tauri::command(async)]
pub fn path_reveal(app: AppHandle, path: String) -> Ipc<()> {
    if !Path::new(&path).exists() {
        return Err(IpcError::missing_file(&path));
    }
    app.opener()
        .reveal_item_in_dir(&path)
        .map_err(|error| IpcError::open_failed(&path, &error))
}

/// a pretend job for checking the queue by hand until real ones exist.
/// it counts to `seconds` and then succeeds, or fails if asked to
#[tauri::command(async)]
pub fn dev_sample_job(
    state: State<'_, AppState>,
    seconds: f32,
    fail: bool,
    output: Option<String>,
) -> JobId {
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
        Ok(match &output {
            Some(path) => json!({ "seconds": seconds, "path": path }),
            None => json!({ "seconds": seconds }),
        })
    });
    handle.id()
}
