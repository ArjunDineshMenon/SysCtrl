#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ipc;
mod sensors;

use ipc::AppState;
use std::sync::Mutex;

fn main() {
    let backends = sensors::detect_backends();

    let app_state = AppState {
        backends: Mutex::new(backends),
    };

    tauri::Builder::default()
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            ipc::get_snapshot,
            ipc::set_fan_speed,
        ])
        .setup(|app| {
            // Start the 1 Hz background loop that pushes snapshots to all
            // webview windows via the "sysctrl://snapshot" event.
            ipc::start_polling_loop(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}