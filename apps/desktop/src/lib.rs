//! the tauri side of vellum: owns the core services and exposes them to the ui as commands.

mod commands;
mod state;

use tauri::{Emitter, Manager};
use vellum_core::events::UI_EVENT_CHANNEL;

pub fn run() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let state = state::AppState::load(config_dir.join("settings.json"))?;

            // every core event goes to the webview on one channel, the ui switches on `topic`
            let handle = app.handle().clone();
            state.bus.on(move |event| {
                if let Err(e) = handle.emit(UI_EVENT_CHANNEL, event) {
                    eprintln!("vellum: could not send event to the ui: {e}");
                }
            });

            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::get_settings,
            commands::update_settings,
            commands::list_jobs,
            commands::cancel_job,
            commands::pause_job,
            commands::resume_job,
            commands::retry_job,
            commands::remove_job,
            commands::clear_finished_jobs,
            commands::start_sample_job,
        ])
        .build(tauri::generate_context!());

    match app {
        Ok(app) => app.run(|handle, event| {
            if let tauri::RunEvent::Exit = event {
                // let running jobs stop cleanly before the process goes away
                handle.state::<state::AppState>().jobs.shutdown();
            }
        }),
        Err(e) => {
            eprintln!("vellum failed to start: {e}");
            std::process::exit(1);
        }
    }
}
