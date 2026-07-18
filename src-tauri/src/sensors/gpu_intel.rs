// Intel GPU sensor implementation using sysfs (i915/Xe kernel drivers) + intel_gpu_top.
// Supports Intel integrated graphics (i915) and newer Xe discrete/integrated GPUs.

#[cfg(target_os = "linux")]
use crate::sensors::{GpuReading, GpuSensor};
#[cfg(target_os = "linux")]
use anyhow::{Context, Result};
#[cfg(target_os = "linux")]
use serde::Deserialize;
#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::Command;

/// Intel GPU sensor using sysfs + intel_gpu_top for usage.
/// Each instance wraps a single DRM card device and its associated hwmon directory.
#[cfg(target_os = "linux")]
pub struct IntelGpuSensor {
    card_path: PathBuf,
    device_path: PathBuf,
    hwmon_path: Option<PathBuf>,
    cached_name: String,
}

#[cfg(target_os = "linux")]
impl IntelGpuSensor {
    /// Probe all Intel GPUs by scanning `/sys/class/drm/card*`.
    /// Returns a sensor for each card with vendor ID 0x8086 (Intel).
    pub fn probe() -> Vec<Self> {
        let drm_path = Path::new("/sys/class/drm");
        let mut sensors = Vec::new();

        let entries = match fs::read_dir(drm_path) {
            Ok(e) => e,
            Err(_) => return sensors,
        };

        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let name_str = file_name.to_string_lossy();

            // Only consider bare "cardN" directories, skip "cardN-*" connector entries
            if !name_str.starts_with("card") || name_str.contains('-') {
                continue;
            }

            let card_path = entry.path();
            let device_path = card_path.join("device");

            // Verify this is an Intel device (vendor ID 0x8086)
            if !is_intel_device(&device_path) {
                continue;
            }

            // Find associated hwmon directory for temperature readings
            let hwmon_path = find_hwmon_dir(&device_path);

            // Determine a human-readable name
            let name = get_device_name(&device_path, &name_str);

            sensors.push(Self {
                card_path,
                device_path,
                hwmon_path,
                cached_name: name,
            });
        }

        sensors
    }

    /// Read a sysfs file as a string, returning None if missing/unreadable.
    fn read_sysfs_string(path: &Path) -> Option<String> {
        fs::read_to_string(path).ok().map(|s| s.trim().to_string())
    }

    /// Read GPU usage via intel_gpu_top JSON output.
    /// Returns None if intel_gpu_top is not available, permission denied, or parsing fails.
    fn read_usage_via_intel_gpu_top() -> Option<f32> {
        let output = Command::new("intel_gpu_top")
            .args(["-J", "-s", "1000", "-o", "-"])
            .output()
            .ok()?;

        if !output.status.success() {
            // Permission denied (CAP_PERFMON) or other error — treat as unavailable
            return None;
        }

        let stdout = String::from_utf8(output.stdout).ok()?;
        let parsed: IntelGpuTopOutput = serde_json::from_str(&stdout).ok()?;

        // intel_gpu_top JSON structure has engines array, find render/3D engine
        // The "busy" field is a percentage (0-100)
        parsed.engines.into_iter()
            .find(|e| e.class == "render" || e.class == "3d" || e.name.to_lowercase().contains("render"))
            .map(|e| e.busy as f32)
            .or_else(|| parsed.engines.first().map(|e| e.busy as f32))
    }
}

#[cfg(target_os = "linux")]
impl GpuSensor for IntelGpuSensor {
    fn read(&self) -> Result<GpuReading> {
        // Usage: via intel_gpu_top (requires root or CAP_PERFMON)
        let usage_percent = Self::read_usage_via_intel_gpu_top();

        // Temperature: read hwmon/temp1_input (millidegrees Celsius)
        // Intel iGPU often shares CPU thermal zone or has its own hwmon.
        // If absent, leave as None (very common — Intel temp reporting is inconsistent).
        let temp_celsius = self.hwmon_path.as_ref().and_then(|hwmon| {
            Self::read_sysfs_string(&hwmon.join("temp1_input"))
                .and_then(|s| s.parse::<u64>().ok())
                .map(|v| v as f32 / 1000.0)
        });

        // VRAM: Intel iGPU shares system RAM — no separate VRAM concept applies.
        // Discrete Intel GPUs (Arc) may have dedicated VRAM but the i915/Xe
        // sysfs interface doesn't expose mem_info_vram_* like amdgpu does.
        // Return None for both fields.
        let vram_used_mb: Option<u32> = None;
        let vram_total_mb: Option<u32> = None;

        // If the device directory itself is gone, that's a hard error (device removed/driver unloaded)
        if !self.device_path.exists() {
            anyhow::bail!("Intel GPU device path vanished: {:?}", self.device_path);
        }

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

#[cfg(target_os = "linux")]
/// Check if the device at `device_path` has Intel vendor ID (0x8086).
fn is_intel_device(device_path: &Path) -> bool {
    let vendor_path = device_path.join("vendor");
    match fs::read_to_string(&vendor_path) {
        Ok(s) => s.trim() == "0x8086",
        Err(_) => false,
    }
}

#[cfg(target_os = "linux")]
/// Find the hwmon directory under `device/hwmon/` for temperature readings.
/// Returns the first hwmon* directory found, or None if not present.
fn find_hwmon_dir(device_path: &Path) -> Option<PathBuf> {
    let hwmon_root = device_path.join("hwmon");
    let entries = fs::read_dir(&hwmon_root).ok()?;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with("hwmon") {
            return Some(entry.path());
        }
    }
    None
}

#[cfg(target_os = "linux")]
/// Get a human-readable device name.
/// Priority: device/product_name -> parse device/uevent for PCI_ID -> fallback "Intel Graphics (cardN)"
fn get_device_name(device_path: &Path, card_name: &str) -> String {
    // Try product_name first (available on some newer kernels)
    if let Some(product_name) = IntelGpuSensor::read_sysfs_string(&device_path.join("product_name")) {
        if !product_name.is_empty() {
            return product_name;
        }
    }

    // Parse uevent for PCI_ID (format: PCI_ID=8086:XXXX)
    if let Some(uevent) = IntelGpuSensor::read_sysfs_string(&device_path.join("uevent")) {
        for line in uevent.lines() {
            if let Some(pci_id) = line.strip_prefix("PCI_ID=") {
                // pci_id format: "8086:46a6" (vendor:device)
                return format!("Intel GPU ({})", pci_id);
            }
        }
    }

    // Fallback
    format!("Intel Graphics ({})", card_name)
}

/// JSON structure for intel_gpu_top -J output.
#[cfg(target_os = "linux")]
#[derive(Debug, Deserialize)]
struct IntelGpuTopOutput {
    engines: Vec<IntelGpuEngine>,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Deserialize)]
struct IntelGpuEngine {
    name: String,
    class: String,
    busy: f64,
}

// The struct only contains PathBuf, Option<PathBuf>, and String — all Send + Sync.
#[cfg(target_os = "linux")]
unsafe impl Send for IntelGpuSensor {}
#[cfg(target_os = "linux")]
unsafe impl Sync for IntelGpuSensor {}