//! the desktop shell. it creates the window, owns the long lived services
//! (event bus, settings, job queue) and exposes them to the ui as commands.
//! everything with real logic lives in the `vellum-*` crates.

mod autosave;
mod bindings;
mod commands;
mod error;
mod host;
mod state;

use tauri::{Manager, RunEvent, WindowEvent};
use vellum_core::shell::session::MAIN_WINDOW;

use crate::state::AppState;

/// the one tauri event every backend event travels on, wrapped in an envelope
pub const EVENT_CHANNEL: &str = "vellum:event";

pub fn run() {
    init_logging();

    let app = tauri::Builder::default()
        // must be the first plugin, so a second launch hands over before anything else starts
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            host::focus_main(app);
            host::open_from_args(app, &args);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let state = AppState::start(app.handle())?;
            app.manage(state);
            let args: Vec<String> = std::env::args().collect();
            host::open_from_args(app.handle(), &args);
            Ok(())
        })
        .on_window_event(|window, event| {
            let WindowEvent::CloseRequested { .. } = event else {
                return;
            };
            let app = window.app_handle();
            if window.label() == MAIN_WINDOW {
                // closing the main window quits, torn off windows included
                app.exit(0);
            } else if let Some(state) = app.try_state::<AppState>() {
                // the session takes the documents back into the main window
                let update = state.shell.window_closed(window.label());
                host::apply(app, &state, &update);
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::ui_ready,
            commands::settings_all,
            commands::settings_set,
            commands::jobs_overview,
            commands::job_pause,
            commands::job_resume,
            commands::job_cancel,
            commands::job_retry,
            commands::job_remove,
            commands::job_set_priority,
            commands::job_move,
            commands::jobs_clear_finished,
            commands::commands_list,
            commands::shell_view,
            commands::shell_run,
            commands::files_plan,
            commands::files_pick,
            commands::recent_list,
            commands::recent_pin,
            commands::recent_remove,
            commands::recent_clear,
            commands::recovery_offer,
            commands::recovery_restore,
            commands::recovery_discard,
            commands::path_open,
            commands::path_reveal,
            commands::dev_sample_job,
        ])
        .build(tauri::generate_context!());

    match app {
        Ok(app) => app.run(|handle, event| {
            if let RunEvent::Exit = event
                && let Some(state) = handle.try_state::<AppState>()
            {
                state.shut_down();
            }
        }),
        Err(error) => {
            tracing::error!(%error, "vellum could not start");
            std::process::exit(1);
        }
    }
}

fn init_logging() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_env("VELLUM_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}
