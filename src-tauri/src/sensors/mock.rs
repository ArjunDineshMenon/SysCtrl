// Mock sensor backends for local development and CI testing.
//
// Run with:  cargo tauri dev -- -- --mock
//
// All readings oscillate on a sine wave derived from the current wall-clock
// second so the UI visibly animates without needing real hardware.  The
// period is 30 s, giving a smooth cycle you can observe during a short test
// session.
//
// Mock topology:
//   CPU        : 8-core, usage 10–90%, temp correlated with usage
//   GPU 0      : "Mock NVIDIA RTX 4090", usage 5–70%, 24 GB VRAM
//   GPU 1      : "Mock AMD RX 7900 XTX", usage 2–50%, 20 GB VRAM
//   Fan 0      : "cpu_fan"     — writable, 800–2400 RPM, 20–80%
//   Fan 1      : "gpu_fan"     — writable, 400–1800 RPM, 10–60%
//   Fan 2      : "chassis_fan" — NOT writable (tests disabled-slider path)

use crate::sensors::{
    CpuReading, CpuSensor, DetectedBackends, FanController, FanReading, GpuReading, GpuSensor,
};
use anyhow::Result;
use std::f64::consts::TAU; // 2π
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Time helper
// ---------------------------------------------------------------------------

/// Seconds elapsed since the Unix epoch as an f64.
fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// A sine value in [0, 1] that completes one cycle every `period_secs`.
/// `phase_offset` shifts the wave so different sensors don't peak together.
fn sine01(period_secs: f64, phase_offset: f64) -> f64 {
    (((now_secs() + phase_offset) / period_secs * TAU).sin() + 1.0) / 2.0
}

/// Linearly interpolate between `lo` and `hi` using a [0,1] value.
fn lerp(lo: f64, hi: f64, t: f64) -> f64 {
    lo + (hi - lo) * t.clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// MockCpuSensor
// ---------------------------------------------------------------------------

pub struct MockCpuSensor;

impl CpuSensor for MockCpuSensor {
    fn read(&self) -> Result<CpuReading> {
        let usage_t = sine01(30.0, 0.0); // 30 s period, no phase shift
        let usage_percent = lerp(10.0, 90.0, usage_t) as f32;

        // Temperature loosely tracks usage: idle ~38°C, loaded ~88°C
        let temp_t = lerp(0.2, 1.0, usage_t as f64); // temp lags usage a bit
        let temp_celsius = lerp(38.0, 88.0, temp_t) as f32;

        // Frequency scales with load: 1.8 GHz idle → 4.2 GHz loaded (MHz)
        let freq_mhz = lerp(1800.0, 4200.0, usage_t) as u32;

        Ok(CpuReading {
            usage_percent,
            temp_celsius: Some(temp_celsius),
            freq_mhz: Some(freq_mhz),
            core_count: 8,
        })
    }
}

// ---------------------------------------------------------------------------
// MockGpuSensor  (two instances with different names / characteristics)
// ---------------------------------------------------------------------------

pub struct MockGpuSensor {
    /// Display name, e.g. "Mock NVIDIA RTX 4090"
    pub gpu_name: &'static str,
    /// Peak usage percent (different GPUs have different load profiles)
    pub peak_usage: f64,
    /// VRAM total in MiB
    pub vram_total_mb: u32,
    /// Phase offset (seconds) so the two GPUs don't oscillate in sync
    pub phase: f64,
}

impl GpuSensor for MockGpuSensor {
    fn read(&self) -> Result<GpuReading> {
        let t = sine01(30.0, self.phase);
        let usage_percent = lerp(5.0, self.peak_usage, t) as f32;

        // Temperature: 35–85°C range, correlated with usage
        let temp_celsius = lerp(35.0, 85.0, t) as f32;

        // VRAM used: proportional-ish to usage (25–80% of total)
        let vram_used_mb = lerp(
            self.vram_total_mb as f64 * 0.25,
            self.vram_total_mb as f64 * 0.80,
            t,
        ) as u32;

        Ok(GpuReading {
            name: self.gpu_name.to_owned(),
            usage_percent: Some(usage_percent),
            temp_celsius: Some(temp_celsius),
            vram_used_mb: Some(vram_used_mb),
            vram_total_mb: Some(self.vram_total_mb),
        })
    }

    fn name(&self) -> &str {
        self.gpu_name
    }
}

// ---------------------------------------------------------------------------
// MockFanController  (3 fans; fan 2 is read-only)
// ---------------------------------------------------------------------------

pub struct MockFanController;

impl FanController for MockFanController {
    fn read_all(&self) -> Result<Vec<FanReading>> {
        // Fan 0 — CPU fan, writable
        let cpu_t = sine01(30.0, 0.0);
        let cpu_pct = lerp(20.0, 80.0, cpu_t) as u8;
        let cpu_rpm = lerp(800.0, 2400.0, cpu_t) as u32;

        // Fan 1 — GPU fan, writable, slightly slower oscillation
        let gpu_t = sine01(30.0, 7.5); // phase-shifted by 7.5 s
        let gpu_pct = lerp(10.0, 60.0, gpu_t) as u8;
        let gpu_rpm = lerp(400.0, 1800.0, gpu_t) as u32;

        // Fan 2 — chassis fan, NOT writable (tests disabled-slider path)
        let ch_t = sine01(30.0, 15.0); // phase-shifted by 15 s
        let ch_rpm = lerp(300.0, 900.0, ch_t) as u32;
        let ch_pct = lerp(10.0, 30.0, ch_t) as u8;

        Ok(vec![
            FanReading {
                label: "cpu_fan".to_owned(),
                rpm: Some(cpu_rpm),
                percent: Some(cpu_pct),
                writable: true,
            },
            FanReading {
                label: "gpu_fan".to_owned(),
                rpm: Some(gpu_rpm),
                percent: Some(gpu_pct),
                writable: true,
            },
            FanReading {
                label: "chassis_fan".to_owned(),
                rpm: Some(ch_rpm),
                percent: Some(ch_pct),
                writable: false, // <-- disabled-slider test
            },
        ])
    }

    fn set_percent(&self, fan_label: &str, percent: u8) -> Result<()> {
        // In mock mode we just log the call — nothing actually changes.
        eprintln!(
            "[SysCtrl::mock] set_percent({:?}, {}) — no-op in mock mode",
            fan_label, percent
        );
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Public constructor
// ---------------------------------------------------------------------------

/// Build a `DetectedBackends` populated with mock sensors.
///
/// Called from `main.rs` when the `--mock` CLI flag is present.
pub fn mock_backends() -> DetectedBackends {
    eprintln!("[SysCtrl] *** MOCK MODE — using synthetic sensor data ***");

    let cpu = Box::new(MockCpuSensor) as Box<dyn CpuSensor>;

    let gpus: Vec<Box<dyn GpuSensor>> = vec![
        Box::new(MockGpuSensor {
            gpu_name: "Mock NVIDIA RTX 4090",
            peak_usage: 70.0,
            vram_total_mb: 24_576, // 24 GB
            phase: 0.0,
        }),
        Box::new(MockGpuSensor {
            gpu_name: "Mock AMD RX 7900 XTX",
            peak_usage: 50.0,
            vram_total_mb: 20_480, // 20 GB
            phase: 10.0, // offset so it doesn't mirror GPU 0 exactly
        }),
    ];

    let fan = Box::new(MockFanController) as Box<dyn FanController>;

    DetectedBackends { cpu, gpus, fan }
}
