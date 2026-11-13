//! carries out what the session asks of the outside world: creating and
//! closing tear-off windows, remembering opened files, and letting the ui
//! show thumbnails of them.

use std::path::Path;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use vellum_core::shell::files::{MediaKind, classify};
use vellum_core::shell::session::{MAIN_WINDOW, ShellUpdate, WindowState};

use crate::state::AppState;

pub fn apply(app: &AppHandle, state: &AppState, update: &ShellUpdate) {
    if !update.opened.is_empty() {
        let paths: Vec<String> = update.opened.iter().map(|open| open.path.clone()).collect();
        allow_previews(app, &paths);
        state.recent.touch_all(&paths);
    }
    for label in &update.windows_removed {
        if let Some(window) = app.get_webview_window(label) {
            // destroy skips the close request, the session already let it go
            if let Err(error) = window.destroy() {
                tracing::warn!(window = label, %error, "could not close a window");
            }
        }
    }
    for window in &update.windows_added {
        if let Err(error) = create_window(app, window) {
            tracing::error!(window = window.label, %error, "could not open a new window");
            // its documents go back to the main window instead of vanishing
            let back = state.shell.window_closed(&window.label);
            apply(app, state, &back);
        }
    }
}

fn create_window(app: &AppHandle, window: &WindowState) -> tauri::Result<()> {
    if app.get_webview_window(&window.label).is_some() {
        return Ok(());
    }
    let mut builder = WebviewWindowBuilder::new(app, &window.label, WebviewUrl::default())
        .title("Vellum")
        .inner_size(1100.0, 720.0)
        .min_inner_size(640.0, 420.0)
        .background_color(tauri::window::Color(15, 16, 20, 255));
    if let Some(position) = window.position {
        // the pointer was over the tab's middle, so open with the title bar under it
        builder = builder.position((position.x - 120.0).max(0.0), (position.y - 16.0).max(0.0));
    }
    builder.build()?;
    Ok(())
}

/// lets the ui load these files through the asset protocol, which is how the
/// home screen shows image thumbnails until phase 2 makes real ones. only
/// images are allowed, and only files the user opened
pub fn allow_previews(app: &AppHandle, paths: &[String]) {
    let scope = app.asset_protocol_scope();
    for path in paths {
        let path = Path::new(path);
        if matches!(classify(path), MediaKind::Image | MediaKind::Vector)
            && let Err(error) = scope.allow_file(path)
        {
            tracing::warn!(path = %path.display(), %error, "could not allow a preview");
        }
    }
}

/// opens files given on the command line, or passed on by a second launch
pub fn open_from_args(app: &AppHandle, args: &[String]) {
    let paths: Vec<String> = args
        .iter()
        .skip(1)
        .filter(|arg| !arg.starts_with('-') && Path::new(arg).is_file())
        .cloned()
        .collect();
    if paths.is_empty() {
        return;
    }
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let record = vellum_core::command::CommandRecord::new(
        "shell.open",
        serde_json::json!({ "paths": paths, "window": MAIN_WINDOW }),
    );
    match state.shell.run(&record) {
        Ok(update) => apply(app, &state, &update),
        Err(error) => tracing::warn!(%error, "could not open the files vellum was started with"),
    }
}

/// brings the main window to the front, for a second launch
pub fn focus_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
