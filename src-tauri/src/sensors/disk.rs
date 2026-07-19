// Disk sensor implementation using the `sysinfo` crate.
//
// Reports each mounted physical disk (or, more precisely, each mounted
// filesystem backed by a real block device) with its model name, capacity,
// and used bytes.  Throughput (read/write MB/s) is left as `None` for now —
// it would require sampling `/proc/diskstats` deltas and is out of scope for
// the first version.

#[cfg(target_os = "linux")]
use crate::sensors::{DiskReading, DiskSensor};
#[cfg(target_os = "linux")]
use sysinfo::Disks;

/// Disk sensor backed by `sysinfo::Disks`.
#[cfg(target_os = "linux")]
pub struct SysinfoDiskSensor;

#[cfg(target_os = "linux")]
impl SysinfoDiskSensor {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(target_os = "linux")]
impl Default for SysinfoDiskSensor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "linux")]
impl DiskSensor for SysinfoDiskSensor {
    fn read_all(&self) -> anyhow::Result<Vec<DiskReading>> {
        let mut disks = Disks::new_with_refreshed_list();

        let mut readings = Vec::with_capacity(disks.len());

        for disk in &disks {
            let total_bytes = disk.total_space();
            let available_bytes = disk.available_space();
            let used_bytes = total_bytes.saturating_sub(available_bytes);

            let dev_path = disk.name().to_string_lossy().to_string();
            let device = dev_path.trim_start_matches("/dev/").to_string();

            let mount = disk
                .mount_point()
                .to_string_lossy()
                .to_string();

            // Model isn't directly available from sysinfo; we read it from sysfs
            // via the base device name when possible.
            let model = read_disk_model(&device);

            readings.push(DiskReading {
                device,
                model,
                mount: Some(mount),
                total_bytes,
                used_bytes,
                read_rate_mbps: None,
                write_rate_mbps: None,
            });
        }

        Ok(readings)
    }
}

/// Best-effort disk model string from `/sys/block/<dev>/device/model`.
///
/// `<dev>` here may include a partition suffix (e.g. "nvme0n1p2"); we strip the
/// trailing partition number to reach the parent block device ("nvme0n1").
#[cfg(target_os = "linux")]
fn read_disk_model(device: &str) -> Option<String> {
    // Strip partition suffix: nvme0n1p2 -> nvme0n1, sda1 -> sda.
    let base = strip_partition_suffix(device);

    let model_path = format!("/sys/block/{}/device/model", base);
    std::fs::read_to_string(&model_path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Remove a trailing partition number from a block-device name.
#[cfg(target_os = "linux")]
fn strip_partition_suffix(device: &str) -> String {
    // NVMe: "nvme0n1p2" -> "nvme0n1" (partition marker is 'p' + digits).
    if let Some(idx) = device.rfind('p') {
        let (head, tail) = device.split_at(idx);
        if !tail[1..].is_empty() && tail[1..].chars().all(|c| c.is_ascii_digit()) {
            return head.to_string();
        }
    }
    // SCSI/SATA: "sda1" -> "sda".
    device
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .to_string()
}
