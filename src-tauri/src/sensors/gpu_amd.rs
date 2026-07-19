// AMD GPU sensor implementation using pure sysfs reads (amdgpu kernel driver).
// Supports discrete AMD GPUs and Ryzen APU integrated graphics.

use crate::sensors::{GpuReading, GpuSensor};
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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

    /// Read GPU usage via radeontop.
    /// Returns None if radeontop is not available or parsing fails.
    fn read_gpu_usage_radeontop() -> Option<f32> {
        let output = Command::new("radeontop")
            .args(["-d", "-", "-l", "1"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8(output.stdout).ok()?;
        // Parse "gpu X.XX%" from the output line
        for part in stdout.split(',') {
            let part = part.trim();
            if part.starts_with("gpu ") {
                let pct_str = part
                    .strip_prefix("gpu ")?
                    .strip_suffix('%')?
                    .trim();
                return pct_str.parse::<f32>().ok();
            }
        }
        None
    }

    /// Read GPU usage (%) from the kernel `gpu_busy_percent` sysfs file.
    /// Present on many amdgpu parts; returns None if absent/unreadable.
    fn read_usage_via_sysfs(&self) -> Option<f32> {
        Self::read_sysfs_string(&self.device_path.join("gpu_busy_percent"))
            .and_then(|s| s.trim().parse::<f32>().ok())
    }

    /// Read the current graphics clock (MHz) from amdgpu sysfs.
    /// Tries `pp_dpm_sclk` (the line marked with '*') first, then falls back to
    /// the hwmon `freq1_input` file.  Returns None if neither is available.
    fn read_clock_mhz(&self) -> Option<u32> {
        // pp_dpm_sclk lists clock states; the active one is marked with '*'.
        if let Some(s) = Self::read_sysfs_string(&self.device_path.join("pp_dpm_sclk")) {
            for line in s.lines() {
                if line.contains('*') {
                    // Format: "0: 200Mhz *" or "1: 1200MHz"
                    if let Some(colon) = line.find(':') {
                        let rest = &line[colon + 1..];
                        let digits: String =
                            rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                        if let Ok(mhz) = digits.parse::<u32>() {
                            return Some(mhz);
                        }
                    }
                }
            }
        }

        // Fallback: hwmon freq1_input (kHz) on some APUs.
        if let Some(hwmon) = &self.hwmon_path {
            if let Some(s) = Self::read_sysfs_string(&hwmon.join("freq1_input")) {
                if let Ok(khz) = s.trim().parse::<u64>() {
                    return Some((khz / 1000) as u32);
                }
            }
        }
        None
    }

    /// Best-effort APU (integrated) detection.
    ///
    /// AMD APUs expose their GPU through the same `amdgpu` driver as discrete
    /// cards, so we can't rely on the driver alone.  We instead match the PCI
    /// device ID against a known list of integrated APU parts (Rembrandt,
    /// Phoenix, Cezanne, Van Gogh, Strix Point, etc.).  Anything not on the
    /// list is treated as discrete (false) — a safe default.
    fn is_apu(device_path: &Path) -> bool {
        let device_id = fs::read_to_string(device_path.join("device"))
            .ok()
            .map(|s| s.trim().to_string());
        match device_id.as_deref() {
            Some(id) => AMD_APU_DEVICE_IDS.iter().any(|d| d.eq_ignore_ascii_case(id)),
            None => false,
        }
    }
}

/// PCI device IDs of well-known AMD integrated APU graphics (Radeon Graphics
/// built into the CPU die).  Used only to flag GPUs as "integrated".
#[cfg(target_os = "linux")]
const AMD_APU_DEVICE_IDS: &[&str] = &[
    // Rembrandt (Ryzen 6000, e.g. 680M)
    "0x1681", "0x1682",
    // Phoenix / Phoenix2 (Ryzen 7000/8000 mobile, e.g. 780M)
    "0x15bf", "0x15c8", "0x15d8",
    // Cezanne / Renoir / Lucienne (Ryzen 5000/4000)
    "0x1638", "0x164c", "0x164e", "0x1636",
    // Van Gogh (Steam Deck)
    "0x163f",
    // Strix Point (Ryzen AI 300, e.g. 890M)
    "0x150e",
];

#[cfg(target_os = "linux")]
impl GpuSensor for AmdGpuSensor {
    fn read(&self) -> Result<GpuReading> {
        // Usage: prefer the kernel gpu_busy_percent sysfs file (no extra tool
        // needed), then fall back to radeontop.
        let usage_percent = self
            .read_usage_via_sysfs()
            .or_else(Self::read_gpu_usage_radeontop);

        // Clock: best-effort from amdgpu pp_dpm_sclk (last "*" marked line) or
        // hwmon freq1_input.  Falls back to None if neither is available.
        let clock_mhz = self.read_clock_mhz();

        // Temperature: read hwmon/temp1_input (millidegrees Celsius)
        let temp_celsius = self.hwmon_path.as_ref().and_then(|hwmon| {
            Self::read_sysfs_string(&hwmon.join("temp1_input"))
                .and_then(|s| s.parse::<u64>().ok())
                .map(|v| v as f32 / 1000.0)
        });

        // VRAM: read device/mem_info_vram_used and device/mem_info_vram_total (bytes -> MB)
        let vram_used_mb = Self::read_sysfs_string(&self.device_path.join("mem_info_vram_used"))
            .and_then(|s| s.parse::<u64>().ok())
            .map(|v| (v / 1024 / 1024) as u32);
        let vram_total_mb = Self::read_sysfs_string(&self.device_path.join("mem_info_vram_total"))
            .and_then(|s| s.parse::<u64>().ok())
            .map(|v| (v / 1024 / 1024) as u32);

        // Integrated detection via known APU device IDs.
        let is_integrated = Self::is_apu(&self.device_path);

        let vram_type: Option<String> = None;

        // If the device directory itself is gone, that's a hard error (device removed/driver unloaded)
        if !self.device_path.exists() {
            anyhow::bail!("AMD GPU device path vanished: {:?}", self.device_path);
        }

        Ok(GpuReading {
            name: self.cached_name.clone(),
            usage_percent,
            temp_celsius,
            clock_mhz,
            is_integrated,
            vram_used_mb,
            vram_total_mb,
            vram_type,
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
