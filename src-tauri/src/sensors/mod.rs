mod cpu;
mod gpu_nvidia;
mod gpu_amd;
mod gpu_intel;
mod gpu_apple;
mod fan;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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

struct StubCpuSensor;
impl CpuSensor for StubCpuSensor {
    fn read(&self) -> Result<CpuReading> {
        Ok(CpuReading {
            usage_percent: 0.0,
            temp_celsius: None,
            freq_mhz: None,
            core_count: 0,
        })
    }
}

struct StubGpuSensor {
    name: String,
}
impl GpuSensor for StubGpuSensor {
    fn read(&self) -> Result<GpuReading> {
        Ok(GpuReading {
            name: self.name.clone(),
            usage_percent: None,
            temp_celsius: None,
            vram_used_mb: None,
            vram_total_mb: None,
        })
    }
    fn name(&self) -> &str {
        &self.name
    }
}

struct StubFanController;
impl FanController for StubFanController {
    fn read_all(&self) -> Result<Vec<FanReading>> {
        Ok(vec![])
    }
    fn set_percent(&self, _fan_label: &str, _percent: u8) -> Result<()> {
        Ok(())
    }
}

pub fn detect_backends() -> DetectedBackends {
    // TODO(stage-2): real detection
    // - probe nvidia-smi for NVIDIA GPUs
    // - probe /sys/class/drm/card* for AMD/Intel GPUs
    // - probe hwmon paths for fans and CPU temps
    // - use sysinfo for CPU core count / usage
    // - use powermetrics on macOS for Apple Silicon GPU
    DetectedBackends {
        cpu: Box::new(StubCpuSensor),
        gpus: vec![],
        fan: Box::new(StubFanController),
    }
}