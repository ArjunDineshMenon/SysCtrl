#[cfg(target_os = "linux")]
mod cpu;
#[cfg(target_os = "linux")]
mod gpu_nvidia;
#[cfg(target_os = "linux")]
mod gpu_amd;
#[cfg(target_os = "linux")]
mod gpu_intel;
#[cfg(target_os = "linux")]
mod gpu_apple;
#[cfg(target_os = "linux")]
mod fan;

#[cfg(target_os = "linux")]
use crate::sensors::cpu::CpuSensorImpl;
#[cfg(target_os = "linux")]
use crate::sensors::fan::SysfsFanController;
#[cfg(target_os = "linux")]
use crate::sensors::gpu_amd::AmdGpuSensor;
#[cfg(target_os = "linux")]
use crate::sensors::gpu_apple::AppleGpuSensor;
#[cfg(target_os = "linux")]
use crate::sensors::gpu_intel::IntelGpuSensor;
#[cfg(target_os = "linux")]
use crate::sensors::gpu_nvidia::NvidiaGpuSensor;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuReading {
    pub usage_percent: f32,
    pub temp_celsius: Option<f32>,
    pub freq_mhz: Option<u32>,
    pub core_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuReading {
    pub name: String,
    pub usage_percent: Option<f32>,
    pub temp_celsius: Option<f32>,
    pub vram_used_mb: Option<u32>,
    pub vram_total_mb: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FanReading {
    pub label: String,
    pub rpm: Option<u32>,
    pub percent: Option<u8>,
    pub writable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemSnapshot {
    pub cpu: CpuReading,
    pub gpus: Vec<GpuReading>,
    pub fans: Vec<FanReading>,
}

pub trait CpuSensor: Send + Sync {
    fn read(&self) -> Result<CpuReading>;
}

pub trait GpuSensor: Send + Sync {
    fn read(&self) -> Result<GpuReading>;
    fn name(&self) -> &str;
}

pub trait FanController: Send + Sync {
    fn read_all(&self) -> Result<Vec<FanReading>>;
    fn set_percent(&self, fan_label: &str, percent: u8) -> Result<()>;
}

pub struct DetectedBackends {
    pub cpu: Box<dyn CpuSensor>,
    pub gpus: Vec<Box<dyn GpuSensor>>,
    pub fan: Box<dyn FanController>,
}

/// No-op CPU sensor for non-Linux platforms (Windows, macOS).
/// Returns zeroed readings so the app compiles and runs without sensor data.
#[cfg(not(target_os = "linux"))]
pub struct NoopCpuSensor;

#[cfg(not(target_os = "linux"))]
impl CpuSensor for NoopCpuSensor {
    fn read(&self) -> Result<CpuReading> {
        Ok(CpuReading {
            usage_percent: 0.0,
            temp_celsius: None,
            freq_mhz: None,
            core_count: 1,
        })
    }
}

/// No-op fan controller for non-Linux platforms.
/// Returns empty fan list and ignores set_percent calls.
#[cfg(not(target_os = "linux"))]
pub struct NoopFanController;

#[cfg(not(target_os = "linux"))]
impl FanController for NoopFanController {
    fn read_all(&self) -> Result<Vec<FanReading>> {
        Ok(vec![])
    }

    fn set_percent(&self, _fan_label: &str, _percent: u8) -> Result<()> {
        Ok(())
    }
}

pub fn detect_backends() -> DetectedBackends {
    // ── GPU probing ──────────────────────────────────────────────────────
    // Each backend's probe() returns Vec<Self>; a machine can have multiple
    // GPUs across vendors (e.g. Intel iGPU + NVIDIA dGPU), so we chain
    // everything into one flat list.
    let gpus: Vec<Box<dyn GpuSensor>> = Vec::new();

    #[cfg(target_os = "linux")]
    {
        for s in NvidiaGpuSensor::probe() {
            gpus.push(Box::new(s));
        }
        for s in AmdGpuSensor::probe() {
            gpus.push(Box::new(s));
        }
        for s in IntelGpuSensor::probe() {
            gpus.push(Box::new(s));
        }
        for s in AppleGpuSensor::probe() {
            gpus.push(Box::new(s));
        }

        // Startup sanity-check log (goes to stderr so it's visible in the
        // terminal when running `cargo tauri dev` but never reaches the UI).
        eprintln!("[SysCtrl] detected {} GPU(s):", gpus.len());
        for (i, gpu) in gpus.iter().enumerate() {
            eprintln!("[SysCtrl]   GPU {}: {}", i, gpu.name());
        }
    }

    // ── CPU probing ──────────────────────────────────────────────────────
    #[cfg(target_os = "linux")]
    let cpu = Box::new(CpuSensorImpl::new()) as Box<dyn CpuSensor>;

    #[cfg(not(target_os = "linux"))]
    let cpu = Box::new(NoopCpuSensor) as Box<dyn CpuSensor>;

    // ── Fan probing ──────────────────────────────────────────────────────
    #[cfg(target_os = "linux")]
    let fan = Box::new(SysfsFanController::probe()) as Box<dyn FanController>;

    #[cfg(not(target_os = "linux"))]
    let fan = Box::new(NoopFanController) as Box<dyn FanController>;

    DetectedBackends { cpu, gpus, fan }
}