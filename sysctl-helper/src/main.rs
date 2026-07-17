//! sysctl-helper — privileged CLI for SysCtrl fan control.
//!
//! Usage:
//!     sysctl-helper set-fan <pwm_sysfs_path> <value_0_to_255>
//!
//! This binary is meant to be installed at `/usr/local/bin/sysctl-helper` and
//! invoked via `pkexec` (polkit) from the unprivileged GUI, *or* installed
//! with the setuid bit so it can write to hwmon sysfs without a password
//! prompt each time.
//!
//! SECURITY: The path argument is strictly validated to prevent this binary
//! from being used as an arbitrary-file-write primitive.  Only paths that:
//!   1. Start with `/sys/class/hwmon/` or `/sys/class/drm/`
//!   2. End with a filename matching `pwm[0-9]+`
//! are accepted.

use std::env;
use std::fs;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        usage();
    }

    match args[1].as_str() {
        "set-fan" => cmd_set_fan(&args[2..]),
        other => {
            eprintln!("error: unknown command '{}'", other);
            eprintln!();
            usage();
        }
    }
}

/// `set-fan <pwm_sysfs_path> <value_0_to_255>`
fn cmd_set_fan(args: &[String]) {
    if args.len() != 2 {
        eprintln!("error: set-fan requires exactly 2 arguments");
        eprintln!("usage: sysctl-helper set-fan <pwm_sysfs_path> <value_0_to_255>");
        process::exit(1);
    }

    let pwm_path = &args[0];
    let value_str = &args[1];

    // ── Validate the sysfs path ──────────────────────────────────────────
    validate_pwm_path(pwm_path);

    // ── Validate the PWM value ───────────────────────────────────────────
    let value: u32 = match value_str.parse() {
        Ok(v) if v <= 255 => v,
        Ok(v) => {
            eprintln!("error: value {} is out of range (must be 0-255)", v);
            process::exit(1);
        }
        Err(_) => {
            eprintln!("error: '{}' is not a valid integer", value_str);
            process::exit(1);
        }
    };

    // ── Write to sysfs ───────────────────────────────────────────────────
    if let Err(e) = fs::write(pwm_path, value.to_string()) {
        eprintln!("error: failed to write to '{}': {}", pwm_path, e);
        process::exit(1);
    }

    // Success — print nothing, exit 0.
}

/// Validate that `path` is a legitimate hwmon PWM sysfs file.
///
/// Accepts paths that:
///  - Start with `/sys/class/hwmon/` **or** `/sys/class/drm/`
///  - End with a filename component matching `pwm[0-9]+` (e.g. `pwm1`, `pwm12`)
///
/// This is intentionally strict to prevent privilege-escalation via arbitrary
/// file writes.  We do NOT follow symlinks, resolve `..` components, or accept
/// any path that doesn't exactly match the pattern.
fn validate_pwm_path(path: &str) {
    // 1. Reject paths containing `..` to prevent traversal attacks.
    if path.contains("..") {
        eprintln!("error: path must not contain '..'");
        process::exit(1);
    }

    // 2. Must start with an allowed sysfs prefix.
    let allowed_prefixes = ["/sys/class/hwmon/", "/sys/class/drm/"];
    if !allowed_prefixes.iter().any(|pfx| path.starts_with(pfx)) {
        eprintln!(
            "error: path must start with /sys/class/hwmon/ or /sys/class/drm/, got '{}'",
            path
        );
        process::exit(1);
    }

    // 3. The final filename component must match `pwm[0-9]+`.
    let filename = match path.rsplit('/').next() {
        Some(f) if !f.is_empty() => f,
        _ => {
            eprintln!("error: path has no filename component");
            process::exit(1);
        }
    };

    if !is_valid_pwm_filename(filename) {
        eprintln!(
            "error: filename '{}' does not match pwm[0-9]+ pattern",
            filename
        );
        process::exit(1);
    }
}

/// Returns `true` if `name` matches the pattern `^pwm[0-9]+$`.
///
/// Manual check — avoids pulling in the `regex` crate for a single pattern.
fn is_valid_pwm_filename(name: &str) -> bool {
    let rest = match name.strip_prefix("pwm") {
        Some(r) => r,
        None => return false,
    };

    // Must have at least one digit, and every char must be a digit.
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

/// Print usage and exit.
fn usage() -> ! {
    eprintln!("sysctl-helper — privileged helper for SysCtrl fan control");
    eprintln!();
    eprintln!("usage:");
    eprintln!("  sysctl-helper set-fan <pwm_sysfs_path> <value_0_to_255>");
    eprintln!();
    eprintln!("example:");
    eprintln!("  sysctl-helper set-fan /sys/class/hwmon/hwmon3/pwm1 128");
    process::exit(1);
}
