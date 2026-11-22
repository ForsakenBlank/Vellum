//! the image editor's ipc commands. each one hands straight over to
//! `vellum_image::service::ImageService`; anything that touches pixels runs on a blocking
//! thread so the window never waits for it.

use std::path::PathBuf;

use tauri::State;
use vellum_core::command::CommandRecord;
use vellum_core::documents::DocumentInfo;
use vellum_core::jobs::JobId;
use vellum_gpu::IRect;
use vellum_image::document::LayerId;
use vellum_image::export::ExportOptions;
use vellum_image::fonts::{FontFamily, GoogleFont};
use vellum_image::outputs::{LayerExport, WebOptions, WebPreview};
use vellum_image::paint::{BrushSettings, StrokePoint};
use vellum_image::service::tools::ThumbnailRequest;
use vellum_image::service::{FontPreview, NodeCatalogue};
use vellum_image::service::{ImageService, NewImage, PreviewRequest, TransformRequest};
use vellum_image::transform::TransformTarget;
use vellum_image::warp::{LiquifyBrush, Pin};

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

/// the box free transform starts from: the selection or the layer's pixels
#[tauri::command]
pub async fn image_transform_box(
    state: State<'_, AppState>,
    document: String,
    target: TransformTarget,
) -> IpcResult<[i32; 4]> {
    off_main(&state, move |images| {
        images.transform_box(&document, target)
    })
    .await
}

/// commits a free transform, warp or transform selection as one step
#[tauri::command]
pub async fn image_transform(
    state: State<'_, AppState>,
    document: String,
    request: TransformRequest,
) -> IpcResult<()> {
    off_main(&state, move |images| {
        images.commit_transform(&document, request)
    })
    .await
}

/// exports the picture as png, jpeg, webp or avif in a job
#[tauri::command]
pub fn image_export(
    state: State<'_, AppState>,
    document: String,
    path: PathBuf,
    options: ExportOptions,
) -> IpcResult<JobId> {
    state
        .images
        .export(&document, &path, options)
        .map_err(IpcError::Invalid)
}

// ---- phase 6: adjustments, filters, retouching, type, styles and exports ----

/// every render node, adjustment, preset, filter, style, pattern and custom shape
#[tauri::command]
pub fn image_catalogue(state: State<'_, AppState>) -> NodeCatalogue {
    state.images.catalogue()
}

/// filter gallery thumbnails as one binary blob: width, height, rgba8 for each
#[tauri::command]
pub async fn image_filter_thumbnails(
    state: State<'_, AppState>,
    document: String,
    filters: Vec<ThumbnailRequest>,
    size: u32,
) -> IpcResult<tauri::ipc::Response> {
    let bytes = off_main(&state, move |images| {
        images.filter_thumbnails(&document, &filters, size)
    })
    .await?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub async fn image_warp_begin(
    state: State<'_, AppState>,
    document: String,
    layer: Option<LayerId>,
) -> IpcResult<LayerId> {
    off_main(&state, move |images| images.warp_begin(&document, layer)).await
}

#[tauri::command]
pub async fn image_liquify(
    state: State<'_, AppState>,
    document: String,
    brush: LiquifyBrush,
    points: Vec<[f32; 2]>,
) -> IpcResult<()> {
    off_main(&state, move |images| {
        images.liquify(&document, brush, &points)
    })
    .await
}

#[tauri::command]
pub async fn image_puppet(
    state: State<'_, AppState>,
    document: String,
    pins: Vec<Pin>,
) -> IpcResult<()> {
    off_main(&state, move |images| images.puppet(&document, &pins)).await
}

#[tauri::command]
pub async fn image_warp_end(
    state: State<'_, AppState>,
    document: String,
    keep: bool,
    title: String,
) -> IpcResult<()> {
    off_main(&state, move |images| {
        images.warp_end(&document, keep, &title)
    })
    .await
}

#[tauri::command]
pub async fn image_fonts(state: State<'_, AppState>) -> IpcResult<Vec<FontFamily>> {
    off_main(&state, move |images| Ok(images.fonts())).await
}

#[tauri::command]
pub async fn image_font_previews(
    state: State<'_, AppState>,
    families: Vec<String>,
    text: String,
    size: f32,
) -> IpcResult<Vec<FontPreview>> {
    off_main(&state, move |images| {
        Ok(images.font_previews(&families, &text, size))
    })
    .await
}

#[tauri::command]
pub async fn image_google_fonts(state: State<'_, AppState>) -> IpcResult<Vec<GoogleFont>> {
    off_main(&state, move |images| images.google_fonts()).await
}

#[tauri::command]
pub fn image_download_font(state: State<'_, AppState>, font: GoogleFont) -> IpcResult<JobId> {
    state.images.download_font(font).map_err(IpcError::Invalid)
}

#[tauri::command]
pub async fn image_missing_fonts(
    state: State<'_, AppState>,
    document: String,
) -> IpcResult<Vec<String>> {
    off_main(&state, move |images| images.missing_fonts(&document)).await
}

/// places a picture file as a smart object, embedded or linked
#[tauri::command]
pub fn image_place(
    state: State<'_, AppState>,
    document: String,
    path: PathBuf,
    linked: bool,
) -> IpcResult<JobId> {
    state
        .images
        .place(&document, &path, linked)
        .map_err(IpcError::Invalid)
}

/// saves a psd, or a psb when the path ends in .psb
#[tauri::command]
pub fn image_save_psd(
    state: State<'_, AppState>,
    document: String,
    path: PathBuf,
) -> IpcResult<JobId> {
    state
        .images
        .save_psd(&document, &path)
        .map_err(IpcError::Invalid)
}

#[tauri::command]
pub fn image_export_layers(
    state: State<'_, AppState>,
    document: String,
    folder: PathBuf,
    options: LayerExport,
) -> IpcResult<JobId> {
    state
        .images
        .export_layers(&document, &folder, options)
        .map_err(IpcError::Invalid)
}

#[tauri::command]
pub fn image_export_artboards(
    state: State<'_, AppState>,
    document: String,
    folder: PathBuf,
    options: ExportOptions,
) -> IpcResult<JobId> {
    state
        .images
        .export_artboards(&document, &folder, options)
        .map_err(IpcError::Invalid)
}

/// save for web: the encoded result and the original over the part on view
#[tauri::command]
pub async fn image_web_preview(
    state: State<'_, AppState>,
    document: String,
    options: WebOptions,
    view: IRect,
) -> IpcResult<WebPreview> {
    off_main(&state, move |images| {
        images.web_preview(&document, &options, view)
    })
    .await
}
