// IPC command handlers for the Tauri frontend.
//
// Commands exposed:
//   • `get_snapshot`  — on-demand poll of CPU / GPU / fan state.
//   • `get_host_info` — static identity of the current device (hostname, distro, kernel).
//   • `set_fan_speed` — delegate a fan duty-cycle write to the helper binary.
//
// A background polling loop (`start_polling_loop`) continuously pushes
// `SystemSnapshot` events to the frontend at 1 Hz, so the UI can simply
// listen for the "sysctrl://snapshot" event rather than polling.

use crate::host;
use crate::sensors::{self, CpuReading, SystemSnapshot};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

// ---------------------------------------------------------------------------
// Shared application state
// ---------------------------------------------------------------------------

/// Holds the detected sensor backends behind a `std::sync::Mutex`.
///
/// WHY `std::sync::Mutex` instead of `tokio::sync::Mutex`?
///
/// All sensor reads are fast blocking syscalls on sysfs files (a couple of
/// microseconds each).  `std::sync::Mutex` is cheaper than the async variant
/// when the critical section is short and never awaits — it avoids the overhead
/// of wrapping every access in an async block.
///
/// If a backend ever becomes genuinely async (e.g. an NVML call that talks to
/// a daemon), swap this for `tokio::sync::Mutex` so the runtime thread isn't
/// blocked.
pub struct AppState {
    pub backends: Mutex<sensors::DetectedBackends>,
}

// ---------------------------------------------------------------------------
// Snapshot helper (used by both the command and the push loop)
// ---------------------------------------------------------------------------

/// Read all backends and assemble a `SystemSnapshot`.
///
/// Errors from individual backends are logged but swallowed — a dead GPU
/// sensor shouldn't prevent the rest of the snapshot from being delivered.
fn collect_snapshot(backends: &sensors::DetectedBackends) -> SystemSnapshot {
    // CPU
    let cpu = backends.cpu.read().unwrap_or_else(|e| {
        eprintln!("[SysCtrl] CPU read error: {:#}", e);
        CpuReading {
            usage_percent: 0.0,
            temp_celsius: None,
            freq_mhz: None,
            core_count: 0,
        }
    });

    // GPUs
    let mut gpus = Vec::with_capacity(backends.gpus.len());
    for gpu in &backends.gpus {
        match gpu.read() {
            Ok(reading) => gpus.push(reading),
            Err(e) => eprintln!("[SysCtrl] GPU '{}' read error: {:#}", gpu.name(), e),
        }
    }

    // Fans
    let fans = backends.fan.read_all().unwrap_or_else(|e| {
        eprintln!("[SysCtrl] fan read error: {:#}", e);
        Vec::new()
    });

    // Disks
    let disks = backends.disks.read_all().unwrap_or_else(|e| {
        eprintln!("[SysCtrl] disk read error: {:#}", e);
        Vec::new()
    });

    // RAM
    let ram = sensors::read_ram();

    SystemSnapshot { cpu, gpus, fans, ram, disks }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Return a fresh system snapshot to the frontend on demand.
#[tauri::command]
pub async fn get_snapshot(state: tauri::State<'_, AppState>) -> Result<SystemSnapshot, String> {
    let guard = state.backends.lock().map_err(|e| e.to_string())?;
    let snapshot = collect_snapshot(&guard);
    Ok(snapshot)
}

/// Return static identity of the device the app is currently running on
/// (hostname, distribution, kernel, architecture).  Read-only, never fails.
#[tauri::command]
pub async fn get_host_info() -> host::HostInfo {
    host::read_host_info()
}

/// Set a fan's duty cycle (0-100 %) via the privileged helper binary.
///
/// `label` must match one of the labels returned in `FanReading`; `percent`
/// is clamped to 0-100 by the backend.
#[tauri::command]
pub async fn set_fan_speed(
    state: tauri::State<'_, AppState>,
    label: String,
    percent: u8,
) -> Result<(), String> {
    let guard = state.backends.lock().map_err(|e| e.to_string())?;
    guard
        .fan
        .set_percent(&label, percent)
        .map_err(|e| format!("{:#}", e))
}

// ---------------------------------------------------------------------------
// Background push loop
// ---------------------------------------------------------------------------

/// Spawn a tokio task that emits a `SystemSnapshot` every second.
///
/// The frontend can listen for the `"sysctrl://snapshot"` event to receive
/// live updates without polling.
pub fn start_polling_loop(app: &AppHandle) {
    let handle = app.clone();

    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));

        loop {
            interval.tick().await;

            // Grab the lock, collect the snapshot, and release immediately.
            // unwrap_or_else recovers from a poisoned mutex (caused by a
            // panic in another thread while holding the lock). The inner
            // data is still usable — we just keep going.
            let snapshot = {
                let state = handle.state::<AppState>();
                let guard = state.backends.lock().unwrap_or_else(|e| e.into_inner());
                collect_snapshot(&guard)
            };

            // Emit to all webview windows.
            if let Err(e) = handle.emit("sysctrl://snapshot", &snapshot) {
                eprintln!("[SysCtrl] failed to emit snapshot: {}", e);
            }
        }
    });
}
