//! the desktop shell. it creates the window, owns the long lived services
//! (event bus, settings, job queue) and exposes them to the ui as commands.
//! everything with real logic lives in the `vellum-*` crates.

mod bindings;
mod commands;
mod error;
mod state;

use tauri::{Manager, RunEvent};

use crate::state::AppState;

/// the one tauri event every backend event travels on, wrapped in an envelope
pub const EVENT_CHANNEL: &str = "vellum:event";

pub fn run() {
    init_logging();

    let app = tauri::Builder::default()
        .setup(|app| {
            let state = AppState::start(app.handle())?;
            app.manage(state);
            Ok(())
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
            commands::dev_sample_job,
        ])
        .build(tauri::generate_context!());

    match app {
        Ok(app) => app.run(|handle, event| {
            if let RunEvent::Exit = event
                && let Some(state) = handle.try_state::<AppState>()
            {
                state.jobs.shut_down();
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
