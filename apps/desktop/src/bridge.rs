//! carries bus events to the webview.

use tauri::{AppHandle, Emitter};

/// forwards every bus event to the ui as a tauri event with the same name
pub struct TauriSink {
    app: AppHandle,
}

impl TauriSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl vellum_core::EventSink for TauriSink {
    fn deliver(&self, name: &'static str, payload: &serde_json::Value) {
        if let Err(err) = self.app.emit(name, payload) {
            tracing::warn!(event = name, %err, "could not send event to the ui");
        }
    }
}
