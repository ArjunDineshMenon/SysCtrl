use tauri::{AppHandle, Emitter, Manager, Runtime};
use crate::sensors::{detect_backends, SensorBackend, SystemSnapshot, FanInfo};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::interval;

#[tauri::command]
fn get_snapshot(state: tauri::State<Arc<Mutex<SensorState>>>) -> Result<SystemSnapshot, String> {
    let state = state.lock().map_err(|e| e.to_string())?;
    Ok(state.last_snapshot.clone())
}

#[tauri::command]
fn set_fan_pwm(state: tauri::State<Arc<Mutex<SensorState>>>, fan_index: usize, pwm: u8) -> Result<(), String> {
    let mut state = state.lock().map_err(|e| e.to_string())?;
    state.set_fan_pwm(fan_index, pwm)
}

#[tauri::command]
fn get_fans(state: tauri::State<Arc<Mutex<SensorState>>>) -> Result<Vec<FanInfo>, String> {
    let state = state.lock().map_err(|e| e.to_string())?;
    Ok(state.last_snapshot.fans.clone())
}

struct SensorState {
    backends: Vec<Box<dyn SensorBackend>>,
    last_snapshot: SystemSnapshot,
}

impl SensorState {
    fn new() -> Self {
        let backends = detect_backends();
        Self {
            backends,
            last_snapshot: SystemSnapshot {
                cpu: crate::sensors::CpuSample { usage_percent: 0.0, temp_celsius: None, freq_mhz: None },
                gpus: Vec::new(),
                fans: Vec::new(),
                timestamp_ms: 0,
            },
        }
    }

    fn set_fan_pwm(&mut self, fan_index: usize, pwm: u8) -> Result<(), String> {
        if let Some(backend) = self.backends.get_mut(fan_index) {
            backend.set_fan_pwm(fan_index, pwm)
        } else {
            Err("Invalid fan index".into())
        }
    }

    fn poll(&mut self) {
        let mut cpu = None;
        let mut gpus = Vec::new();
        let mut fans = Vec::new();

        for backend in &mut self.backends {
            if cpu.is_none() {
                cpu = backend.sample_cpu();
            }
            gpus.extend(backend.sample_gpus());
            fans.extend(backend.sample_fans());
        }

        self.last_snapshot = SystemSnapshot {
            cpu: cpu.unwrap_or(crate::sensors::CpuSample { usage_percent: 0.0, temp_celsius: None, freq_mhz: None }),
            gpus,
            fans,
            timestamp_ms: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64,
        };
    }
}

pub fn start_polling_loop<R: Runtime>(app: AppHandle<R>) {
    let state = app.state::<Arc<Mutex<SensorState>>>();
    let app_handle = app.clone();

    tokio::spawn(async move {
        let mut interval = interval(Duration::from_millis(1000));
        loop {
            interval.tick().await;
            let mut state_guard = state.lock().unwrap();
            state_guard.poll();
            let snapshot = state_guard.last_snapshot.clone();
            drop(state_guard);
            let _ = app_handle.emit("sysctrl://snapshot", snapshot);
        }
    });
}

pub fn init<R: Runtime>(app: &mut tauri::App<R>) -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(Mutex::new(SensorState::new()));
    app.manage(state.clone());
    start_polling_loop(app.app_handle().clone());
    Ok(())
}