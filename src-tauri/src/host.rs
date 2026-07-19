// Host/device identity read using only universal Linux interfaces:
//   • /etc/os-release   (distro name/version — present on every systemd distro
//     and most others; Arch/Alpine provide it too)
//   • the `hostname` command or /etc/hostname
//   • `uname` via sysinfo-independent std (kernel + arch)
//
// No distro-specific tooling is required, so this works across the full
// spectrum of Linux distributions targeted by the global launch.

#[cfg(target_os = "linux")]
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::process::Command;

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostInfo {
    pub hostname: String,
    pub distro: String,
    pub distro_version: Option<String>,
    pub kernel: String,
    pub arch: String,
}

/// Read host identity for the "current device" overview card.
#[cfg(target_os = "linux")]
pub fn read_host_info() -> HostInfo {
    let hostname = read_hostname();
    let (distro, distro_version) = read_os_release();
    let (kernel, arch) = read_uname();
    HostInfo {
        hostname,
        distro,
        distro_version,
        kernel,
        arch,
    }
}

#[cfg(target_os = "linux")]
fn read_hostname() -> String {
    if let Ok(s) = fs::read_to_string("/etc/hostname") {
        let t = s.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(target_os = "linux")]
fn read_os_release() -> (String, Option<String>) {
    let contents = match fs::read_to_string("/etc/os-release") {
        Ok(c) => c,
        Err(_) => return ("Linux".to_string(), None),
    };
    let mut name = None;
    let mut version = None;
    for line in contents.lines() {
        if let Some(v) = line.strip_prefix("PRETTY_NAME=") {
            name = Some(strip_quotes(v));
        } else if name.is_none() {
            if let Some(v) = line.strip_prefix("NAME=") {
                name = Some(strip_quotes(v));
            }
        }
        if let Some(v) = line.strip_prefix("VERSION_ID=") {
            version = Some(strip_quotes(v));
        }
    }
    (name.unwrap_or_else(|| "Linux".to_string()), version)
}

#[cfg(target_os = "linux")]
fn strip_quotes(s: &str) -> String {
    s.trim().trim_matches('"').to_string()
}

#[cfg(target_os = "linux")]
fn read_uname() -> (String, String) {
    let out = Command::new("uname").args(["-srm"]).output().ok();
    if let Some(o) = out {
        if let Ok(s) = String::from_utf8(o.stdout) {
            let parts: Vec<&str> = s.trim().split_whitespace().collect();
            // "Linux 6.14.0 x86_64" -> kernel="6.14.0", arch="x86_64"
            let kernel = parts.get(1).map(|s| s.to_string()).unwrap_or_default();
            let arch = parts.get(2).map(|s| s.to_string()).unwrap_or_default();
            return (kernel, arch);
        }
    }
    ("unknown".to_string(), "unknown".to_string())
}

#[cfg(not(target_os = "linux"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostInfo {
    pub hostname: String,
    pub distro: String,
    pub distro_version: Option<String>,
    pub kernel: String,
    pub arch: String,
}

#[cfg(not(target_os = "linux"))]
pub fn read_host_info() -> HostInfo {
    HostInfo {
        hostname: "unknown".to_string(),
        distro: "unknown".to_string(),
        distro_version: None,
        kernel: "unknown".to_string(),
        arch: "unknown".to_string(),
    }
}
