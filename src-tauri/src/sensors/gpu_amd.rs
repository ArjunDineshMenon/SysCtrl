// AMD GPU sensor implementation using pure sysfs reads (amdgpu kernel driver).
// Supports discrete AMD GPUs and Ryzen APU integrated graphics.

use crate::sensors::{GpuReading, GpuSensor};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
/// AMD GPU sensor using sysfs (amdgpu driver).
/// Each instance wraps a single DRM card device and its associated hwmon directory.
pub struct AmdGpuSensor {
    card_path: PathBuf,
    device_path: PathBuf,
    hwmon_path: Option<PathBuf>,
    cached_name: String,
}

#[cfg(target_os = "linux")]
impl AmdGpuSensor {
    /// Probe all AMD GPUs by scanning `/sys/class/drm/card*`.
    /// Returns a sensor for each card with vendor ID 0x1002 (AMD).
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

            // Verify this is an AMD device (vendor ID 0x1002)
            if !is_amd_device(&device_path) {
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

    /// Read a sysfs file as u64, returning None if missing/unreadable/invalid.
    fn read_sysfs_u64(path: &Path) -> Option<u64> {
        Self::read_sysfs_string(path)?.parse().ok()
    }
}

#[cfg(target_os = "linux")]
impl GpuSensor for AmdGpuSensor {
    fn read(&self) -> Result<GpuReading> {
        // Usage: read device/gpu_busy_percent (integer 0-100)
        let usage_percent = Self::read_sysfs_u64(&self.device_path.join("gpu_busy_percent"))
            .map(|v| v as f32);

        // Temperature: read hwmon/temp1_input (millidegrees Celsius)
        let temp_celsius = self.hwmon_path.as_ref().and_then(|hwmon| {
            Self::read_sysfs_u64(&hwmon.join("temp1_input")).map(|v| v as f32 / 1000.0)
        });

        // VRAM: read device/mem_info_vram_used and device/mem_info_vram_total (bytes -> MB)
        let vram_used_mb = Self::read_sysfs_u64(&self.device_path.join("mem_info_vram_used"))
            .map(|v| (v / 1024 / 1024) as u32);
        let vram_total_mb = Self::read_sysfs_u64(&self.device_path.join("mem_info_vram_total"))
            .map(|v| (v / 1024 / 1024) as u32);

        // If the device directory itself is gone, that's a hard error (device removed/driver unloaded)
        if !self.device_path.exists() {
            anyhow::bail!("AMD GPU device path vanished: {:?}", self.device_path);
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
/// Check if the device at `device_path` has AMD vendor ID (0x1002).
fn is_amd_device(device_path: &Path) -> bool {
    let vendor_path = device_path.join("vendor");
    match fs::read_to_string(&vendor_path) {
        Ok(s) => s.trim() == "0x1002",
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
/// Priority: device/product_name -> parse device/uevent for PCI_ID -> fallback "AMD GPU (cardN)"
fn get_device_name(device_path: &Path, card_name: &str) -> String {
    // Try product_name first (available on some newer kernels)
    if let Some(product_name) = AmdGpuSensor::read_sysfs_string(&device_path.join("product_name")) {
        if !product_name.is_empty() {
            return product_name;
        }
    }

    // Parse uevent for PCI_ID (format: PCI_ID=1002:XXXX)
    if let Some(uevent) = AmdGpuSensor::read_sysfs_string(&device_path.join("uevent")) {
        for line in uevent.lines() {
            if let Some(pci_id) = line.strip_prefix("PCI_ID=") {
                // pci_id format: "1002:731f" (vendor:device)
                return format!("AMD GPU ({})", pci_id);
            }
        }
    }

    // Fallback
    format!("AMD GPU ({})", card_name)
}

// The struct only contains PathBuf, Option<PathBuf>, and String — all Send + Sync.
#[cfg(target_os = "linux")]
unsafe impl Send for AmdGpuSensor {}
#[cfg(target_os = "linux")]
unsafe impl Sync for AmdGpuSensor {}