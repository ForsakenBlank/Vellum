//! commands the ui can invoke. they stay thin: parse, call into the core, map errors.

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use serde::{Serialize, Serializer};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;
use vellum_core::AppSettings;
use vellum_core::app::{AutosaveStatus, CommandOutcome, OpenFilesResult};
use vellum_core::command::{CommandError, CommandInfo, CommandRecord};
use vellum_core::documents::{DocumentError, DocumentInfo, DocumentState};
use vellum_core::history::{HistoryView, NodeId, SnapshotId};
use vellum_core::jobs::{
    JobContext, JobError, JobId, JobPriority, JobQueueError, JobResource, JobSpec, JobsOverview,
};
use vellum_core::notifications::NotificationsChanged;
use vellum_core::recent::{self, RecentFiles};
use vellum_core::recovery::RecoveryOffer;
use vellum_core::settings::SettingsError;
use vellum_core::shortcuts::Keymap;
use vellum_core::ui::AppInfo;

use crate::state::{AppState, allow_asset};

/// errors reach the ui as a plain message
#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error(transparent)]
    Jobs(#[from] JobQueueError),
    #[error(transparent)]
    Event(#[from] vellum_core::events::EventError),
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error(transparent)]
    Document(#[from] DocumentError),
    #[error(transparent)]
    Tauri(#[from] tauri::Error),
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
    state.services.settings.get()
}

/// takes a json merge patch, eg `{ "appearance": { "theme": "light" } }`
#[tauri::command]
pub fn update_settings(
    state: State<'_, AppState>,
    patch: serde_json::Value,
) -> IpcResult<AppSettings> {
    Ok(state.services.settings.apply_patch(&patch)?)
}

#[tauri::command]
pub fn reset_settings(state: State<'_, AppState>) -> IpcResult<AppSettings> {
    Ok(state.services.settings.reset()?)
}

#[tauri::command]
pub fn list_commands(state: State<'_, AppState>) -> Vec<CommandInfo> {
    state.services.command_infos()
}

#[tauri::command]
pub fn run_command(
    state: State<'_, AppState>,
    record: CommandRecord,
    document: Option<String>,
) -> IpcResult<CommandOutcome> {
    Ok(state.services.run_command(&record, document)?)
}

#[tauri::command]
pub fn get_keymap(state: State<'_, AppState>) -> Keymap {
    state.services.keymap()
}

#[tauri::command]
pub fn list_documents(state: State<'_, AppState>) -> Vec<DocumentInfo> {
    state.services.documents.list()
}

#[tauri::command]
pub fn open_files(
    app: AppHandle,
    state: State<'_, AppState>,
    paths: Vec<PathBuf>,
    tool: Option<String>,
    window: String,
) -> OpenFilesResult {
    for path in &paths {
        allow_asset(&app, path);
    }
    state.services.open_files(&paths, tool.as_deref(), &window)
}

#[tauri::command]
pub fn close_document(state: State<'_, AppState>, id: String) -> IpcResult<()> {
    Ok(state.services.documents.close(&id)?)
}

#[tauri::command]
pub fn move_document(
    state: State<'_, AppState>,
    id: String,
    window: String,
    index: Option<usize>,
) -> IpcResult<()> {
    Ok(state
        .services
        .documents
        .move_to_window(&id, &window, index)?)
}

/// opens a document in a window of its own
#[tauri::command]
pub fn tear_off_document(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> IpcResult<String> {
    let info = state.services.documents.info(&id)?;
    let label = state.next_window_label();
    WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html".into()))
        .title(format!("{} - Vellum", info.title))
        .inner_size(1100.0, 720.0)
        .min_inner_size(640.0, 420.0)
        .build()?;
    state.services.documents.move_to_window(&id, &label, None)?;
    Ok(label)
}

#[tauri::command]
pub fn document_command(
    state: State<'_, AppState>,
    id: String,
    record: CommandRecord,
) -> IpcResult<()> {
    Ok(state.services.documents.execute(&id, &record)?)
}

/// ends a run of typing or a drag, so the next edit is its own undo step
#[tauri::command]
pub fn seal_document(state: State<'_, AppState>, id: String) -> IpcResult<()> {
    Ok(state.services.documents.seal(&id)?)
}

#[tauri::command]
pub fn get_document(state: State<'_, AppState>, id: String) -> IpcResult<DocumentState> {
    Ok(state.services.documents.state(&id)?)
}

#[tauri::command]
pub fn document_history(state: State<'_, AppState>, id: String) -> IpcResult<HistoryView> {
    Ok(state.services.documents.history(&id)?)
}

#[tauri::command]
pub fn checkout_history(state: State<'_, AppState>, id: String, node: NodeId) -> IpcResult<()> {
    Ok(state.services.documents.checkout(&id, node)?)
}

#[tauri::command]
pub fn create_snapshot(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> IpcResult<SnapshotId> {
    Ok(state.services.documents.create_snapshot(&id, name)?)
}

#[tauri::command]
pub fn list_recent(state: State<'_, AppState>) -> RecentFiles {
    state.services.recent_files()
}

#[tauri::command]
pub fn pin_recent(state: State<'_, AppState>, path: String, pinned: bool) -> IpcResult<()> {
    recent::set_pinned(&state.services.recent, &path, pinned)?;
    Ok(())
}

#[tauri::command]
pub fn remove_recent(state: State<'_, AppState>, path: String) -> IpcResult<()> {
    recent::remove(&state.services.recent, &path)?;
    Ok(())
}

#[tauri::command]
pub fn recovery_offer(state: State<'_, AppState>) -> Option<RecoveryOffer> {
    state.services.recovery.as_ref().and_then(|r| r.offer())
}

#[tauri::command]
pub fn restore_session(state: State<'_, AppState>) -> Vec<DocumentInfo> {
    match &state.services.recovery {
        Some(recovery) => recovery.restore(&state.services.documents),
        None => Vec::new(),
    }
}

#[tauri::command]
pub fn discard_recovery(state: State<'_, AppState>) {
    if let Some(recovery) = &state.services.recovery {
        recovery.discard();
    }
}

#[tauri::command]
pub fn autosave_status(state: State<'_, AppState>) -> AutosaveStatus {
    state.services.autosave_status()
}

#[tauri::command]
pub fn list_notifications(state: State<'_, AppState>) -> NotificationsChanged {
    state.services.notifications.list()
}

#[tauri::command]
pub fn dismiss_notification(state: State<'_, AppState>, id: u64) {
    state.services.notifications.dismiss(id);
}

/// opens a file a job produced with the system's default app. only job results are
/// allowed, so the ui cannot be used to open arbitrary files.
#[tauri::command]
pub fn open_result(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    reveal: bool,
) -> IpcResult<()> {
    if !state.services.is_job_result(&path) {
        return Err(IpcError::Invalid(
            "That file is not the result of a job in the list.".into(),
        ));
    }
    let opener = app.opener();
    let opened = if reveal {
        opener.reveal_item_in_dir(&path)
    } else {
        opener.open_path(&path, None::<&str>)
    };
    opened.map_err(|err| IpcError::Invalid(format!("Could not open {path}: {err}")))
}

#[tauri::command]
pub fn list_jobs(state: State<'_, AppState>) -> JobsOverview {
    state.services.jobs.overview()
}

#[tauri::command]
pub fn cancel_job(state: State<'_, AppState>, id: JobId) -> IpcResult<()> {
    Ok(state.services.jobs.cancel(id)?)
}

#[tauri::command]
pub fn pause_job(state: State<'_, AppState>, id: JobId) -> IpcResult<()> {
    Ok(state.services.jobs.pause(id)?)
}

#[tauri::command]
pub fn resume_job(state: State<'_, AppState>, id: JobId) -> IpcResult<()> {
    Ok(state.services.jobs.resume(id)?)
}

#[tauri::command]
pub fn retry_job(state: State<'_, AppState>, id: JobId) -> IpcResult<()> {
    Ok(state.services.jobs.retry(id)?)
}

#[tauri::command]
pub fn remove_job(state: State<'_, AppState>, id: JobId) -> IpcResult<()> {
    Ok(state.services.jobs.remove(id)?)
}

#[tauri::command]
pub fn clear_finished_jobs(state: State<'_, AppState>) {
    state.services.jobs.clear_finished();
}

#[tauri::command]
pub fn set_job_priority(
    state: State<'_, AppState>,
    id: JobId,
    priority: JobPriority,
) -> IpcResult<()> {
    Ok(state.services.jobs.set_priority(id, priority)?)
}

#[tauri::command]
pub fn move_job(state: State<'_, AppState>, id: JobId, before: Option<JobId>) -> IpcResult<()> {
    Ok(state.services.jobs.move_before(id, before)?)
}

/// a job that does nothing but tick for a while, then writes a small text file as its
/// result. it lets you try the queue and "open result" by hand until real jobs arrive in
/// phase 2. `fail_first` makes the first attempt fail to show retries.
#[tauri::command]
pub fn start_sample_job(
    app: AppHandle,
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
    let out_dir = app.path().app_cache_dir()?.join("sample-jobs");
    let steps = u64::from(seconds) * 20;
    let spec = JobSpec::new(format!("Sample job ({seconds} s)"), resource)
        .with_retries(u32::from(fail_first), Duration::from_secs(1));
    let id = state
        .services
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
            std::fs::create_dir_all(&out_dir)?;
            let path = out_dir.join(format!("sample-{}.txt", ctx.id().0));
            std::fs::write(
                &path,
                format!("sample job {} ran {steps} steps\n", ctx.id()),
            )?;
            Ok(serde_json::json!({ "path": path.display().to_string() }))
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
    Ok(state.services.bus.dispatch_from_ui(&name, payload)?)
}
