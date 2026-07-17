// Apple Silicon GPU sensor implementation for Asahi Linux and macOS.
// Uses powermetrics on macOS and sysfs on Asahi Linux.

use crate::sensors::{GpuReading, GpuSensor};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Apple Silicon GPU sensor.
/// On macOS: uses `powermetrics` for GPU usage, temperature, and memory.
/// On Asahi Linux: uses sysfs (drm/panfrost or drm/asahi) for available metrics.
pub struct AppleGpuSensor {
    #[cfg(target_os = "macos")]
    cached_name: String,
    #[cfg(target_os = "linux")]
    card_path: PathBuf,
    #[cfg(target_os = "linux")]
    hwmon_path: Option<PathBuf>,
    #[cfg(target_os = "linux")]
    cached_name: String,
}

impl AppleGpuSensor {
    /// Probe for Apple Silicon GPU.
    /// On macOS: always returns one sensor if on Apple Silicon (detected via sysctl).
    /// On Linux (Asahi): scans `/sys/class/drm/card*` for panfrost/asahi driver.
    pub fn probe() -> Vec<Self> {
        #[cfg(target_os = "macos")]
        {
            // Check if we're on Apple Silicon via sysctl
            let output = Command::new("sysctl")
                .args(["-n", "machdep.cpu.brand_string"])
                .output();
            let is_apple_silicon = output
                .map(|o| String::from_utf8_lossy(&o.stdout).contains("Apple"))
                .unwrap_or(false);

            if !is_apple_silicon {
                return vec![];
            }

            vec![Self {
                cached_name: "Apple Silicon GPU".to_string(),
            }]
        }

        #[cfg(target_os = "linux")]
        {
            let drm_path = Path::new("/sys/class/drm");
            let mut sensors = Vec::new();

            let entries = match fs::read_dir(drm_path) {
                Ok(e) => e,
                Err(_) => return sensors,
            };

            for entry in entries.flatten() {
                let file_name = entry.file_name();
                let name_str = file_name.to_string_lossy();

                // Only consider bare "cardN" directories
                if !name_str.starts_with("card") || name_str.contains('-') {
                    continue;
                }

                let card_path = entry.path();
                let device_path = card_path.join("device");

                // Check if this is an Apple/Asahi GPU (vendor 0x106b for Apple, or driver name)
                if !is_apple_gpu_device(&device_path) {
                    continue;
                }

                // Find hwmon for temperature (may not exist on Asahi yet)
                let hwmon_path = find_hwmon_dir(&device_path);

                // Get name from uevent or fallback
                let name = get_device_name(&device_path, &name_str);

                sensors.push(Self {
                    card_path,
                    hwmon_path,
                    cached_name: name,
                });
            }

            sensors
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            vec![]
        }
    }

    #[cfg(target_os = "macos")]
    fn read_via_powermetrics() -> Option<GpuReading> {
        // powermetrics -n 1 -s gpu_power --show-process-gpu --format plist
        // We use a simpler invocation: powermetrics -n 1 --samplers gpu_power -f plist
        let output = Command::new("powermetrics")
            .args(["-n", "1", "--samplers", "gpu_power", "-f", "plist"])
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let plist_str = String::from_utf8(output.stdout).ok()?;
        // Parse plist - we'll use a simple string search for the keys we need
        // since adding a plist parser crate would be overkill for this
        let usage = extract_plist_float(&plist_str, "GPU Utilization")?;
        let temp = extract_plist_float(&plist_str, "GPU Temperature")?;
        let vram_used = extract_plist_int(&plist_str, "GPU Memory Used")?;
        let vram_total = extract_plist_int(&plist_str, "GPU Memory Total")?;

        Some(GpuReading {
            name: "Apple Silicon GPU".to_string(),
            usage_percent: Some(usage),
            temp_celsius: Some(temp),
            vram_used_mb: Some((vram_used / 1024 / 1024) as u32),
            vram_total_mb: Some((vram_total / 1024 / 1024) as u32),
        })
    }

    #[cfg(target_os = "linux")]
    fn read_sysfs_string(path: &Path) -> Option<String> {
        fs::read_to_string(path).ok().map(|s| s.trim().to_string())
    }

    #[cfg(target_os = "linux")]
    fn read_sysfs_u64(path: &Path) -> Option<u64> {
        Self::read_sysfs_string(path)?.parse().ok()
    }
}

impl GpuSensor for AppleGpuSensor {
    fn read(&self) -> Result<GpuReading> {
        #[cfg(target_os = "macos")]
        {
            if let Some(reading) = Self::read_via_powermetrics() {
                return Ok(reading);
            }
            // Fallback if powermetrics fails
            return Ok(GpuReading {
                name: self.cached_name.clone(),
                usage_percent: None,
                temp_celsius: None,
                vram_used_mb: None,
                vram_total_mb: None,
            });
        }

        #[cfg(target_os = "linux")]
        {
            let device_path = self.card_path.join("device");

            // Usage: try to read from debugfs or sysfs if available (panfrost/asahi)
            // Currently no standard sysfs busy percent for Asahi, so None
            let usage_percent: Option<f32> = None;

            // Temperature from hwmon if available
            let temp_celsius = self.hwmon_path.as_ref().and_then(|hwmon| {
                Self::read_sysfs_u64(&hwmon.join("temp1_input")).map(|v| v as f32 / 1000.0)
            });

            // VRAM: Apple Silicon uses unified memory, no dedicated VRAM
            let vram_used_mb = None;
            let vram_total_mb = None;

            if !device_path.exists() {
                anyhow::bail!("Apple GPU device path vanished: {:?}", device_path);
            }

            Ok(GpuReading {
                name: self.cached_name.clone(),
                usage_percent,
                temp_celsius,
                vram_used_mb,
                vram_total_mb,
            })
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            Ok(GpuReading {
                name: self.cached_name.clone(),
                usage_percent: None,
                temp_celsius: None,
                vram_used_mb: None,
                vram_total_mb: None,
            })
        }
    }

    fn name(&self) -> &str {
        &self.cached_name
    }
}

#[cfg(target_os = "macos")]
fn extract_plist_float(plist: &str, key: &str) -> Option<f32> {
    // Simple plist parsing for <key>Key</key><real>Value</real> or <integer>Value</integer>
    let key_tag = format!("<key>{}</key>", key);
    let start = plist.find(&key_tag)?;
    let after_key = &plist[start + key_tag.len()..];
    // Look for <real> or <integer>
    let real_start = after_key.find("<real>")?;
    let real_end = after_key[real_start..].find("</real>")?;
    let value_str = &after_key[real_start + 6..real_start + real_end];
    value_str.trim().parse().ok()
}

#[cfg(target_os = "macos")]
fn extract_plist_int(plist: &str, key: &str) -> Option<u64> {
    let key_tag = format!("<key>{}</key>", key);
    let start = plist.find(&key_tag)?;
    let after_key = &plist[start + key_tag.len()..];
    let int_start = after_key.find("<integer>")?;
    let int_end = after_key[int_start..].find("</integer>")?;
    let value_str = &after_key[int_start + 9..int_start + int_end];
    value_str.trim().parse().ok()
}

#[cfg(target_os = "linux")]
fn is_apple_gpu_device(device_path: &Path) -> bool {
    // Check vendor ID (Apple = 0x106b) or driver name
    let vendor_path = device_path.join("vendor");
    if let Ok(vendor) = fs::read_to_string(&vendor_path) {
        if vendor.trim() == "0x106b" {
            return true;
        }
    }

    // Check driver symlink for panfrost/asahi
    let driver_path = device_path.join("driver");
    if let Ok(target) = fs::read_link(&driver_path) {
        let driver_name = target.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if driver_name.contains("panfrost") || driver_name.contains("asahi") {
            return true;
        }
    }

    false
}

#[cfg(target_os = "linux")]
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
fn get_device_name(device_path: &Path, card_name: &str) -> String {
    // Parse uevent for PCI_ID
    if let Some(uevent) = AppleGpuSensor::read_sysfs_string(&device_path.join("uevent")) {
        for line in uevent.lines() {
            if let Some(pci_id) = line.strip_prefix("PCI_ID=") {
                return format!("Apple GPU ({})", pci_id);
            }
        }
    }
    format!("Apple GPU ({})", card_name)
}

// Send + Sync: only contains PathBuf, Option<PathBuf>, String
unsafe impl Send for AppleGpuSensor {}
unsafe impl Sync for AppleGpuSensor {}