//! the render engine in the window: commands that open a picture in a viewport and stream
//! its display tiles to the webview, which draws them with webgl (`ARCHITECTURE.md` section
//! 24). the engine starts on a background thread so start-up never waits for the gpu.

use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tauri::State;
use vellum_core::AppSettings;
use vellum_core::app::AppServices;
use vellum_core::settings::{RenderSettings, SettingsChanged};
use vellum_gpu::view::{TileId, ViewSettings, ViewState};
use vellum_gpu::{
    ColorSpace, ColorTransform, Engine, EngineOptions, EngineStatus, IccProfile, ViewHub, ViewInfo,
    WorkingSpace,
};
use vellum_media::picture::{self, DecodeOptions};

use crate::ipc::{IpcError, IpcResult};
use crate::state::AppState;

/// how long one frame request may spend rendering before it returns what it has. the
/// window keeps drawing at full speed meanwhile, this only paces how fast tiles arrive.
const FRAME_BUDGET: Duration = Duration::from_millis(12);

/// the engine and its viewports, ready once the background start has finished
#[derive(Clone, Default)]
pub struct Renderer {
    hub: Arc<OnceLock<ViewHub>>,
    /// called after the display profile changes, so image views rebuild with it
    on_display: Arc<OnceLock<Box<dyn Fn() + Send + Sync>>>,
}

impl Renderer {
    /// opens the engine on a background thread and follows the render settings
    pub fn start(services: &AppServices) -> Self {
        let renderer = Self::default();
        let settings = services.settings.get().render;
        let changes = services.bus.subscribe::<SettingsChanged<AppSettings>>();
        let hub = renderer.hub.clone();
        let on_display = renderer.on_display.clone();
        let spawned = std::thread::Builder::new()
            .name("vellum-render".into())
            .spawn(move || {
                let engine = Engine::new(EngineOptions {
                    use_gpu: settings.use_gpu,
                    gpu_cache_bytes: u64::from(settings.gpu_cache_mb) << 20,
                    cpu_cache_bytes: u64::from(settings.cpu_cache_mb) << 20,
                    ..Default::default()
                });
                let status = engine.status();
                tracing::info!(backend = ?status.backend, adapter = ?status.adapter, "render engine ready");
                let _ = hub.set(ViewHub::new(Arc::new(engine)));
                let Some(hub) = hub.get() else { return };
                let mut last = settings;
                while let Some(change) = changes.recv() {
                    let next = change.settings.render.clone();
                    if next != last {
                        apply(hub, &last, &next);
                        if last.display_profile != next.display_profile
                            && let Some(f) = on_display.get()
                        {
                            f();
                        }
                        last = next;
                    }
                }
            });
        if let Err(err) = spawned {
            tracing::error!(%err, "the render engine could not start");
        }
        renderer
    }

    /// waits for the engine to finish starting. only call off the main thread.
    fn hub(&self) -> &ViewHub {
        self.hub.wait()
    }

    /// the slot the hub lands in once the engine has started, for the image editor
    pub fn hub_slot(&self) -> Arc<OnceLock<ViewHub>> {
        self.hub.clone()
    }

    /// runs `f` whenever the display profile changes
    pub fn on_display_change(&self, f: impl Fn() + Send + Sync + 'static) {
        let _ = self.on_display.set(Box::new(f));
    }

    pub fn close(&self, document: &str) {
        if let Some(hub) = self.hub.get() {
            hub.close(document);
        }
    }
}

fn apply(hub: &ViewHub, old: &RenderSettings, new: &RenderSettings) {
    let engine = hub.engine();
    engine.set_use_gpu(new.use_gpu);
    engine.set_budgets(
        u64::from(new.gpu_cache_mb) << 20,
        u64::from(new.cpu_cache_mb) << 20,
    );
    if old.display_profile != new.display_profile {
        hub.set_display(display_transform(new, WORKING));
    }
}

/// what pictures opened only to view render in. image documents pick their own working
/// space (srgb storage for 8 and 16 bit documents, like photoshop, linear for 32 bit).
const WORKING: WorkingSpace = WorkingSpace::SRGB;

/// from a working space to the display, using the profile picked in settings
pub fn display_transform(settings: &RenderSettings, working: WorkingSpace) -> ColorTransform {
    let fallback = ColorTransform::between(working.space(), ColorSpace::SRGB);
    let Some(path) = &settings.display_profile else {
        return fallback;
    };
    let profile = std::fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|bytes| IccProfile::from_bytes(&bytes).map_err(|e| e.to_string()))
        .and_then(|p| p.from_working(working).map_err(|e| e.to_string()));
    profile.unwrap_or_else(|err| {
        tracing::warn!(%err, path, "the display profile could not be used, showing srgb");
        fallback
    })
}

async fn off_main<T: Send + 'static>(
    work: impl FnOnce() -> IpcResult<T> + Send + 'static,
) -> IpcResult<T> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|err| IpcError::Invalid(format!("the render task stopped: {err}")))?
}

/// opens a document's picture in a viewport, decoding it the first time. the view stays
/// open while the document does, so switching tabs back is instant.
#[tauri::command]
pub async fn open_view(state: State<'_, AppState>, document: String) -> IpcResult<ViewInfo> {
    let info = state.services.documents.info(&document)?;
    if info.doc_type == "image" {
        let images = state.images.clone();
        return off_main(move || images.open_view(&document).map_err(IpcError::Invalid)).await;
    }
    let path = info
        .path
        .ok_or_else(|| IpcError::Invalid("This document has no picture to show yet.".into()))?;
    let renderer = state.renderer.clone();
    let media = state.media.clone();
    let settings = state.services.settings.get().render;
    off_main(move || {
        let hub = renderer.hub();
        if let Some(open) = hub.info(&document) {
            return Ok(open);
        }
        let mut picture =
            picture::decode(Path::new(&path), DecodeOptions::default(), media.ffmpeg())?;
        let icc = picture.icc.take();
        if picture.frames.is_empty() {
            return Err(IpcError::Invalid("The picture has nothing to show.".into()));
        }
        // the first frame moves into the viewport rather than being copied
        let image = picture.frames.swap_remove(0).image;
        let (w, h) = image.dimensions();
        let view = ViewSettings {
            working: WORKING,
            display: display_transform(&settings, WORKING),
            ..Default::default()
        };
        Ok(hub.open_rgba8(&document, w, h, image.into_raw(), icc.as_deref(), view))
    })
    .await
}

/// the display tiles the window is missing, as one binary frame
#[tauri::command]
pub async fn view_frame(
    state: State<'_, AppState>,
    document: String,
    view: ViewState,
    evicted: Vec<TileId>,
    reset: bool,
) -> IpcResult<tauri::ipc::Response> {
    let renderer = state.renderer.clone();
    off_main(move || {
        let bytes = renderer
            .hub()
            .frame(&document, &view, &evicted, reset, FRAME_BUDGET)
            .map_err(|err| IpcError::Invalid(err.to_string()))?;
        Ok(tauri::ipc::Response::new(bytes))
    })
    .await
}

#[tauri::command]
pub fn close_view(state: State<'_, AppState>, document: String) {
    state.renderer.close(&document);
}

/// which device draws and why, for settings
#[tauri::command]
pub async fn render_status(state: State<'_, AppState>) -> IpcResult<EngineStatus> {
    let renderer = state.renderer.clone();
    off_main(move || Ok(renderer.hub().engine().status())).await
}
