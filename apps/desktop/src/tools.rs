//! commands for the quick tools: run a session, the live estimates its view shows, previews
//! of inputs and results, and copying results to the clipboard as files.
//!
//! files the ui may preview or copy are limited to what the user gave a quick tool and
//! what jobs made, like `open_result`, so these cannot be used to read other files.

use std::path::{Path, PathBuf};

use clipboard_rs::{Clipboard, ClipboardContext};
use tauri::{AppHandle, State};
use vellum_core::batch_rename::{self, RenamePattern, RenamePlan};
use vellum_core::jobs::JobId;
use vellum_export::anim::AnimOptions;
use vellum_export::convert::ConvertTarget;
use vellum_export::service::{AnimEstimate, GifFramesView, QuickTools, SizePlan};
use vellum_export::size::SizeOptions;

use crate::ipc::IpcError;
use crate::state::{AppState, allow_asset};

type IpcResult<T> = Result<T, IpcError>;

/// runs quick tool work off the main thread, since it probes files and starts ffmpeg
async fn off_main<T: Send + 'static>(
    state: &State<'_, AppState>,
    work: impl FnOnce(QuickTools) -> IpcResult<T> + Send + 'static,
) -> IpcResult<T> {
    let tools = state.tools.clone();
    tauri::async_runtime::spawn_blocking(move || work(tools))
        .await
        .map_err(|err| IpcError::Invalid(format!("the quick tool task stopped: {err}")))?
}

/// starts the session's tool on its files, one job per file
#[tauri::command]
pub fn run_quick_tool(state: State<'_, AppState>, document: String) -> IpcResult<Vec<JobId>> {
    state
        .tools
        .run(&document)
        .map_err(|err| IpcError::Invalid(err.to_string()))
}

#[tauri::command]
pub async fn size_plan(
    state: State<'_, AppState>,
    path: PathBuf,
    options: SizeOptions,
) -> IpcResult<SizePlan> {
    off_main(&state, move |tools| Ok(tools.size_plan(&path, &options)?)).await
}

#[tauri::command]
pub async fn anim_estimate(
    state: State<'_, AppState>,
    path: PathBuf,
    options: AnimOptions,
) -> IpcResult<AnimEstimate> {
    off_main(&state, move |tools| {
        Ok(tools.anim_estimate(&path, &options)?)
    })
    .await
}

#[tauri::command]
pub async fn gif_frames(state: State<'_, AppState>, path: PathBuf) -> IpcResult<GifFramesView> {
    off_main(&state, move |tools| Ok(tools.gif_frames(&path)?)).await
}

#[tauri::command]
pub async fn convert_targets(
    state: State<'_, AppState>,
    path: PathBuf,
) -> IpcResult<Vec<ConvertTarget>> {
    off_main(&state, move |tools| Ok(tools.convert_targets(&path)?)).await
}

/// the new names a rename pattern gives the session's files, before anything is renamed.
/// it reads the list from the document, so it can only look at files the user added.
#[tauri::command]
pub async fn rename_plan(
    state: State<'_, AppState>,
    document: String,
    pattern: RenamePattern,
) -> IpcResult<RenamePlan> {
    let info = state
        .services
        .documents
        .info(&document)
        .map_err(|err| IpcError::Invalid(err.to_string()))?;
    let inputs: Vec<PathBuf> = info.inputs.iter().map(PathBuf::from).collect();
    tauri::async_runtime::spawn_blocking(move || Ok(batch_rename::plan(&inputs, &pattern)))
        .await
        .map_err(|err| IpcError::Invalid(format!("the rename task stopped: {err}")))?
}

/// the exact frame at `seconds` as a cached picture, for trim handles
#[tauri::command]
pub async fn media_frame(
    state: State<'_, AppState>,
    path: PathBuf,
    seconds: f64,
    height: u32,
) -> IpcResult<String> {
    if !seconds.is_finite() || !(16..=1080).contains(&height) {
        return Err(IpcError::Invalid(
            "a frame needs a time and a height of 16 to 1080 pixels".into(),
        ));
    }
    let media = state.media.clone();
    tauri::async_runtime::spawn_blocking(move || {
        Ok(media
            .frame_at(&path, seconds, height)?
            .display()
            .to_string())
    })
    .await
    .map_err(|err| IpcError::Invalid(format!("the frame task stopped: {err}")))?
}

/// true for files the user handed to a quick tool or that a job made
fn may_touch(state: &AppState, path: &str) -> bool {
    state.services.is_job_result(path)
        || state.services.documents.list().iter().any(|doc| {
            doc.inputs.iter().any(|input| input == path) || doc.path.as_deref() == Some(path)
        })
}

/// lets the window show a file at full size, for the before and after slider
#[tauri::command]
pub fn preview_file(app: AppHandle, state: State<'_, AppState>, path: String) -> IpcResult<()> {
    if !may_touch(&state, &path) {
        return Err(IpcError::Invalid(
            "Only files in a quick tool or made by a job can be previewed.".into(),
        ));
    }
    allow_asset(&app, Path::new(&path));
    Ok(())
}

/// puts files on the clipboard the way a file manager's copy does, so they paste straight
/// into a chat app
#[tauri::command]
pub async fn copy_files(state: State<'_, AppState>, paths: Vec<String>) -> IpcResult<()> {
    if paths.is_empty() {
        return Err(IpcError::Invalid("Nothing to copy.".into()));
    }
    for path in &paths {
        if !may_touch(&state, path) {
            return Err(IpcError::Invalid(format!(
                "{path} is not a result Vellum made, so it was not copied."
            )));
        }
        if !Path::new(path).exists() {
            return Err(IpcError::Invalid(format!("{path} is not there any more.")));
        }
    }
    tauri::async_runtime::spawn_blocking(move || {
        let clipboard = ClipboardContext::new().map_err(|err| {
            IpcError::Invalid(format!("The clipboard could not be opened: {err}"))
        })?;
        let files = paths.iter().map(|p| clipboard_path(p)).collect();
        clipboard
            .set_files(files)
            .map_err(|err| IpcError::Invalid(format!("The files could not be copied: {err}")))
    })
    .await
    .map_err(|err| IpcError::Invalid(format!("the clipboard task stopped: {err}")))?
}

/// linux file managers read a list of file urls, where spaces and brackets must be
/// escaped. windows and macos take plain paths.
fn clipboard_path(path: &str) -> String {
    if cfg!(any(target_os = "windows", target_os = "macos")) {
        return path.to_owned();
    }
    let mut url = String::from("file://");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-_.~".contains(&byte) {
            url.push(char::from(byte));
        } else {
            url.push_str(&format!("%{byte:02X}"));
        }
    }
    url
}
