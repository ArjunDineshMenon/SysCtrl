// Fan controller implementation using hwmon sysfs interface.
//
// READ path: runs unprivileged in the main GUI process — reads fanN_input (RPM)
//            and pwmN (duty cycle 0-255, converted to percent) from sysfs.
//
// WRITE path: delegates to `/usr/local/bin/sysctl-helper` (a small setuid/polkit
//             helper binary) so the GUI never needs root.

#[cfg(target_os = "linux")]
use crate::sensors::{FanController, FanReading};
#[cfg(target_os = "linux")]
use anyhow::{bail, Context, Result};
#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::Command;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A single discovered fan header on the system.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
struct FanEntry {
    /// Human-readable label (e.g. "CPU fan", "GPU fan 1", "Fan 2 (nct6775)").
    label: String,
    /// Absolute path to `fanN_input` (RPM reading).
    input_path: PathBuf,
    /// Absolute path to `pwmN` (duty-cycle control), if one exists for this fan.
    pwm_path: Option<PathBuf>,
}

/// Fan controller backed by the Linux hwmon sysfs interface.
///
/// Scans motherboard hwmon devices (`/sys/class/hwmon/hwmon*`) and discrete GPU
/// hwmon devices (`/sys/class/drm/card*/device/hwmon/hwmon*`).
#[cfg(target_os = "linux")]
pub struct SysfsFanController {
    fans: Vec<FanEntry>,
}

// FanEntry is just PathBuf + String — both are Send + Sync.
// SysfsFanController holds a Vec of those, so it is too.
// The compiler can prove this on its own, but an explicit assertion makes the
// intent visible and avoids any future-proofing surprises.
#[cfg(target_os = "linux")]
unsafe impl Send for SysfsFanController {}
#[cfg(target_os = "linux")]
unsafe impl Sync for SysfsFanController {}

// ---------------------------------------------------------------------------
// Known CPU chip names — used to build a nicer "CPU fan" label.
// ---------------------------------------------------------------------------

/// hwmon `name` values that belong to a CPU thermal/fan chip.
#[cfg(target_os = "linux")]
const CPU_CHIP_NAMES: &[&str] = &[
    "nct6775", "nct6776", "nct6779", "nct6791", "nct6792", "nct6793",
    "nct6795", "nct6796", "nct6797", "nct6798",
    "it87", "it8603", "it8613", "it8620", "it8622", "it8625",
    "it8628", "it8655", "it8665", "it8686", "it8688",
    "it8689", "it8695", "it8705", "it8712", "it8716",
    "it8718", "it8720", "it8721", "it8726", "it8728",
    "it8732", "it8771", "it8772", "it8781", "it8782",
    "it8783", "it8786", "it8790",
    "k10temp",
    "coretemp",
    "w83627ehf", "w83627dhg", "w83667hg", "w83795g",
    "f71882fg", "f71889fg",
];

/// Returns `true` if the hwmon `name` is a known CPU/motherboard super-I/O chip.
#[cfg(target_os = "linux")]
fn is_cpu_chip(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    CPU_CHIP_NAMES.iter().any(|&n| lower == n)
}

/// Returns `true` if `hwmon_dir` lives under `/sys/class/drm/card*` — i.e. it
/// belongs to a discrete GPU.
#[cfg(target_os = "linux")]
fn is_gpu_hwmon(hwmon_dir: &Path) -> bool {
    // Canonical example: /sys/class/drm/card0/device/hwmon/hwmon3
    hwmon_dir
        .to_str()
        .map(|s| s.contains("/drm/card"))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Probing
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
impl SysfsFanController {
    /// Scan the system for all fan headers.
    ///
    /// Searches two sysfs trees:
    ///  1. `/sys/class/hwmon/hwmon*` — motherboard super-I/O chips.
    ///  2. `/sys/class/drm/card*/device/hwmon/hwmon*` — GPU fan headers.
    pub fn probe() -> Self {
        let mut fans = Vec::new();
        let mut gpu_counter: u32 = 0;

        // --- 1. Motherboard / generic hwmon devices ---
        if let Ok(entries) = fs::read_dir("/sys/class/hwmon") {
            for entry in entries.flatten() {
                let hwmon_dir = entry.path();
                let hwmon_name = read_hwmon_name(&hwmon_dir);
                Self::scan_hwmon_dir(&hwmon_dir, &hwmon_name, &mut gpu_counter, &mut fans);
            }
        }

        // --- 2. GPU hwmon devices (amdgpu / radeon / nouveau / xe) ---
        if let Ok(cards) = fs::read_dir("/sys/class/drm") {
            for card in cards.flatten() {
                let card_name = card.file_name();
                let card_str = card_name.to_string_lossy();
                // Only look at cardN entries, skip renderD* etc.
                if !card_str.starts_with("card") || card_str.contains('-') {
                    continue;
                }
                let hwmon_parent = card.path().join("device/hwmon");
                if let Ok(hwmons) = fs::read_dir(&hwmon_parent) {
                    for hwmon in hwmons.flatten() {
                        let hwmon_dir = hwmon.path();
                        let hwmon_name = read_hwmon_name(&hwmon_dir);
                        Self::scan_hwmon_dir(
                            &hwmon_dir,
                            &hwmon_name,
                            &mut gpu_counter,
                            &mut fans,
                        );
                    }
                }
            }
        }

        Self { fans }
    }

    /// Walk a single hwmon directory looking for `fanN_input` files.
    fn scan_hwmon_dir(
        hwmon_dir: &Path,
        hwmon_name: &str,
        gpu_counter: &mut u32,
        out: &mut Vec<FanEntry>,
    ) {
        let Ok(dir_entries) = fs::read_dir(hwmon_dir) else {
            return;
        };

        for de in dir_entries.flatten() {
            let fname = de.file_name();
            let fname_str = fname.to_string_lossy();

            if let Some(n) = parse_fan_index(&fname_str) {
                let input_path = hwmon_dir.join(format!("fan{}_input", n));

                // Look for a sibling pwmN — if it exists the fan is writable.
                let pwm_file = hwmon_dir.join(format!("pwm{}", n));
                let pwm_path = if pwm_file.exists() {
                    Some(pwm_file)
                } else {
                    None
                };

                let label = build_label(hwmon_dir, hwmon_name, n, gpu_counter);

                out.push(FanEntry {
                    label,
                    input_path,
                    pwm_path,
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// FanController trait implementation
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
impl FanController for SysfsFanController {
    /// Read RPM and duty-cycle percentage for every discovered fan.
    fn read_all(&self) -> Result<Vec<FanReading>> {
        let mut readings = Vec::with_capacity(self.fans.len());

        for fan in &self.fans {
            // RPM from fanN_input (an integer string, e.g. "1200\n").
            let rpm = read_u32(&fan.input_path);

            // Duty-cycle from pwmN (0-255 raw), converted to 0-100 %.
            let (percent, writable) = match &fan.pwm_path {
                Some(p) => {
                    let pct = read_u32(p).map(|raw| {
                        ((raw as f32 / 255.0) * 100.0).round() as u8
                    });
                    (pct, true)
                }
                None => (None, false),
            };

            readings.push(FanReading {
                label: fan.label.clone(),
                rpm,
                percent,
                writable,
            });
        }

        Ok(readings)
    }

    /// Set a fan's duty cycle via the privileged helper binary.
    ///
    /// The GUI app does **not** write to sysfs directly (that would require
    /// root).  Instead we invoke `/usr/local/bin/sysctl-helper set-fan <path> <value>`.
    fn set_percent(&self, fan_label: &str, percent: u8) -> Result<()> {
        let percent = percent.min(100);

        let fan = self
            .fans
            .iter()
            .find(|f| f.label == fan_label)
            .ok_or_else(|| anyhow::anyhow!("fan not found: {}", fan_label))?;

        let pwm_path = fan
            .pwm_path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("fan '{}' is not writable (no PWM control)", fan_label))?;

        // Convert 0-100 % → 0-255 raw PWM value.
        let pwm_value = ((percent as f32 / 100.0) * 255.0).round() as u32;

        let pwm_path_str = pwm_path.to_string_lossy();

        // ── Option A (default): use pkexec for a polkit password prompt. ──
        // This pops up a graphical auth dialog the first time.
        let output = Command::new("pkexec")
            .args([
                "/usr/local/bin/sysctl-helper",
                "set-fan",
                &pwm_path_str,
                &pwm_value.to_string(),
            ])
            .output()
            .context("failed to launch pkexec for sysctl-helper")?;

        // ── Option B (alternative): call the helper directly.            ──
        // Requires that sysctl-helper is installed with the setuid bit set
        // (`chmod u+s /usr/local/bin/sysctl-helper`) *or* that a sudoers
        // exception is configured.  This avoids the polkit prompt but means
        // the binary runs as root without user confirmation each time.
        //
        // let output = Command::new("/usr/local/bin/sysctl-helper")
        //     .args(["set-fan", &pwm_path_str, &pwm_value.to_string()])
        //     .output()
        //     .context("failed to launch sysctl-helper")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "sysctl-helper failed (exit {}): {}",
                output.status.code().unwrap_or(-1),
                stderr.trim()
            );
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Read the `name` file inside a hwmon directory (e.g. "nct6775").
#[cfg(target_os = "linux")]
fn read_hwmon_name(hwmon_dir: &Path) -> String {
    fs::read_to_string(hwmon_dir.join("name"))
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Extract the numeric index N from a filename matching `fanN_input`.
#[cfg(target_os = "linux")]
fn parse_fan_index(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("fan")?;
    let num_str = rest.strip_suffix("_input")?;
    num_str.parse().ok()
}

/// Build a human-readable label for a fan.
///
///  * If the hwmon `name` matches a known CPU chip → `"CPU fan"` (or `"CPU fan N"` when N > 1).
///  * If the hwmon dir lives under `/sys/class/drm/card*` → `"GPU fan N"`.
///  * Otherwise → `"Fan N (hwmon_name)"`.
#[cfg(target_os = "linux")]
fn build_label(hwmon_dir: &Path, hwmon_name: &str, fan_index: u32, gpu_counter: &mut u32) -> String {
    // Try the kernel-provided label first (fanN_label).
    let label_path = hwmon_dir.join(format!("fan{}_label", fan_index));
    if let Ok(raw) = fs::read_to_string(&label_path) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    if is_gpu_hwmon(hwmon_dir) {
        *gpu_counter += 1;
        return format!("GPU fan {}", gpu_counter);
    }

    if is_cpu_chip(hwmon_name) {
        return if fan_index == 1 {
            "CPU fan".to_string()
        } else {
            format!("CPU fan {}", fan_index)
        };
    }

    format!("Fan {} ({})", fan_index, hwmon_name)
}

/// Read a sysfs file containing a single unsigned integer.
#[cfg(target_os = "linux")]
fn read_u32(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}