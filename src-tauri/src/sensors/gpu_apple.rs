// Apple Silicon GPU sensor for Asahi Linux (open-source asahi/honeykrisp DRM driver).
// Runs on Linux only — not macOS (which uses powermetrics via a separate backend).

#[cfg(target_os = "linux")]
use crate::sensors::{GpuReading, GpuSensor};
#[cfg(target_os = "linux")]
use anyhow::Result;
#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};

/// Apple GPU sensor using sysfs (Asahi DRM driver) + macsmc-hwmon for temperature.
/// Each instance wraps a single DRM card device.
#[cfg(target_os = "linux")]
pub struct AppleGpuSensor {
    card_path: PathBuf,
    device_path: PathBuf,
    hwmon_macsmc_path: Option<PathBuf>,
    cached_name: String,
}

#[cfg(target_os = "linux")]
impl AppleGpuSensor {
    /// Probe all Apple Silicon GPUs by scanning `/sys/class/drm/card*`.
    /// Matches vendor ID 0x106b (Apple's PCI-ish vendor ID in Asahi stack) OR
    /// driver symlink basename "asahi" (secondary check, since support has been
    /// a moving target across kernel versions).
    pub fn probe() -> Vec<Self> {
        let drm_path = Path::new("/sys/class/drm");
        let mut sensors = Vec::new();

        let entries = match fs::read_dir(drm_path) {
            Ok(e) => e,
            Err(_) => return sensors,
        };

        // Pre-scan for macsmc hwmon once (shared across all Apple GPUs)
        let macsmc_hwmon = find_macsmc_hwmon();

        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let name_str = file_name.to_string_lossy();

            // Only consider bare "cardN" directories, skip "cardN-*" connector entries
            if !name_str.starts_with("card") || name_str.contains('-') {
                continue;
            }

            let card_path = entry.path();
            let device_path = card_path.join("device");

            // Check vendor ID first (0x106b = Apple in Asahi stack)
            let is_apple = is_apple_vendor(&device_path) || is_asahi_driver(&device_path);

            if !is_apple {
                continue;
            }

            sensors.push(Self {
                card_path,
                device_path,
                hwmon_macsmc_path: macsmc_hwmon.clone(),
                cached_name: format!("Apple GPU ({})", name_str),
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

    /// Read GPU usage from device/gpu_busy_percent (amdgpu-style).
    /// Asahi driver has added this in some kernel versions but it's not
    /// universally available yet. Return None if absent — best-effort only.
    fn read_usage(&self) -> Option<f32> {
        Self::read_sysfs_u64(&self.device_path.join("gpu_busy_percent"))
            .map(|v| v as f32)
    }

    /// Read GPU temperature from macsmc-hwmon.
    /// Scans for a tempN_label containing "GPU" (case-insensitive), then reads
    /// the corresponding tempN_input. Returns None if no GPU-labeled sensor found.
    fn read_temp(&self) -> Option<f32> {
        let macsmc = self.hwmon_macsmc_path.as_ref()?;

        let entries = fs::read_dir(macsmc).ok()?;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("temp") && name.ends_with("_label") {
                if let Ok(label) = fs::read_to_string(entry.path()) {
                    let label = label.trim().to_lowercase();
                    if label.contains("gpu") {
                        // Found GPU label — read corresponding tempN_input
                        let input_name = name.replace("_label", "_input");
                        let input_path = macsmc.join(input_name);
                        if let Ok(temp_str) = fs::read_to_string(input_path) {
                            if let Ok(temp_milli) = temp_str.trim().parse::<i64>() {
                                return Some(temp_milli as f32 / 1000.0);
                            }
                        }
                    }
                }
            }
        }
        None
    }
}

#[cfg(target_os = "linux")]
impl GpuSensor for AppleGpuSensor {
    fn read(&self) -> Result<GpuReading> {
        // Usage: best-effort via gpu_busy_percent (may be absent)
        let usage_percent = self.read_usage();

        // Temperature: from macsmc-hwmon with GPU label match
        let temp_celsius = self.read_temp();

        // VRAM: Apple Silicon uses unified memory — no separate VRAM concept.
        // The Asahi driver does not expose dedicated VRAM counters via sysfs.
        let vram_used_mb: Option<u32> = None;
        let vram_total_mb: Option<u32> = None;

        // If the device directory itself is gone, that's a hard error (device removed/driver unloaded)
        if !self.device_path.exists() {
            anyhow::bail!("Apple GPU device path vanished: {:?}", self.device_path);
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
/// Check if device has Apple vendor ID (0x106b) in the Asahi stack.
fn is_apple_vendor(device_path: &Path) -> bool {
    let vendor_path = device_path.join("vendor");
    match fs::read_to_string(&vendor_path) {
        Ok(s) => s.trim() == "0x106b",
        Err(_) => false,
    }
}

#[cfg(target_os = "linux")]
/// Check if device's driver symlink points to "asahi" driver.
/// Secondary check since vendor ID support has been inconsistent across kernels.
fn is_asahi_driver(device_path: &Path) -> bool {
    let driver_path = device_path.join("driver");
    if let Ok(target) = fs::read_link(&driver_path) {
        if let Some(basename) = target.file_name() {
            return basename.to_string_lossy() == "asahi";
        }
    }
    false
}

#[cfg(target_os = "linux")]
/// Find the macsmc-hwmon directory by scanning `/sys/class/hwmon/hwmon*/name`
/// for a name containing "macsmc". Returns the hwmon directory path if found.
fn find_macsmc_hwmon() -> Option<PathBuf> {
    let hwmon_root = Path::new("/sys/class/hwmon");
    let entries = fs::read_dir(hwmon_root).ok()?;

    for entry in entries.flatten() {
        let name_path = entry.path().join("name");
        if let Ok(name) = fs::read_to_string(&name_path) {
            if name.to_lowercase().contains("macsmc") {
                return Some(entry.path());
            }
        }
    }
    None
}

// The struct only contains PathBuf, Option<PathBuf>, and String — all Send + Sync.
#[cfg(target_os = "linux")]
unsafe impl Send for AppleGpuSensor {}
#[cfg(target_os = "linux")]
unsafe impl Sync for AppleGpuSensor {}