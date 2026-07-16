// NVIDIA GPU sensor implementation using nvml-wrapper
// Provides multi-GPU support with cached device names and graceful NVML init failure handling.

use crate::sensors::{GpuReading, GpuSensor};
use anyhow::{Context, Result};
use nvml_wrapper::Nvml;
use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
use std::sync::OnceLock;

/// NVIDIA GPU sensor using NVML (NVIDIA Management Library).
/// Each instance wraps a single GPU device and caches its name at probe time.
pub struct NvidiaGpuSensor {
    nvml: Nvml,
    device_index: u32,
    cached_name: String,
}

impl NvidiaGpuSensor {
    /// Probe all available NVIDIA GPUs and return a sensor for each.
    /// Returns empty Vec if NVML initialization fails (e.g., no NVIDIA driver).
    pub fn probe() -> Vec<Self> {
        // Initialize NVML once. If it fails (no driver, no GPU, etc.), return empty vec.
        let nvml = match Nvml::init() {
            Ok(n) => n,
            Err(_) => return vec![],
        };

        let device_count = match nvml.device_count() {
            Ok(c) => c,
            Err(_) => return vec![],
        };

        let mut sensors = Vec::with_capacity(device_count as usize);

        for i in 0..device_count {
            let device = match nvml.device_by_index(i) {
                Ok(d) => d,
                Err(_) => continue,
            };

            let name = device
                .name()
                .unwrap_or_else(|_| format!("NVIDIA GPU {}", i));

            sensors.push(Self {
                nvml: nvml.clone(),
                device_index: i,
                cached_name: name,
            });
        }

        sensors
    }
}

impl GpuSensor for NvidiaGpuSensor {
    fn read(&self) -> Result<GpuReading> {
        let device = self
            .nvml
            .device_by_index(self.device_index)
            .context("failed to get device by index")?;

        // GPU utilization (percentage)
        let utilization = device
            .utilization_rates()
            .context("failed to read utilization rates")?;
        let usage_percent = Some(utilization.gpu as f32);

        // GPU temperature (Celsius)
        let temp_celsius = device
            .temperature(TemperatureSensor::Gpu)
            .ok()
            .map(|t| t as f32);

        // VRAM info (bytes -> MB)
        let mem_info = device
            .memory_info()
            .context("failed to read memory info")?;
        let vram_used_mb = Some((mem_info.used / 1024 / 1024) as u32);
        let vram_total_mb = Some((mem_info.total / 1024 / 1024) as u32);

        Ok(GpuReading {
            name: self.cached_name.clone(),
            usage_percent,
            temp_celsius,
            vram_used_mb,
            vram_total_mb,
        })
    }

    fn name(&self) -> &str {
        &self.cached_name
    }
}

// NVML is thread-safe and can be cloned cheaply (Arc internally).
// We implement Send + Sync explicitly for clarity.
unsafe impl Send for NvidiaGpuSensor {}
unsafe impl Sync for NvidiaGpuSensor {}