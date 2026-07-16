// AMD GPU sensor implementation using pure sysfs reads (amdgpu kernel driver).
// Supports discrete AMD GPUs and Ryzen APU integrated graphics.

use crate::sensors::{GpuReading, GpuSensor};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// AMD GPU sensor using sysfs (amdgpu driver).
/// Each instance wraps a single DRM card device and its associated hwmon directory.
pub struct AmdGpuSensor {
    card_path: PathBuf,
    hwmon_path: Option<PathBuf>,
    cached_name: String,
}

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

    /// Read a sysfs file as u32, returning None if missing/unreadable/invalid.
    fn read_sysfs_u32(path: &Path) -> Option<u32> {
        Self::read_sysfs_u64(path).map(|v| v as u32)
    }
}

impl GpuSensor for AmdGpuSensor {
    fn read(&self) -> Result<GpuReading> {
        let device_path = self.card_path.join("device");

        // GPU usage percentage (0-100) from gpu_busy_percent
        let usage_percent = Self::read_sysfs_u32(&device_path.join("gpu_busy_percent"))
            .map(|v| v as f32);

        // Temperature from hwmon (millidegrees Celsius -> degrees Celsius)
        let temp_celsius = self.hwmon_path.as_ref().and_then(|hwmon| {
            Self::read_sysfs_u64(&hwmon.join("temp1_input")).map(|v| v as f32 / 1000.0)
        });

        // VRAM usage (bytes -> MB)
        let vram_used_mb = Self::read_sysfs_u64(&device_path.join("mem_info_vram_used"))
            .map(|v| (v / 1024 / 1024) as u32);
        let vram_total_mb = Self::read_sysfs_u64(&device_path.join("mem_info_vram_total"))
            .map(|v| (v / 1024 / 1024) as u32);

        // If the device directory itself is gone, that's a hard error (device unplugged/driver unloaded)
        if !device_path.exists() {
            anyhow::bail!("AMD GPU device path vanished: {:?}", device_path);
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

/// Check if the device at `device_path` has AMD vendor ID (0x1002).
fn is_amd_device(device_path: &Path) -> bool {
    let vendor_path = device_path.join("vendor");
    match fs::read_to_string(&vendor_path) {
        Ok(s) => s.trim() == "0x1002",
        Err(_) => false,
    }
}

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

/// Get a human-readable device name.
/// Priority: product_name file -> parse uevent for PCI_ID -> fallback "AMD GPU (cardN)"
fn get_device_name(device_path: &Path, card_name: &str) -> String {
    // 1. Try product_name (available on newer kernels / some devices)
    if let Some(name) = AmdGpuSensor::read_sysfs_string(&device_path.join("product_name")) {
        if !name.is_empty() {
            return name;
        }
    }

    // 2. Parse uevent for PCI_ID (format: PCI_ID=1002:XXXX)
    if let Some(uevent) = AmdGpuSensor::read_sysfs_string(&device_path.join("uevent")) {
        for line in uevent.lines() {
            if let Some(pci_id) = line.strip_prefix("PCI_ID=") {
                // pci_id format: "1002:731f" (vendor:device)
                return format!("AMD GPU ({})", pci_id);
            }
        }
    }

    // 3. Fallback
    format!("AMD GPU ({})", card_name)
}

// NVML is not used here; this is pure sysfs. The struct is Send + Sync because
// it only contains PathBuf and String (both Send + Sync).
unsafe impl Send for AmdGpuSensor {}
unsafe impl Sync for AmdGpuSensor {}