//! the image editor's ipc commands. each one hands straight over to
//! `vellum_image::service::ImageService`; anything that touches pixels runs on a blocking
//! thread so the window never waits for it.

use std::path::PathBuf;

use tauri::State;
use vellum_core::command::CommandRecord;
use vellum_core::documents::DocumentInfo;
use vellum_core::jobs::JobId;
use vellum_image::paint::{BrushSettings, StrokePoint};
use vellum_image::service::{ImageService, NewImage, PreviewRequest};

use crate::ipc::{IpcError, IpcResult};
use crate::state::AppState;

async fn off_main<T: Send + 'static>(
    state: &State<'_, AppState>,
    work: impl FnOnce(ImageService) -> Result<T, String> + Send + 'static,
) -> IpcResult<T> {
    let images = state.images.clone();
    tauri::async_runtime::spawn_blocking(move || work(images))
        .await
        .map_err(|err| IpcError::Invalid(format!("the image task stopped: {err}")))?
        .map_err(IpcError::Invalid)
}

#[tauri::command]
pub fn image_new(
    state: State<'_, AppState>,
    spec: NewImage,
    window: String,
) -> IpcResult<DocumentInfo> {
    state
        .images
        .new_document(&spec, &window)
        .map_err(IpcError::Invalid)
}

/// runs an image document command (layers, selections, fills) and redraws
#[tauri::command]
pub async fn image_command(
    state: State<'_, AppState>,
    document: String,
    record: CommandRecord,
) -> IpcResult<()> {
    off_main(&state, move |images| images.execute(&document, &record)).await
}

#[tauri::command]
pub async fn image_stroke_begin(
    state: State<'_, AppState>,
    document: String,
    settings: BrushSettings,
) -> IpcResult<()> {
    off_main(&state, move |images| {
        images.begin_stroke(&document, settings)
    })
    .await
}

#[tauri::command]
pub async fn image_stroke(
    state: State<'_, AppState>,
    document: String,
    points: Vec<StrokePoint>,
) -> IpcResult<()> {
    off_main(&state, move |images| images.stroke(&document, &points)).await
}

#[tauri::command]
pub async fn image_stroke_end(state: State<'_, AppState>, document: String) -> IpcResult<()> {
    off_main(&state, move |images| images.end_stroke(&document)).await
}

#[tauri::command]
pub async fn image_stroke_cancel(state: State<'_, AppState>, document: String) -> IpcResult<()> {
    off_main(&state, move |images| {
        images.cancel_stroke(&document);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn image_preview(
    state: State<'_, AppState>,
    document: String,
    request: PreviewRequest,
) -> IpcResult<()> {
    off_main(&state, move |images| {
        images.set_preview(&document, request);
        Ok(())
    })
    .await
}

/// the selection's outline for marching ants
#[tauri::command]
pub async fn image_outline(
    state: State<'_, AppState>,
    document: String,
    level: u32,
) -> IpcResult<Vec<Vec<[f32; 2]>>> {
    off_main(&state, move |images| images.outline(&document, level)).await
}

#[tauri::command]
pub async fn image_magnetic_begin(
    state: State<'_, AppState>,
    document: String,
    sample_all: bool,
    contrast: f32,
) -> IpcResult<()> {
    off_main(&state, move |images| {
        images.begin_magnetic(&document, sample_all, contrast)
    })
    .await
}

#[tauri::command]
pub async fn image_magnetic(
    state: State<'_, AppState>,
    document: String,
    from: [i32; 2],
    to: [i32; 2],
    width: i32,
) -> IpcResult<Vec<[i32; 2]>> {
    off_main(&state, move |images| {
        images.magnetic(&document, (from[0], from[1]), (to[0], to[1]), width)
    })
    .await
}

#[tauri::command]
pub fn image_magnetic_end(state: State<'_, AppState>, document: String) {
    state.images.end_magnetic(&document);
}

#[tauri::command]
pub async fn image_pick_color(
    state: State<'_, AppState>,
    document: String,
    x: i32,
    y: i32,
    size: u32,
    all: bool,
) -> IpcResult<[f32; 4]> {
    off_main(&state, move |images| {
        images.pick_color(&document, x, y, size, all)
    })
    .await
}

/// layer, mask and navigator thumbnails as one binary blob
#[tauri::command]
pub async fn image_thumbnails(
    state: State<'_, AppState>,
    document: String,
    layers: Vec<(u32, bool)>,
    size: u32,
) -> IpcResult<tauri::ipc::Response> {
    let bytes = off_main(&state, move |images| {
        images.thumbnails(&document, &layers, size)
    })
    .await?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub fn image_save_as(
    state: State<'_, AppState>,
    document: String,
    path: PathBuf,
) -> IpcResult<JobId> {
    state
        .images
        .save_as(&document, &path)
        .map_err(IpcError::Invalid)
}
