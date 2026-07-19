#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod host;
mod ipc;
mod sensors;

use ipc::AppState;
use std::sync::Mutex;

fn main() {
    // Check for the --mock flag before anything else.
    // Usage:  cargo tauri dev -- -- --mock
    //         cargo run -- --mock
    let use_mock = std::env::args().any(|a| a == "--mock");

    let backends = if use_mock {
        sensors::mock::mock_backends()
    } else {
        sensors::detect_backends()
    };

    let app_state = AppState {
        backends: Mutex::new(backends),
    };

    tauri::Builder::default()
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            ipc::get_snapshot,
            ipc::get_host_info,
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
