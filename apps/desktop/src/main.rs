// keeps a console window from opening next to the app in windows release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    vellum_app::run()
}
