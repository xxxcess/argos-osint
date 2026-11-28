//! Host profile. CPU, RAM, architecture, and GPU / unified-memory budget.
//!
//! The Apple Silicon path follows the same idea as Odysseus hwfit: there is
//! no discrete VRAM, so the GPU budget is a fraction of unified memory unless
//! `iogpu.wired_limit_mb` is set. Results are cached under `~/.argos`.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sysinfo::{Disks, System};

use crate::paths::{ensure_home, hardware_cache_path};

const CACHE_TTL_SECS: u64 = 60 * 30;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HardwareProfile {
    pub arch: String,
    pub os: String,
    pub cpu_name: String,
    pub logical_cores: usize,
    pub physical_cores: Option<usize>,
    pub cpu_usage: f32,
    pub total_ram_gb: f64,
    pub available_ram_gb: f64,
    pub has_gpu: bool,
    pub gpu_name: Option<String>,
    pub gpu_vram_gb: Option<f64>,
    pub gpu_cores: Option<u32>,
    pub unified_memory: bool,
    pub backend: String,
    pub gpu_error: Option<String>,
    pub disk_total_gb: f64,
    pub disk_available_gb: f64,
    pub probed_at: u64,
}

impl HardwareProfile {
    pub fn unknown() -> Self {
        Self {
            arch: std::env::consts::ARCH.into(),
            os: std::env::consts::OS.into(),
            cpu_name: "probing".into(),
            logical_cores: 0,
            physical_cores: None,
            cpu_usage: 0.0,
            total_ram_gb: 0.0,
            available_ram_gb: 0.0,
            has_gpu: false,
            gpu_name: None,
            gpu_vram_gb: None,
            gpu_cores: None,
            unified_memory: false,
            backend: "cpu".into(),
            gpu_error: None,
            disk_total_gb: 0.0,
            disk_available_gb: 0.0,
            probed_at: 0,
        }
    }

    pub fn one_line(&self) -> String {
        let gpu = if self.has_gpu {
            format!(
                "{} {}{:.1} GB{}",
                self.gpu_name.as_deref().unwrap_or("GPU"),
                if self.unified_memory {
                    "unified "
                } else {
                    "VRAM "
                },
                self.gpu_vram_gb.unwrap_or(0.0),
                self.gpu_cores
                    .map(|c| format!(", {c} cores"))
                    .unwrap_or_default(),
            )
        } else {
            self.gpu_error.clone().unwrap_or_else(|| "no GPU".into())
        };
        format!(
            "{os} {arch} · {cores} cores · {cpu} · RAM {ram:.1} GB ({free:.1} free) · {gpu}",
            os = self.os,
            arch = self.arch,
            cores = self.logical_cores,
            cpu = self.cpu_name,
            ram = self.total_ram_gb,
            free = self.available_ram_gb,
            gpu = gpu,
        )
    }

    pub fn ram_pct(&self) -> u16 {
        if self.total_ram_gb <= 0.0 {
            return 0;
        }
        let used = (self.total_ram_gb - self.available_ram_gb).max(0.0);
        ((used / self.total_ram_gb) * 100.0).clamp(0.0, 100.0) as u16
    }

    pub fn disk_pct(&self) -> u16 {
        if self.disk_total_gb <= 0.0 {
            return 0;
        }
        let used = (self.disk_total_gb - self.disk_available_gb).max(0.0);
        ((used / self.disk_total_gb) * 100.0).clamp(0.0, 100.0) as u16
    }
}

pub fn classify_arch(machine: &str) -> &'static str {
    let m = machine.to_lowercase();
    if m.contains("aarch64") || m.contains("arm64") || m == "arm" {
        "arm64"
    } else if m.contains("x86_64") || m.contains("amd64") {
        "x86_64"
    } else if m.is_empty() {
        std::env::consts::ARCH
    } else {
        "other"
    }
}

/// Metal working-set budget. Matches the Odysseus fractions for 16 / 64 GB
/// machines, and prefers an explicit wired limit when one is set.
pub fn metal_vram_gb(total_gb: f64, wired_limit_mb: Option<u64>) -> f64 {
    if let Some(mb) = wired_limit_mb {
        if mb > 0 {
            return round1(mb as f64 / 1024.0);
        }
    }
    let frac = if total_gb <= 16.0 {
        0.67
    } else if total_gb <= 64.0 {
        0.75
    } else {
        0.80
    };
    round1(total_gb * frac)
}

pub fn parse_apple_gpu_cores(text: &str) -> Option<u32> {
    if let Ok(data) = serde_json::from_str::<serde_json::Value>(text) {
        let displays = data
            .get("SPDisplaysDataType")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        for gpu in displays {
            let model = gpu
                .get("sppci_model")
                .or_else(|| gpu.get("_name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !model.to_lowercase().contains("apple") {
                continue;
            }
            if let Some(cores) = gpu.get("sppci_cores").and_then(|v| v.as_str()) {
                if let Ok(n) = cores.trim().parse::<u32>() {
                    return Some(n);
                }
            }
            if let Some(n) = gpu.get("sppci_cores").and_then(|v| v.as_u64()) {
                return Some(n as u32);
            }
        }
    }
    let re = regex::Regex::new(r"Total Number of Cores:\s*(\d+)").ok()?;
    re.captures(text)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

fn round1(n: f64) -> f64 {
    (n * 10.0).round() / 10.0
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn run(args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(args.first()?);
    if args.len() > 1 {
        cmd.args(&args[1..]);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() && out.stdout.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Fresh physical memory headroom for local workers.
///
/// Count reclaimable cache on macOS at normal memory pressure, and constrain
/// Linux headroom by the cgroup limit when available.
#[must_use]
pub fn available_memory_bytes() -> u64 {
    let mut system = System::new();
    system.refresh_memory();
    #[cfg(target_os = "macos")]
    if let Some(available) = run(&[
        "/usr/sbin/sysctl",
        "-n",
        "kern.memorystatus_vm_pressure_level",
        "hw.pagesize",
        "vm.page_free_count",
        "vm.page_pageable_external_count",
        "vm.page_purgeable_count",
    ])
    .as_deref()
    .and_then(macos_reclaimable_memory_bytes)
    {
        return available.min(system.total_memory());
    }
    let available = system.available_memory().min(system.total_memory());
    if available == 0 {
        return 0;
    }
    system
        .cgroup_limits()
        .map_or(available, |limits| available.min(limits.free_memory))
}

#[cfg(any(target_os = "macos", test))]
fn macos_reclaimable_memory_bytes(output: &str) -> Option<u64> {
    // The pressure sysctl reports the userspace event mask, where normal is 1.
    // Free, file-backed and purgeable pages can be reclaimed without swapping.
    let mut values = output.split_whitespace();
    if values.next()? != "1" {
        return None;
    }
    let page_size: u64 = values.next()?.parse().ok()?;
    let free: u64 = values.next()?.parse().ok()?;
    let file_backed: u64 = values.next()?.parse().ok()?;
    let purgeable: u64 = values.next()?.parse().ok()?;
    if values.next().is_some() || page_size == 0 {
        return None;
    }
    free.checked_add(file_backed)?
        .checked_add(purgeable)?
        .checked_mul(page_size)
}

/// Free bytes available to the current user on the filesystem containing the path.
#[must_use]
pub fn filesystem_available_bytes(path: &std::path::Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let path = CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut output = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: path is NUL terminated and output is valid syscall storage.
        if unsafe { libc::statvfs(path.as_ptr(), output.as_mut_ptr()) } != 0 {
            return None;
        }
        // SAFETY: successful statvfs initialized the complete output.
        let output = unsafe { output.assume_init() };
        // Field widths vary across Unix; retain the u64 overflow boundary.
        let available = u128::from(output.f_bavail);
        let fragment_size = u128::from(output.f_frsize);
        u64::try_from(available * fragment_size).ok()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// Current resident bytes of the process on supported Embedded hosts.
#[must_use]
pub fn process_resident_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        use std::io::Read;

        let mut bytes = String::new();
        std::fs::File::open("/proc/self/statm")
            .ok()?
            .take(513)
            .read_to_string(&mut bytes)
            .ok()?;
        if bytes.len() > 512 {
            return None;
        }
        let pages = bytes.split_whitespace().nth(1)?.parse::<u64>().ok()?;
        // SAFETY: sysconf has no pointer arguments and reports the page size.
        let page_size = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok()?;
        pages.checked_mul(page_size).filter(|bytes| *bytes > 0)
    }
    #[cfg(target_os = "macos")]
    {
        let mut info = std::mem::MaybeUninit::<libc::mach_task_basic_info_data_t>::uninit();
        let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
        // SAFETY: The task port belongs to this process. Successful task_info
        // initializes the buffer and reports the number of integers written.
        #[allow(deprecated)]
        let status = unsafe {
            libc::task_info(
                libc::mach_task_self(),
                libc::MACH_TASK_BASIC_INFO as libc::task_flavor_t,
                info.as_mut_ptr().cast::<libc::integer_t>(),
                &mut count,
            )
        };
        if status != libc::KERN_SUCCESS || count < libc::MACH_TASK_BASIC_INFO_COUNT {
            return None;
        }
        // SAFETY: task_info reported success and the complete result size.
        let bytes = unsafe { info.assume_init() }.resident_size;
        (bytes > 0).then_some(bytes)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    None
}

fn disk_totals() -> (f64, f64) {
    let disks = Disks::new_with_refreshed_list();
    let mut total = 0u64;
    let mut avail = 0u64;
    for disk in disks.list() {
        total = total.saturating_add(disk.total_space());
        avail = avail.saturating_add(disk.available_space());
    }
    (
        round1(total as f64 / 1_073_741_824.0),
        round1(avail as f64 / 1_073_741_824.0),
    )
}

fn probe_nvidia() -> Result<(String, f64, u32), String> {
    let out = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .map_err(|e| format!("nvidia-smi: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(if err.trim().is_empty() {
            format!("nvidia-smi exited {}", out.status)
        } else {
            err.trim().to_string()
        });
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut names = Vec::new();
    let mut vram = 0.0f64;
    let mut count = 0u32;
    for line in text.lines() {
        let mut parts = line.split(',');
        let name = parts.next().unwrap_or("").trim();
        let mem = parts.next().unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }
        names.push(name.to_string());
        if let Ok(mb) = mem.parse::<f64>() {
            vram += mb / 1024.0;
        }
        count += 1;
    }
    if count == 0 {
        return Err("nvidia-smi returned no GPUs".into());
    }
    Ok((names.join(" + "), round1(vram), count))
}

fn probe_apple(total_gb: f64) -> Option<HardwareProfile> {
    if std::env::consts::OS != "macos" {
        return None;
    }
    let arch = run(&["uname", "-m"]).unwrap_or_else(|| std::env::consts::ARCH.into());
    if classify_arch(&arch) != "arm64" {
        return None;
    }
    let brand = run(&["sysctl", "-n", "machdep.cpu.brand_string"])
        .unwrap_or_else(|| "Apple Silicon".into());
    let wired = run(&["sysctl", "-n", "iogpu.wired_limit_mb"]).and_then(|s| s.parse::<u64>().ok());
    let profiler = run(&["system_profiler", "SPDisplaysDataType", "-json"]).unwrap_or_default();
    let cores = parse_apple_gpu_cores(&profiler);
    let vram = metal_vram_gb(total_gb, wired.filter(|n| *n > 0));
    Some(partial_gpu(brand, vram, cores, true, "metal"))
}

fn partial_gpu(
    name: String,
    vram: f64,
    cores: Option<u32>,
    unified: bool,
    backend: &str,
) -> HardwareProfile {
    HardwareProfile {
        arch: String::new(),
        os: String::new(),
        cpu_name: String::new(),
        logical_cores: 0,
        physical_cores: None,
        cpu_usage: 0.0,
        total_ram_gb: 0.0,
        available_ram_gb: 0.0,
        has_gpu: true,
        gpu_name: Some(name),
        gpu_vram_gb: Some(vram),
        gpu_cores: cores,
        unified_memory: unified,
        backend: backend.into(),
        gpu_error: None,
        disk_total_gb: 0.0,
        disk_available_gb: 0.0,
        probed_at: 0,
    }
}

/// Live CPU sample. Call twice with a short sleep if the first reading is 0.
pub fn sample_cpu(sys: &mut System) -> f32 {
    sys.refresh_cpu_usage();
    sys.global_cpu_usage()
}

pub fn profile_fresh() -> HardwareProfile {
    let mut sys = System::new_all();
    sys.refresh_memory();
    sys.refresh_cpu_all();
    std::thread::sleep(std::time::Duration::from_millis(180));
    let cpu_usage = sample_cpu(&mut sys);
    let total_ram_gb = round1(sys.total_memory() as f64 / 1_073_741_824.0);
    let available_ram_gb = round1(sys.available_memory() as f64 / 1_073_741_824.0);
    let logical = sys.cpus().len();
    let cpu_name = sys
        .cpus()
        .first()
        .map(|c| c.brand().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            run(&["sysctl", "-n", "machdep.cpu.brand_string"]).unwrap_or_else(|| "unknown".into())
        });
    let physical = sys.physical_core_count();
    let (disk_total, disk_avail) = disk_totals();

    let mut gpu_error = None;
    let gpu = if let Some(apple) = probe_apple(total_ram_gb) {
        Some(apple)
    } else {
        match probe_nvidia() {
            Ok((name, vram, _count)) => Some(partial_gpu(name, vram, None, false, "cuda")),
            Err(err) => {
                if which("nvidia-smi") {
                    gpu_error = Some(err);
                }
                None
            }
        }
    };

    let arch = classify_arch(std::env::consts::ARCH).to_string();
    let backend = gpu.as_ref().map(|g| g.backend.clone()).unwrap_or_else(|| {
        if arch == "arm64" {
            "cpu_arm".into()
        } else {
            "cpu_x86".into()
        }
    });

    HardwareProfile {
        arch,
        os: std::env::consts::OS.into(),
        cpu_name,
        logical_cores: logical,
        physical_cores: physical,
        cpu_usage,
        total_ram_gb,
        available_ram_gb,
        has_gpu: gpu.as_ref().map(|g| g.has_gpu).unwrap_or(false),
        gpu_name: gpu.as_ref().and_then(|g| g.gpu_name.clone()),
        gpu_vram_gb: gpu.as_ref().and_then(|g| g.gpu_vram_gb),
        gpu_cores: gpu.as_ref().and_then(|g| g.gpu_cores),
        unified_memory: gpu.as_ref().map(|g| g.unified_memory).unwrap_or(false),
        backend,
        gpu_error,
        disk_total_gb: disk_total,
        disk_available_gb: disk_avail,
        probed_at: now_secs(),
    }
}

fn which(bin: &str) -> bool {
    Command::new("which")
        .arg(bin)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn profile_cached(fresh: bool) -> HardwareProfile {
    if !fresh {
        if let Some(hit) = read_cache() {
            return hit;
        }
    }
    let profile = profile_fresh();
    let _ = write_cache(&profile);
    profile
}

/// Non-blocking hardware profile probe dispatched to the blocking thread pool
/// so callers on async runtimes do not freeze executor threads on CPU sampling sleeps.
pub async fn profile_fresh_async() -> HardwareProfile {
    tokio::task::spawn_blocking(profile_fresh)
        .await
        .unwrap_or_else(|_| profile_fresh())
}

/// Non-blocking cached hardware profile fetch.
pub async fn profile_cached_async(fresh: bool) -> HardwareProfile {
    tokio::task::spawn_blocking(move || profile_cached(fresh))
        .await
        .unwrap_or_else(|_| profile_cached(fresh))
}

fn read_cache() -> Option<HardwareProfile> {
    let raw = std::fs::read_to_string(hardware_cache_path()).ok()?;
    let profile: HardwareProfile = serde_json::from_str(&raw).ok()?;
    if now_secs().saturating_sub(profile.probed_at) > CACHE_TTL_SECS {
        return None;
    }
    Some(profile)
}

fn write_cache(profile: &HardwareProfile) -> std::io::Result<()> {
    let _ = ensure_home();
    let path = hardware_cache_path();
    std::fs::write(
        path,
        serde_json::to_string_pretty(profile).unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_headroom_counts_reclaimable_pages_only_in_normal_pressure() {
        let pages = "4096\n65536\n655360\n196608";
        assert_eq!(
            macos_reclaimable_memory_bytes(&format!("1\n{pages}")),
            Some(3584 * 1024 * 1024),
        );
        for pressure in [2, 4] {
            assert_eq!(
                macos_reclaimable_memory_bytes(&format!("{pressure}\n{pages}")),
                None,
            );
        }
        assert_eq!(macos_reclaimable_memory_bytes("1\n4096\n65536"), None);
        assert_eq!(
            macos_reclaimable_memory_bytes("1\n4096\n18446744073709551615\n1\n0"),
            None,
        );
    }

    #[test]
    fn metal_budget_tracks_ram_class() {
        assert_eq!(metal_vram_gb(16.0, None), 10.7);
        assert_eq!(metal_vram_gb(32.0, None), 24.0);
        assert_eq!(metal_vram_gb(128.0, None), 102.4);
        assert_eq!(metal_vram_gb(32.0, Some(20480)), 20.0);
    }

    #[test]
    fn parses_apple_cores_from_json_and_text() {
        let json = r#"{"SPDisplaysDataType":[{"sppci_model":"Apple M4","sppci_cores":"10"}]}"#;
        assert_eq!(parse_apple_gpu_cores(json), Some(10));
        assert_eq!(
            parse_apple_gpu_cores("Chipset Model: Apple M2\nTotal Number of Cores: 8\n"),
            Some(8)
        );
        assert_eq!(parse_apple_gpu_cores("Intel UHD"), None);
    }

    #[test]
    fn arch_buckets() {
        assert_eq!(classify_arch("arm64"), "arm64");
        assert_eq!(classify_arch("x86_64"), "x86_64");
    }

    #[tokio::test]
    async fn profile_fresh_async_produces_valid_profile() {
        let profile = profile_fresh_async().await;
        assert!(!profile.arch.is_empty());
        assert!(!profile.os.is_empty());
    }
}
