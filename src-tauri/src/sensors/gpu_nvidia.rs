// NVIDIA GPU sensor implementation using nvml-wrapper
// Provides multi-GPU support with cached device names and graceful NVML init failure handling.

#[cfg(target_os = "linux")]
use crate::sensors::{GpuReading, GpuSensor};
#[cfg(target_os = "linux")]
use anyhow::{Context, Result};
#[cfg(target_os = "linux")]
use nvml_wrapper::Nvml;
#[cfg(target_os = "linux")]
use nvml_wrapper::enum_wrappers::device::TemperatureSensor;

/// NVIDIA GPU sensor using NVML (NVIDIA Management Library).
/// Each instance wraps a single GPU device and caches its name at probe time.
#[cfg(target_os = "linux")]
pub struct NvidiaGpuSensor {
    nvml: std::sync::Arc<Nvml>,
    device_index: u32,
    cached_name: String,
}

#[cfg(target_os = "linux")]
impl NvidiaGpuSensor {
    /// Probe all available NVIDIA GPUs and return a sensor for each.
    /// Returns empty Vec if NVML initialization fails (e.g., no NVIDIA driver).
    pub fn probe() -> Vec<Self> {
        let nvml = match Nvml::init() {
            Ok(n) => std::sync::Arc::new(n),
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

#[cfg(target_os = "linux")]
impl GpuSensor for NvidiaGpuSensor {
    fn read(&self) -> Result<GpuReading> {
        // Only the device handle lookup itself is treated as a hard failure —
        // if the device vanished entirely (unplugged, driver unload), the
        // whole read legitimately fails. Everything after this degrades
        // field-by-field instead of failing the whole struct.
        let device = self
            .nvml
            .device_by_index(self.device_index)
            .context("failed to get device by index")?;

        // GPU utilization (percentage) — degrade to None on failure.
        let usage_percent = device
            .utilization_rates()
            .ok()
            .map(|u| u.gpu as f32);

        // GPU temperature (Celsius) — degrade to None on failure.
        let temp_celsius = device
            .temperature(TemperatureSensor::Gpu)
            .ok()
            .map(|t| t as f32);

        // VRAM info (bytes -> MB) — degrade to None on failure.
        let (vram_used_mb, vram_total_mb) = match device.memory_info() {
            Ok(mem_info) => (
                Some((mem_info.used / 1024 / 1024) as u32),
                Some((mem_info.total / 1024 / 1024) as u32),
            ),
            Err(_) => (None, None),
        };

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