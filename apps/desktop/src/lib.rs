//! the tauri side of vellum. it owns the core services (event bus, settings, job queue),
//! forwards events to the ui and exposes a small set of ipc commands. the real work lives in
//! the `vellum-*` crates so it can also run from the cli and in tests without a window.

mod bridge;
mod ipc;
mod state;

use tauri::{Manager, RunEvent};
use tracing_subscriber::EnvFilter;

pub use state::AppState;

pub fn run() {
    init_logging();

    let app = tauri::Builder::default()
        .setup(|app| {
            let state = AppState::init(app.handle())?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::app_info,
            ipc::get_settings,
            ipc::update_settings,
            ipc::reset_settings,
            ipc::list_jobs,
            ipc::cancel_job,
            ipc::pause_job,
            ipc::resume_job,
            ipc::retry_job,
            ipc::remove_job,
            ipc::clear_finished_jobs,
            ipc::set_job_priority,
            ipc::move_job,
            ipc::start_sample_job,
            ipc::ui_event,
        ])
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(err) => {
            tracing::error!(%err, "vellum could not start");
            std::process::exit(1);
        }
    };

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            // ask running jobs to stop so they do not leave half written files behind
            if let Some(state) = handle.try_state::<AppState>() {
                state.jobs.cancel_all();
            }
        }
    });
}

fn init_logging() {
    let filter = EnvFilter::try_from_env("VELLUM_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,vellum_core=debug,vellum_desktop=debug"));
    // a second init in the same process (tests) is harmless, so the error is ignored
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}
