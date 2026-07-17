// Fan controller implementation using hwmon sysfs interface.
// Supports reading fan speeds (RPM), PWM duty cycle, and setting fan percentages.

use crate::sensors::{FanController, FanReading};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Fan controller using hwmon sysfs interface.
/// Scans all hwmon devices for fan inputs and PWM controls.
pub struct HwmonFanController {
    fan_paths: Vec<FanPathInfo>,
}

#[derive(Debug, Clone)]
struct FanPathInfo {
    hwmon_dir: PathBuf,
    fan_index: u32,        // 1-based index for fanN_input
    pwm_index: Option<u32>, // 1-based index for pwmN (if writable)
    label: String,
}

impl HwmonFanController {
    /// Probe all hwmon devices for fan inputs and PWM controls.
    /// Returns a controller with all discovered fans.
    pub fn probe() -> Self {
        let hwmon_base = Path::new("/sys/class/hwmon");
        let mut fan_paths = Vec::new();

        let entries = match fs::read_dir(hwmon_base) {
            Ok(e) => e,
            Err(_) => return Self { fan_paths: Vec::new() },
        };

        for entry in entries.flatten() {
            let hwmon_dir = entry.path();
            let name = Self::read_hwmon_name(&hwmon_dir);

            // Scan for fan*_input files
            if let Ok(dir_entries) = fs::read_dir(&hwmon_dir) {
                for fan_entry in dir_entries.flatten() {
                    let file_name = fan_entry.file_name();
                    let name_str = file_name.to_string_lossy();

                    // Match fan{N}_input pattern
                    if let Some(index) = Self::parse_fan_index(&name_str) {
                        let label = Self::build_fan_label(&hwmon_dir, &name, index);
                        let pwm_index = Self::find_matching_pwm(&hwmon_dir, index);

                        fan_paths.push(FanPathInfo {
                            hwmon_dir: hwmon_dir.clone(),
                            fan_index: index,
                            pwm_index,
                            label,
                        });
                    }
                }
            }
        }

        Self { fan_paths }
    }

    /// Read the hwmon device name from the "name" file.
    fn read_hwmon_name(hwmon_dir: &Path) -> String {
        fs::read_to_string(hwmon_dir.join("name"))
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    }

    /// Parse fan index from "fan{N}_input" filename.
    fn parse_fan_index(name: &str) -> Option<u32> {
        if name.starts_with("fan") && name.ends_with("_input") {
            let num_str = &name[3..name.len() - 6];
            num_str.parse().ok()
        } else {
            None
        }
    }

    /// Build a human-readable label for the fan.
    fn build_fan_label(hwmon_dir: &Path, hwmon_name: &str, index: u32) -> String {
        // Try to read fan{N}_label first
        let label_path = hwmon_dir.join(format!("fan{}_label", index));
        if let Ok(label) = fs::read_to_string(&label_path) {
            let label = label.trim();
            if !label.is_empty() {
                return format!("{} {}", hwmon_name, label);
            }
        }
        format!("{} fan{}", hwmon_name, index)
    }

    /// Find PWM control that matches this fan index.
    /// Some hwmon devices have pwm{N} corresponding to fan{N}.
    fn find_matching_pwm(hwmon_dir: &Path, fan_index: u32) -> Option<u32> {
        // First try direct mapping: pwm{N} for fan{N}
        let pwm_path = hwmon_dir.join(format!("pwm{}", fan_index));
        if pwm_path.exists() {
            // Check if it's writable (has pwm{N}_enable with value > 0)
            let enable_path = hwmon_dir.join(format!("pwm{}_enable", fan_index));
            if let Ok(enable_str) = fs::read_to_string(&enable_path) {
                if enable_str.trim().parse::<u32>().unwrap_or(0) > 0 {
                    return Some(fan_index);
                }
            }
        }

        // Fallback: scan all pwm*_enable files for writable ones
        if let Ok(entries) = fs::read_dir(hwmon_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy();
                if name.starts_with("pwm") && name.ends_with("_enable") {
                    let num_str = &name[3..name.len() - 7];
                    if let Ok(pwm_idx) = num_str.parse::<u32>() {
                        let enable_path = entry.path();
                        if let Ok(enable_str) = fs::read_to_string(&enable_path) {
                            if enable_str.trim().parse::<u32>().unwrap_or(0) > 0 {
                                return Some(pwm_idx);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Read a sysfs file as u32, returning None if missing/invalid.
    fn read_u32(path: &Path) -> Option<u32> {
        fs::read_to_string(path).ok()?.trim().parse().ok()
    }

    /// Read a sysfs file as string, returning None if missing.
    fn read_string(path: &Path) -> Option<String> {
        fs::read_to_string(path).ok().map(|s| s.trim().to_string())
    }

    /// Write a u32 value to a sysfs file.
    fn write_u32(path: &Path, value: u32) -> Result<()> {
        fs::write(path, value.to_string())
            .with_context(|| format!("failed to write to {:?}", path))
    }
}

impl FanController for HwmonFanController {
    fn read_all(&self) -> Result<Vec<FanReading>> {
        let mut readings = Vec::with_capacity(self.fan_paths.len());

        for fan in &self.fan_paths {
            // Read RPM from fan{N}_input
            let rpm_path = fan.hwmon_dir.join(format!("fan{}_input", fan.fan_index));
            let rpm = Self::read_u32(&rpm_path);

            // Read PWM duty cycle percentage from pwm{N} (0-255), convert to 0-100%
            let percent = fan.pwm_index.and_then(|pwm_idx| {
                let pwm_path = fan.hwmon_dir.join(format!("pwm{}", pwm_idx));
                Self::read_u32(&pwm_path).map(|pwm| ((pwm as f32 / 255.0) * 100.0).round() as u8)
            });

            // Writable if we have a PWM control
            let writable = fan.pwm_index.is_some();

            readings.push(FanReading {
                label: fan.label.clone(),
                rpm,
                percent,
                writable,
            });
        }

        Ok(readings)
    }

    fn set_percent(&self, fan_label: &str, percent: u8) -> Result<()> {
        // Clamp to 0-100
        let percent = percent.min(100);

        // Find the fan by label
        let fan = self
            .fan_paths
            .iter()
            .find(|f| f.label == fan_label)
            .ok_or_else(|| anyhow::anyhow!("fan not found: {}", fan_label))?;

        // Must have PWM control to set percentage
        let pwm_idx = fan
            .pwm_index
            .ok_or_else(|| anyhow::anyhow!("fan '{}' is not writable (no PWM control)", fan_label))?;

        // Convert 0-100% to 0-255 PWM value
        let pwm_value = ((percent as f32 / 100.0) * 255.0).round() as u32;

        // Ensure PWM is enabled (mode 1 = manual/user control)
        let enable_path = fan.hwmon_dir.join(format!("pwm{}_enable", pwm_idx));
        if enable_path.exists() {
            Self::write_u32(&enable_path, 1)?;
        }

        // Write the PWM value
        let pwm_path = fan.hwmon_dir.join(format!("pwm{}", pwm_idx));
        Self::write_u32(&pwm_path, pwm_value)?;

        Ok(())
    }
}

// Send + Sync: only contains Vec<FanPathInfo> with PathBuf and String
unsafe impl Send for HwmonFanController {}
unsafe impl Sync for HwmonFanController {}