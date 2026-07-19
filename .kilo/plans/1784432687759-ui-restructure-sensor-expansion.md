# SysCtrl UI Restructure + Sensor Expansion

## Goal
Rebuild the frontend into a single-window, two-view app (Tauri v2, vanilla JS, no new deps):
1. **Overview page** — full static specs + live values for CPU, GPU(s), RAM, fans, storage.
2. **Graphs page** — user picks a component (CPU / GPU / RAM / Disk) and sees its time-series graphs.

Add the backend sensor fields currently missing (GPU clock, RAM model+speed, disk/storage, integrated-vs-discrete GPU flag). Fan **readouts** shown; no fan *control* UI (per user). Consolidate dead files and strip debug `eprintln!` spam. No breaking changes to existing working data paths.

---

## Decisions (confirmed with user)
- **Add all new sensors** (GPU clock, RAM model/speed, disk, integrated flag).
- **In-page view switching** via sidebar (no router lib, no multi-window).
- **Consolidate + strip debug logs**: new UI in `dist/index.html`; delete `sysctrl-dashboard.js`; keep root `index.html` only as a thin dev entry OR delete (see Task 8); remove per-second `eprintln!` debug lines. `--mock` must keep working.

---

## Backend changes (Rust — `src-tauri/src/sensors/`)

### 1. Extend data models (`mod.rs`)
Add to `GpuReading`:
- `clock_mhz: Option<u32>` (core/graphics clock)
- `is_integrated: bool` (true for Intel iGPU / AMD APU / Apple)
- `vram_type: Option<String>` (e.g. "GDDR6") — best-effort, may stay None
Add `RamReading` fields:
- `model: Option<String>` (e.g. "DDR4", "DDR5")
- `speed_mhz: Option<u32>` (e.g. 3200)
Add new structs:
- `DiskReading { device: String, model: Option<String>, mount: Option<String>, total_bytes: u64, used_bytes: u64, read_rate: Option<f32>, write_rate: Option<f32> }`
- `SystemSnapshot.disks: Vec<DiskReading>` (add field, default `vec![]`).
Keep all existing fields/serialization intact (frontend already consumes them).

### 2. GPU clock + integrated flag
- `gpu_nvidia.rs`: add `clock_mhz` via `device.clock_info(nvml_wrapper::enum_wrappers::device::Clock::Graphics, ClockDomain)` (degrade to None). Set `is_integrated=false` (NVIDIA dGPU).
- `gpu_amd.rs`: read `device/pp_dpm_sclk` or `device/hwmon/*/freq1_input` for clock; set `is_integrated=true` if name/PCI implies APU (AMD vendor + no dedicated card is hard to detect — use heuristic: AMD + `modalias`/`class` 0x0300 is discrete; treat all AMD as discrete=false only when APU detected. Simpler: add `is_integrated` param to `probe()` choosing `false` for discrete cards, `true` for known APU. Acceptable: default AMD discrete=`false`, but mark Ryzen APU by checking `cpuinfo`/APU model — keep simple: leave a TODO, default `false` unless easily detectable.)
- `gpu_intel.rs`: `is_integrated=true` (all Intel client GPUs are iGPU/integrated). Clock via `intel_gpu_top` JSON `frequency` or sysfs `gt_act_freq_mhz` (best-effort).
- `gpu_apple.rs`: `is_integrated=true` (unified memory). Clock best-effort None.
- `mock.rs`: add `clock_mhz` (sine 1000–2500), `is_integrated` (GPU0=false, GPU1=false), keep existing. Add mock disks (e.g. "nvme0n1" 500GB used 200GB).

### 3. RAM model + speed (`mod.rs` `read_ram`)
- Read `/sys/devices/system/edac/mc/mc0/dimm0/size` is unreliable; prefer parsing **DMI**. Without a crate, shell out to `dmidecode -t memory` (root may be needed; degrade to None if unavailable/permission denied — do NOT fail the snapshot). Parse: `Type:` → model (DDR4/DDR5), `Speed:` → speed_mhz, `Part Number:`/`Manufacturer:` → model string. Guard with `Command::new("dmidecode")…` and `.ok()` so it never errors the read. Fallback: read `/proc/meminfo` + kernel `dmesg`/`sysfs` best-effort, else None.
- Keep `used_mb/total_mb/percent` logic unchanged.

### 4. New disk sensor (`sensors/disk.rs`, new file, `#[cfg(target_os="linux")]`)
- List block devices: read `/proc/partitions` or `/sys/block` for disks (skip loops/ram).
- Per disk: model from `/sys/block/<dev>/device/model`; total/used via `statvfs` on mountpoint (parse `/proc/mounts` for the disk's mount, prefer `/` and biggest fs). Use Rust std `std::fs::metadata`/`statvfs` via `libc`? Prefer **`sysinfo` (already a dependency, unused)** — use `sysinfo::Disks` to get name/mount/total/available. This is why `sysinfo` is in Cargo.toml; finally use it.
- Throughput (read/write rate): optional — read `/proc/diskstats` deltas in the polling loop. If too complex, set `read_rate/write_rate=None` for v1 (graphs can show usage % only). Keep simple: report capacity + used%, skip rate in first pass (note as TODO in code).
- Add `DiskSensor` trait + `detect_backends()` wiring (`disks` field). `collect_snapshot` adds `sensors::read_disks()`.
- Mock: 1–2 fake disks.

### 5. Remove debug `eprintln!` spam
- `cpu.rs:214` `[DEBUG cpu]` line → remove.
- `gpu_amd.rs` `get_device_name` debug lines (171–199) → remove (keep logic).
- `ipc.rs:86-92` `[DEBUG] cpu=...` block in `get_snapshot` → remove.
- Keep `eprintln!` startup logs (GPU detection list) — those are informational, not per-second.

### 6. Capabilities / config
- No new Tauri commands needed (read-only). `set_fan_speed` stays unused by UI (keep command).
- `csp` left as-is (user chose option without CSP hardening).

---

## Frontend changes (`dist/index.html` — the build target)

### 7. Restructure into two views
- Sidebar: make **Overview** and **Graphs** functional (toggle `.view-overview` / `.view-graphs` visibility; others stay `nav-future` disabled).
- **Overview view**: spec cards/sections:
  - CPU: usage %, clock (GHz), temp, cores, model (best-effort from `/proc/cpuinfo` model name — add small `read_cpu_model()`? optional; show cores·clock for now, add model if easy).
  - GPU(s): iterate `snap.gpus`; for each show **name + "(Integrated)" badge when `is_integrated`**, usage %, clock (GHz), temp, VRAM used/total, type. Explicit "Integrated Graphics" label when flag true.
  - RAM: used/total GB, %, **model (DDR4/5) + speed (MHz)** when present, else "—".
  - Fans: iterate `snap.fans` → label + RPM + duty % (read-only). No sliders/buttons.
  - Storage: iterate `snap.disks` → device + model + used/total GB + % bar.
- **Graphs view**: a selector (dropdown/buttons) for CPU / GPU / RAM / Disk. On selection, show SVG polylines for that component's metrics:
  - CPU → usage %, temp, clock.
  - GPU → usage %, temp, clock (pick first GPU or a GPU selector if >1).
  - RAM → usage %.
  - Disk → used % (per disk or selected disk).
  - Keep history buffers per metric in JS (already have pattern). 1 Hz push feeds them. Range tabs (1m/5m/1h) become functional by scaling how many of the 60+ buffered points are drawn (extend MAX_HISTORY to ~360 for 1h at 1Hz, or downsample).
- Chart legend + 3 gradients already exist; extend for new series.

### 8. Consolidation / dead files
- Build from `dist/index.html` (already `frontendDist: "../dist"`). Rewrite it as the single source of truth.
- Delete `sysctrl-dashboard.js`.
- Root `index.html`: either delete it or replace its body with a redirect/note pointing to `dist/` build. **Recommended: delete it** to avoid confusion, since Tauri serves `dist/index.html`. If a dev wants the old mock, `--mock` still drives `dist/index.html` via backend mock backends (frontend already handles missing data gracefully). Confirm no other reference to root `index.html` exists (grep: only `tauri.conf.json` references `index.html` but under `dist`? verify — `tauri.conf.json` `url: "index.html"` resolves inside `frontendDist` = `dist`, so root `index.html` is NOT used by build).

### 9. Robustness (preserve current working behavior)
- Keep `waitForTauri` retry + initial `get_snapshot` + `sysctrl://snapshot` listener.
- All new fields rendered with `fmt()` null-guards ("—") so missing data never breaks layout.
- No new npm deps; vanilla JS only.

---

## Validation
1. `cd src-tauri && cargo check` (linux) — compiles, no unused-import errors after edits.
2. `cargo tauri dev -- -- --mock` — Overview shows synthetic CPU/GPU/RAM/fan/disk values, Graphs page renders CPU/GPU/RAM/Disk series, range tabs switch, no console errors, no `eprintln!` CPU spam.
3. Toggle `--mock` off on a real Linux host (if available): confirm real CPU freq/temp, GPU (integrated badge for Intel iGPU), RAM, disks appear; fields that can't be read show "—" without crashing.
4. Confirm `dist/index.html` is the only frontend; `sysctrl-dashboard.js` and root `index.html` removed/confirmed-unused.
5. `git status` clean of build artifacts (target/ already gitignored).

## Risks / open questions
- `dmidecode` needs root; on normal user it returns None → RAM model/speed show "—". Acceptable v1; could later add polkit helper. **No snapshot failure.**
- AMD APU vs dGPU detection is heuristic; default conservative (flag may be wrong for some APUs). Frontend just shows the badge from backend.
- Disk throughput (read/write rate) deferred to TODO (capacity + usage only in v1).
- `sysinfo` crate finally used for disks — verify it's in `Cargo.toml` (it is: `sysinfo = "0.35"`).
