//! the tauri side of vellum. it starts the core services, forwards events to the ui,
//! exposes ipc commands and manages windows. the real work lives in the `vellum-*` crates
//! so it can also run from the cli and in tests without a window.

mod bridge;
mod image;
mod ipc;
mod render;
mod state;
mod tools;

use tauri::{Manager, RunEvent, WindowEvent};
use tracing_subscriber::EnvFilter;
use vellum_core::documents::MAIN_WINDOW;

pub use state::AppState;

pub fn run() {
    init_logging();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let state = AppState::init(app.handle())?;
            app.manage(state);
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // closing the main window quits, like every other editor
            WindowEvent::CloseRequested { .. } if window.label() == MAIN_WINDOW => {
                window.app_handle().exit(0);
            }
            // documents in a closed window go back to the main one, nothing is lost
            WindowEvent::Destroyed if window.label() != MAIN_WINDOW => {
                if let Some(state) = window.try_state::<AppState>() {
                    state.services.documents.window_closed(window.label());
                }
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            ipc::app_info,
            ipc::get_settings,
            ipc::update_settings,
            ipc::reset_settings,
            ipc::list_commands,
            ipc::run_command,
            ipc::get_keymap,
            ipc::list_documents,
            ipc::open_files,
            ipc::close_document,
            ipc::move_document,
            ipc::tear_off_document,
            ipc::document_command,
            ipc::seal_document,
            ipc::get_document,
            ipc::document_history,
            ipc::checkout_history,
            ipc::create_snapshot,
            ipc::list_recent,
            ipc::pin_recent,
            ipc::remove_recent,
            ipc::recovery_offer,
            ipc::restore_session,
            ipc::discard_recovery,
            ipc::autosave_status,
            ipc::list_notifications,
            ipc::dismiss_notification,
            ipc::open_result,
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
            ipc::media_capabilities,
            ipc::probe_media,
            ipc::media_thumbnail,
            ipc::media_filmstrip,
            ipc::media_waveform,
            ipc::waveform_overview,
            ipc::strip_metadata,
            ipc::media_cache_stats,
            ipc::clean_media_cache,
            render::open_view,
            render::view_frame,
            render::close_view,
            render::render_status,
            image::image_new,
            image::image_command,
            image::image_stroke_begin,
            image::image_stroke,
            image::image_stroke_end,
            image::image_stroke_cancel,
            image::image_preview,
            image::image_outline,
            image::image_magnetic_begin,
            image::image_magnetic,
            image::image_magnetic_end,
            image::image_pick_color,
            image::image_thumbnails,
            image::image_save_as,
            image::image_transform_box,
            image::image_transform,
            image::image_export,
            tools::run_quick_tool,
            tools::size_plan,
            tools::anim_estimate,
            tools::gif_frames,
            tools::convert_targets,
            tools::rename_plan,
            tools::media_frame,
            tools::preview_file,
            tools::copy_files,
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
            // cancel jobs so nothing is left half written, then the last autosave
            if let Some(state) = handle.try_state::<AppState>() {
                state.services.shutdown();
            }
        }
    });
}

fn init_logging() {
    let filter = EnvFilter::try_from_env("VELLUM_LOG").unwrap_or_else(|_| {
        EnvFilter::new("info,vellum_core=debug,vellum_media=debug,vellum_desktop=debug")
    });
    // a second init in the same process (tests) is harmless, so the error is ignored
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}
