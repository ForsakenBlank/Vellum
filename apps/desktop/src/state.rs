use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tauri::{AppHandle, Manager};
use vellum_core::app::{AppPaths, AppServices};
use vellum_core::ui::UiReady;
use vellum_export::service::QuickTools;
use vellum_media::MediaService;

use crate::bridge::TauriSink;
use crate::render::Renderer;

/// everything the ipc commands need, managed by tauri
pub struct AppState {
    pub services: AppServices,
    pub media: MediaService,
    pub tools: QuickTools,
    pub renderer: Renderer,
    pub settings_path: PathBuf,
    next_window: AtomicU64,
}

impl AppState {
    pub fn init(app: &AppHandle) -> Result<Self, Box<dyn std::error::Error>> {
        let paths = AppPaths {
            config_dir: app.path().app_config_dir()?,
            data_dir: app.path().app_data_dir()?,
        };
        let settings_path = paths.config_dir.join("settings.json");
        let services = AppServices::start(paths)?;
        services.bus.add_sink(Arc::new(TauriSink::new(app.clone())));
        services.bus.accept_from_ui::<UiReady>();
        log_ui_ready(&services);

        let cache_dir = app.path().app_cache_dir()?.join("media");
        let media = MediaService::start(&services, &cache_dir);
        // previews live in the cache folder, which holds nothing but vellum's own files
        if let Err(err) = app
            .asset_protocol_scope()
            .allow_directory(media.cache_dir(), true)
        {
            tracing::warn!(%err, "previews will not show");
        }

        // recent files may show their own image as a thumbnail
        for file in services.recent_files().files {
            allow_asset(app, Path::new(&file.path));
        }

        let tools = QuickTools::new(&services, &media);
        let renderer = Renderer::start(&services);
        Ok(Self {
            services,
            media,
            tools,
            renderer,
            settings_path,
            next_window: AtomicU64::new(1),
        })
    }

    /// a label for a new document window, `doc-1`, `doc-2` and so on
    pub fn next_window_label(&self) -> String {
        format!("doc-{}", self.next_window.fetch_add(1, Ordering::Relaxed))
    }
}

/// lets the webview load one file through the asset protocol. only files the user opened
/// are allowed, never whole folders.
pub fn allow_asset(app: &AppHandle, path: &Path) {
    if let Err(err) = app.asset_protocol_scope().allow_file(path) {
        tracing::debug!(?path, %err, "could not allow a thumbnail");
    }
}

fn log_ui_ready(services: &AppServices) {
    let ready = services.bus.subscribe::<UiReady>();
    let spawned = std::thread::Builder::new()
        .name("vellum-ui-ready".into())
        .spawn(move || {
            while let Some(event) = ready.recv() {
                tracing::info!(version = %event.ui_version, "a window is ready");
            }
        });
    if let Err(err) = spawned {
        tracing::warn!(%err, "could not watch for the ui");
    }
}
