#[cfg(target_os = "linux")]
mod cpu;
#[cfg(target_os = "linux")]
mod gpu_nvidia;
#[cfg(target_os = "linux")]
mod gpu_amd;
#[cfg(target_os = "linux")]
mod gpu_intel;
#[cfg(target_os = "linux")]
mod gpu_apple;
#[cfg(target_os = "linux")]
mod fan;
#[cfg(target_os = "linux")]
mod disk;

// Mock backends (compiled on all platforms — no cfg gate).
pub mod mock;

#[cfg(target_os = "linux")]
use crate::sensors::cpu::CpuSensorImpl;
#[cfg(target_os = "linux")]
use crate::sensors::disk::SysinfoDiskSensor;
#[cfg(target_os = "linux")]
use crate::sensors::fan::SysfsFanController;
#[cfg(target_os = "linux")]
use crate::sensors::gpu_amd::AmdGpuSensor;
#[cfg(target_os = "linux")]
use crate::sensors::gpu_apple::AppleGpuSensor;
#[cfg(target_os = "linux")]
use crate::sensors::gpu_intel::IntelGpuSensor;
#[cfg(target_os = "linux")]
use crate::sensors::gpu_nvidia::NvidiaGpuSensor;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuReading {
    pub usage_percent: f32,
    pub temp_celsius: Option<f32>,
    pub freq_mhz: Option<u32>,
    pub core_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuReading {
    pub name: String,
    pub usage_percent: Option<f32>,
    pub temp_celsius: Option<f32>,
    pub clock_mhz: Option<u32>,
    pub is_integrated: bool,
    pub vram_used_mb: Option<u32>,
    pub vram_total_mb: Option<u32>,
    pub vram_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FanReading {
    pub label: String,
    pub rpm: Option<u32>,
    pub percent: Option<u8>,
    pub writable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RamReading {
    pub used_mb: u32,
    pub total_mb: u32,
    pub percent: f32,
    pub model: Option<String>,
    pub speed_mhz: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskReading {
    /// Block device name, e.g. "nvme0n1" or "sda".
    pub device: String,
    /// Model string from the kernel (e.g. "Samsung SSD 980").
    pub model: Option<String>,
    /// Mount point this usage figure corresponds to (e.g. "/").
    pub mount: Option<String>,
    /// Total capacity in bytes.
    pub total_bytes: u64,
    /// Used bytes.
    pub used_bytes: u64,
    /// Sustained read throughput in MB/s (best-effort, may be None).
    pub read_rate_mbps: Option<f32>,
    /// Sustained write throughput in MB/s (best-effort, may be None).
    pub write_rate_mbps: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemSnapshot {
    pub cpu: CpuReading,
    pub gpus: Vec<GpuReading>,
    pub fans: Vec<FanReading>,
    pub ram: RamReading,
    pub disks: Vec<DiskReading>,
}

/// Read RAM usage from /proc/meminfo and DIMM model/speed via `dmidecode`.
///
/// MemTotal/MemAvailable compute used memory (same as `free`/`htop`).  DIMM
/// model + speed are best-effort: we shell out to `dmidecode -t memory`, which
/// normally requires root.  If it's missing, denied, or unparseable we simply
/// leave those fields `None` — the snapshot must never fail because of RAM
/// metadata we can't read.
pub fn read_ram() -> RamReading {
    let (model, speed_mhz) = read_ram_metadata();

    let contents = match fs::read_to_string("/proc/meminfo") {
        Ok(c) => c,
        Err(_) => return RamReading {
            used_mb: 0,
            total_mb: 0,
            percent: 0.0,
            model,
            speed_mhz,
        },
    };

    let mut total_kb: u64 = 0;
    let mut available_kb: u64 = 0;

    for line in contents.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total_kb = parse_meminfo_value(rest);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            available_kb = parse_meminfo_value(rest);
        }
        if total_kb > 0 && available_kb > 0 {
            break;
        }
    }

    if total_kb == 0 {
        return RamReading {
            used_mb: 0,
            total_mb: 0,
            percent: 0.0,
            model,
            speed_mhz,
        };
    }

    let used_kb = total_kb.saturating_sub(available_kb);
    let total_mb = (total_kb / 1024) as u32;
    let used_mb = (used_kb / 1024) as u32;
    let percent = (used_kb as f64 / total_kb as f64 * 100.0) as f32;

    RamReading {
        used_mb,
        total_mb,
        percent,
        model,
        speed_mhz,
    }
}

/// Best-effort DIMM model + speed via `dmidecode -t memory`.
///
/// Returns `(None, None)` if dmidecode is unavailable, denied (no root), or the
/// output can't be parsed.  Never errors — this is purely supplementary info.
fn read_ram_metadata() -> (Option<String>, Option<u32>) {
    let output = std::process::Command::new("dmidecode")
        .args(["-t", "memory"])
        .output()
        .ok();

    let output = match output {
        Some(o) if o.status.success() => o,
        _ => return (None, None),
    };

    let text = String::from_utf8_lossy(&output.stdout);

    let mut model: Option<String> = None;
    let mut speed: Option<u32> = None;

    for line in text.lines() {
        let line = line.trim();
        if model.is_none() {
            if let Some(rest) = line.strip_prefix("Type:") {
                let v = rest.trim();
                // Ignore the generic "Type: DDR4" vs part-number ambiguity by
                // preferring a real Part Number when present.
                if !v.is_empty() && v != "Unknown" && !v.starts_with("Other") {
                    // Keep DDRx type as the model fallback.
                    if v.starts_with("DDR") {
                        model = Some(v.to_string());
                    }
                }
            } else if let Some(rest) = line.strip_prefix("Part Number:") {
                let v = rest.trim();
                if !v.is_empty() && v != "Unknown" && v != "Not Specified" {
                    model = Some(v.to_string());
                }
            }
        }
        if speed.is_none() {
            if let Some(rest) = line.strip_prefix("Speed:") {
                let v = rest.trim().trim_end_matches("MT/s").trim_end_matches("MHz").trim();
                if let Ok(n) = v.parse::<u32>() {
                    speed = Some(n);
                }
            }
        }
        // Stop early once we have both from the first populated DIMM.
        if model.is_some() && speed.is_some() {
            break;
        }
    }

    (model, speed)
}

/// Parse a /proc/meminfo value line like "  12345 kB" into a u64 (kB).
fn parse_meminfo_value(s: &str) -> u64 {
    s.trim()
        .strip_suffix("kB")
        .or_else(|| s.trim().strip_suffix("KB"))
        .unwrap_or(s.trim())
        .trim()
        .parse::<u64>()
        .unwrap_or(0)
}

pub trait CpuSensor: Send + Sync {
    fn read(&self) -> Result<CpuReading>;
}

pub trait GpuSensor: Send + Sync {
    fn read(&self) -> Result<GpuReading>;
    fn name(&self) -> &str;
}

pub trait FanController: Send + Sync {
    fn read_all(&self) -> Result<Vec<FanReading>>;
    fn set_percent(&self, fan_label: &str, percent: u8) -> Result<()>;
}

pub trait DiskSensor: Send + Sync {
    fn read_all(&self) -> Result<Vec<DiskReading>>;
}

pub struct DetectedBackends {
    pub cpu: Box<dyn CpuSensor>,
    pub gpus: Vec<Box<dyn GpuSensor>>,
    pub fan: Box<dyn FanController>,
    pub disks: Box<dyn DiskSensor>,
}

/// No-op CPU sensor for non-Linux platforms (Windows, macOS).
/// Returns zeroed readings so the app compiles and runs without sensor data.
#[cfg(not(target_os = "linux"))]
pub struct NoopCpuSensor;

#[cfg(not(target_os = "linux"))]
impl CpuSensor for NoopCpuSensor {
    fn read(&self) -> Result<CpuReading> {
        Ok(CpuReading {
            usage_percent: 0.0,
            temp_celsius: None,
            freq_mhz: None,
            core_count: 1,
        })
    }
}

/// No-op fan controller for non-Linux platforms.
/// Returns empty fan list and ignores set_percent calls.
#[cfg(not(target_os = "linux"))]
pub struct NoopFanController;

#[cfg(not(target_os = "linux"))]
impl FanController for NoopFanController {
    fn read_all(&self) -> Result<Vec<FanReading>> {
        Ok(vec![])
    }

    fn set_percent(&self, _fan_label: &str, _percent: u8) -> Result<()> {
        Ok(())
    }
}

/// No-op disk sensor for non-Linux platforms.
#[cfg(not(target_os = "linux"))]
pub struct NoopDiskSensor;

#[cfg(not(target_os = "linux"))]
impl DiskSensor for NoopDiskSensor {
    fn read_all(&self) -> Result<Vec<DiskReading>> {
        Ok(vec![])
    }
}

pub fn detect_backends() -> DetectedBackends {
    // ── GPU probing ──────────────────────────────────────────────────────
    // Each backend's probe() returns Vec<Self>; a machine can have multiple
    // GPUs across vendors (e.g. Intel iGPU + NVIDIA dGPU), so we chain
    // everything into one flat list.
    let mut gpus: Vec<Box<dyn GpuSensor>> = Vec::new();

    #[cfg(target_os = "linux")]
    {
        for s in NvidiaGpuSensor::probe() {
            gpus.push(Box::new(s));
        }
        for s in AmdGpuSensor::probe() {
            gpus.push(Box::new(s));
        }
        for s in IntelGpuSensor::probe() {
            gpus.push(Box::new(s));
        }
        for s in AppleGpuSensor::probe() {
            gpus.push(Box::new(s));
        }

        // Startup sanity-check log (goes to stderr so it's visible in the
        // terminal when running `cargo tauri dev` but never reaches the UI).
        eprintln!("[SysCtrl] detected {} GPU(s):", gpus.len());
        for (i, gpu) in gpus.iter().enumerate() {
            eprintln!("[SysCtrl]   GPU {}: {}", i, gpu.name());
        }
    }

    // ── CPU probing ──────────────────────────────────────────────────────
    #[cfg(target_os = "linux")]
    let cpu = Box::new(CpuSensorImpl::new()) as Box<dyn CpuSensor>;

    #[cfg(not(target_os = "linux"))]
    let cpu = Box::new(NoopCpuSensor) as Box<dyn CpuSensor>;

    // ── Fan probing ──────────────────────────────────────────────────────
    #[cfg(target_os = "linux")]
    let fan = Box::new(SysfsFanController::probe()) as Box<dyn FanController>;

    #[cfg(not(target_os = "linux"))]
    let fan = Box::new(NoopFanController) as Box<dyn FanController>;

    // ── Disk probing ─────────────────────────────────────────────────────
    #[cfg(target_os = "linux")]
    let disks = Box::new(SysinfoDiskSensor::new()) as Box<dyn DiskSensor>;

    #[cfg(not(target_os = "linux"))]
    let disks = Box::new(NoopDiskSensor) as Box<dyn DiskSensor>;

    DetectedBackends { cpu, gpus, fan, disks }
}