#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    if let Some(code) = vidlib_tauri::metadata::property_helper_main() {
        std::process::exit(code);
    }
    vidlib_tauri::run();
}
