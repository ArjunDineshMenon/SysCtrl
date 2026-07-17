mod cpu;
mod gpu_nvidia;
mod gpu_amd;
mod gpu_intel;
mod gpu_apple;
mod fan;

use crate::sensors::cpu::CpuSensorImpl;
use crate::sensors::fan::HwmonFanController;
use crate::sensors::gpu_amd::AmdGpuSensor;
use crate::sensors::gpu_apple::AppleGpuSensor;
use crate::sensors::gpu_intel::IntelGpuSensor;
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

pub fn detect_backends() -> DetectedBackends {
    // Probe all GPU backends
    let mut gpus: Vec<Box<dyn GpuSensor>> = Vec::new();

    // NVIDIA (NVML)
    for s in NvidiaGpuSensor::probe() {
        gpus.push(Box::new(s));
    }

    // AMD (amdgpu sysfs)
    for s in AmdGpuSensor::probe() {
        gpus.push(Box::new(s));
    }

    // Intel (i915/Xe sysfs + intel_gpu_top)
    for s in IntelGpuSensor::probe() {
        gpus.push(Box::new(s));
    }

    // Apple Silicon (powermetrics on macOS, sysfs on Asahi Linux)
    for s in AppleGpuSensor::probe() {
        gpus.push(Box::new(s));
    }

    // CPU sensor (sysinfo)
    let cpu = Box::new(CpuSensorImpl::new()) as Box<dyn CpuSensor>;

    // Fan controller (hwmon sysfs)
    let fan = Box::new(HwmonFanController::probe()) as Box<dyn FanController>;

    DetectedBackends { cpu, gpus, fan }
}
