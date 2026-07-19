use super::{CpuReading, CpuSensor};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

/// Linux CPU sensor implementation that works across Intel, AMD Ryzen, and Apple Silicon (Asahi Linux).
///
/// The hwmon path is resolved lazily on first read and cached in a OnceLock to avoid
/// repeated filesystem scans. This is safe because the hwmon device path doesn't change
/// at runtime.
#[cfg(target_os = "linux")]
pub struct LinuxCpuSensor {
    hwmon_path: OnceLock<Option<PathBuf>>,
}

#[cfg(target_os = "linux")]
impl LinuxCpuSensor {
    pub fn new() -> Self {
        Self {
            hwmon_path: OnceLock::new(),
        }
    }

    /// Find the hwmon directory for CPU temperature by scanning /sys/class/hwmon
    /// and matching against known driver names in priority order.
    fn find_cpu_temp_hwmon() -> Option<PathBuf> {
        let hwmon_base = Path::new("/sys/class/hwmon");
        let entries = fs::read_dir(hwmon_base).ok()?;

        // Priority order: k10temp (AMD Ryzen) > coretemp (Intel) > zenpower (AMD fallback) > macsmc/apple (Apple Silicon)
        let mut candidates: Vec<(u8, PathBuf)> = Vec::new();

        for entry in entries.flatten() {
            let path = entry.path();
            let name_path = path.join("name");
            // Some hwmon entries lack a "name" file — skip them instead of
            // aborting the entire scan (was `ok()?` which returned None).
            let name = match fs::read_to_string(&name_path) {
                Ok(s) => s.trim().to_lowercase(),
                Err(_) => continue,
            };

            let priority = match name.as_str() {
                "k10temp" => 0,      // AMD Ryzen (Zen) - highest priority
                "coretemp" => 1,     // Intel
                "zenpower" => 2,     // AMD fallback (older)
                n if n == "macsmc" || n.contains("apple") => 3, // Apple Silicon / Asahi
                _ => continue,
            };
            candidates.push((priority, path));
        }

        candidates.sort_by_key(|(p, _)| *p);
        candidates.into_iter().next().map(|(_, p)| p)
    }

    /// Read CPU temperature from the cached hwmon path.
    fn read_temp_celsius(&self) -> Option<f32> {
        let hwmon_dir = self.hwmon_path.get_or_init(Self::find_cpu_temp_hwmon).as_ref()?;

        // Read all temp*_input and temp*_label files
        let mut temp_inputs: Vec<(Option<String>, f32)> = Vec::new();

        for entry in fs::read_dir(hwmon_dir).ok()?.flatten() {
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.starts_with("temp") && file_name.ends_with("_input") {
                let num = &file_name[4..file_name.len() - 6]; // extract number between "temp" and "_input"
                let input_path = entry.path();
                let label_path = hwmon_dir.join(format!("temp{}_label", num));

                let label = fs::read_to_string(&label_path).ok().map(|s| s.trim().to_string());
                // If this particular temp input is unreadable or unparseable,
                // skip it rather than aborting the whole function.
                let temp_celsius = match fs::read_to_string(&input_path) {
                    Ok(s) => match s.trim().parse::<i64>() {
                        Ok(milli) => milli as f32 / 1000.0,
                        Err(_) => continue,
                    },
                    Err(_) => continue,
                };

                temp_inputs.push((label, temp_celsius));
            }
        }

        if temp_inputs.is_empty() {
            return None;
        }

        // For coretemp, prefer "Package id 0" label.
        // If the name file is unreadable at this point, just skip the
        // coretemp preference logic and fall through to the first temp.
        let hwmon_name = fs::read_to_string(hwmon_dir.join("name"))
            .map(|s| s.trim().to_lowercase())
            .unwrap_or_default();
        if hwmon_name == "coretemp" {
            for (label, temp) in &temp_inputs {
                if label.as_deref() == Some("Package id 0") {
                    return Some(*temp);
                }
            }
        }

        // Otherwise use the first available temperature (typically temp1_input)
        temp_inputs.into_iter().next().map(|(_, t)| t)
    }

    /// Read current CPU frequency in MHz.
    /// Tries cpufreq scaling_cur_freq first, falls back to /proc/cpuinfo average.
    fn read_freq_mhz() -> Option<u32> {
        // Try cpufreq first (more accurate, reflects current scaling)
        let cpufreq_path = Path::new("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq");
        if let Ok(content) = fs::read_to_string(cpufreq_path) {
            if let Ok(khz) = content.trim().parse::<u64>() {
                return Some((khz / 1000) as u32); // kHz to MHz
            }
        }

        // Fallback: parse /proc/cpuinfo for "cpu MHz" fields and average them
        let cpuinfo = fs::read_to_string("/proc/cpuinfo").ok()?;
        let mut sum = 0.0;
        let mut count = 0;

        for line in cpuinfo.lines() {
            if let Some(rest) = line.strip_prefix("cpu MHz") {
                if let Some(val_str) = rest.split(':').nth(1) {
                    if let Ok(mhz) = val_str.trim().parse::<f32>() {
                        sum += mhz;
                        count += 1;
                    }
                }
            }
        }

        if count > 0 {
            Some((sum / count as f32).round() as u32)
        } else {
            None
        }
    }

    /// Read CPU usage percent by sampling /proc/stat twice with a ~200ms interval.
    fn read_usage_percent() -> Result<f32> {
        let sample1 = Self::read_cpu_stat()?;
        thread::sleep(Duration::from_millis(200));
        let sample2 = Self::read_cpu_stat()?;

        let total_delta = sample2.total - sample1.total;
        let idle_delta = sample2.idle - sample1.idle;

        if total_delta == 0 {
            return Ok(0.0);
        }

        let usage = 100.0 * (1.0 - (idle_delta as f32 / total_delta as f32));
        Ok(usage.clamp(0.0, 100.0))
    }

    /// Parse the first "cpu " aggregate line from /proc/stat.
    /// Returns (total_jiffies, idle_jiffies).
    fn read_cpu_stat() -> Result<CpuStatSample> {
        let content = fs::read_to_string("/proc/stat")
            .context("failed to read /proc/stat")?;

        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("cpu ") {
                let parts: Vec<u64> = rest
                    .split_whitespace()
                    .filter_map(|s| s.parse().ok())
                    .collect();

                // /proc/stat format: user nice system idle iowait irq softirq steal guest guest_nice
                // We need at least 4 values (user, nice, system, idle)
                if parts.len() >= 4 {
                    let user = parts[0];
                    let nice = parts[1];
                    let system = parts[2];
                    let idle = parts[3];
                    let iowait = parts.get(4).copied().unwrap_or(0);
                    let irq = parts.get(5).copied().unwrap_or(0);
                    let softirq = parts.get(6).copied().unwrap_or(0);
                    let steal = parts.get(7).copied().unwrap_or(0);

                    let total = user + nice + system + idle + iowait + irq + softirq + steal;
                    return Ok(CpuStatSample { total, idle });
                }
            }
        }

        anyhow::bail!("no aggregate 'cpu ' line found in /proc/stat");
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy)]
struct CpuStatSample {
    total: u64,
    idle: u64,
}

#[cfg(target_os = "linux")]
impl CpuSensor for LinuxCpuSensor {
    fn read(&self) -> Result<CpuReading> {
        let usage_percent = Self::read_usage_percent()?;
        let temp_celsius = self.read_temp_celsius();
        let freq_mhz = Self::read_freq_mhz();
        let core_count = thread::available_parallelism()
            .map(|n| n.get() as u32)
            .unwrap_or(1);

        Ok(CpuReading {
            usage_percent,
            temp_celsius,
            freq_mhz,
            core_count,
        })
    }
}

#[cfg(target_os = "linux")]
impl Default for LinuxCpuSensor {
    fn default() -> Self {
        Self::new()
    }
}

/// Type alias for the public API expected by detect_backends()
#[cfg(target_os = "linux")]
pub type CpuSensorImpl = LinuxCpuSensor;