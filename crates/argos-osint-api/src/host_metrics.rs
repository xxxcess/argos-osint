//! Host telemetry sampled outside Tokio workers. @tag:host-metrics

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use axum::extract::Extension;
use axum::Json;
use chrono::DateTime;
use chrono::Utc;
use serde::Serialize;
use sysinfo::CpuRefreshKind;
use sysinfo::DiskRefreshKind;
use sysinfo::Disks;
use sysinfo::Networks;
use sysinfo::System;

use crate::ApiError;

/// Measurements describe the Rust API host, never the browser or a remote provider.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct HostMetrics {
    cpu_percent: Option<f64>,
    memory_percent: Option<f64>,
    disk_percent: Option<f64>,
    network_kbps: Option<f64>,
    /// Host OS name from sysinfo. This is not a hostname.
    platform: Option<String>,
    chipset: Option<String>,
}

/// One sampler per router preserves CPU/network deltas across browser polls.
#[derive(Default)]
pub(crate) struct HostSampler {
    networks: Networks,
    sampled_at: Option<Instant>,
    disk_sampled_at: Option<Instant>,
    cpu_counters: Option<CpuCounters>,
    cpu: PercentSample,
    memory: PercentSample,
    disk: PercentSample,
    snapshot: HostMetrics,
}

pub(crate) type SharedSampler = Arc<Mutex<HostSampler>>;

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const DISK_SAMPLE_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) struct TimedHostMetrics {
    pub(crate) cpu: Option<ObservedPercent>,
    pub(crate) memory: Option<ObservedPercent>,
    pub(crate) disk: Option<ObservedPercent>,
    pub(crate) next_sample_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ObservedPercent {
    pub(crate) value: f64,
    pub(crate) observed_at: DateTime<Utc>,
}

#[derive(Default)]
struct PercentSample {
    last: Option<ObservedPercent>,
    available: bool,
}

impl PercentSample {
    fn record(&mut self, value: Option<f64>, observed_at: DateTime<Utc>) {
        self.available = value.is_some();
        if let Some(value) = value {
            self.last = Some(ObservedPercent { value, observed_at });
        }
    }

    fn current(&self) -> Option<ObservedPercent> {
        self.last.filter(|_| self.available)
    }
}

#[derive(Clone, Copy)]
struct CpuCounters {
    busy: u64,
    total: u64,
}

impl CpuCounters {
    fn utilization(self, previous: Self) -> Option<f64> {
        let busy = self.busy.checked_sub(previous.busy)?;
        let total = self.total.checked_sub(previous.total)?;
        (total > 0 && busy <= total).then(|| (busy as f64 / total as f64) * 100.0)
    }
}

#[derive(Clone, Default)]
pub(crate) struct HostReading {
    metrics: HostMetrics,
    sampled_at: Option<Instant>,
}

impl HostReading {
    pub(crate) fn current(&self, now: Instant) -> HostMetrics {
        if self
            .sampled_at
            .is_some_and(|sampled| now.duration_since(sampled) <= Duration::from_secs(15))
        {
            self.metrics.clone()
        } else {
            HostMetrics::default()
        }
    }
}

/// Connection-independent producer: slow system reads never block socket traffic.
pub(crate) struct HostFeed {
    sampler: SharedSampler,
    samples: tokio::sync::watch::Sender<HostReading>,
    running: AtomicBool,
}

impl HostFeed {
    pub(crate) fn new(sampler: SharedSampler) -> Self {
        let (samples, _) = tokio::sync::watch::channel(HostReading::default());
        Self {
            sampler,
            samples,
            running: AtomicBool::new(false),
        }
    }

    pub(crate) fn subscribe(self: &Arc<Self>) -> tokio::sync::watch::Receiver<HostReading> {
        let receiver = self.samples.subscribe();
        if self
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let feed = self.clone();
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(SAMPLE_INTERVAL);
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tick.tick().await;
                    if feed.samples.receiver_count() == 0 {
                        feed.running.store(false, Ordering::Release);
                        // A subscriber can arrive between the count and release.
                        if feed.samples.receiver_count() > 0
                            && feed
                                .running
                                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                                .is_ok()
                        {
                            continue;
                        }
                        break;
                    }
                    match read(Extension(feed.sampler.clone())).await {
                        Ok(snapshot) => {
                            feed.samples.send_replace(HostReading {
                                metrics: snapshot.0,
                                sampled_at: Some(Instant::now()),
                            });
                        }
                        Err(_) => {
                            feed.samples.send_replace(HostReading::default());
                            feed.running.store(false, Ordering::Release);
                            break;
                        }
                    }
                }
            });
        }
        receiver
    }
}

fn percent(used: u64, total: u64) -> Option<f64> {
    (total > 0).then(|| (used.min(total) as f64 / total as f64) * 100.0)
}

fn network_rate(bytes: u64, elapsed: Duration) -> Option<f64> {
    (elapsed.as_secs_f64() > 0.0).then(|| bytes as f64 / 1024.0 / elapsed.as_secs_f64())
}

#[cfg(target_os = "linux")]
fn cpu_counters() -> Option<CpuCounters> {
    use std::io::BufRead;
    use std::io::BufReader;

    let file = std::fs::File::open("/proc/stat").ok()?;
    let line = BufReader::new(file).lines().next()?.ok()?;
    parse_cpu_counters(&line)
}

#[cfg(target_os = "linux")]
fn parse_cpu_counters(line: &str) -> Option<CpuCounters> {
    let mut fields = line.split_ascii_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    let mut next = || fields.next()?.parse::<u64>().ok();
    let user = next()?;
    let nice = next()?;
    let system = next()?;
    let idle = next()?;
    let io_wait = next()?;
    let irq = next()?;
    let soft_irq = next()?;
    let steal = next()?;
    let busy = user
        .checked_add(nice)?
        .checked_add(system)?
        .checked_add(irq)?
        .checked_add(soft_irq)?;
    // Linux already includes guest and guest_nice in user and nice.
    let total = busy
        .checked_add(idle)?
        .checked_add(io_wait)?
        .checked_add(steal)?;
    Some(CpuCounters { busy, total })
}

#[cfg(target_os = "linux")]
fn memory_percent() -> Option<f64> {
    parse_memory_percent(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

#[cfg(target_os = "linux")]
fn parse_memory_percent(contents: &str) -> Option<f64> {
    let mut total = None;
    let mut available = None;
    for line in contents.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key {
            "MemTotal" => total = value.split_ascii_whitespace().next()?.parse::<u64>().ok(),
            "MemAvailable" => {
                available = value.split_ascii_whitespace().next()?.parse::<u64>().ok()
            }
            _ => {}
        }
        if let (Some(total), Some(available)) = (total, available) {
            return percent(total.checked_sub(available)?, total);
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn mach_host() -> libc::host_t {
    static PORT: std::sync::OnceLock<libc::host_t> = std::sync::OnceLock::new();
    *PORT.get_or_init(|| {
        // SAFETY: This gets the current host port without consuming any caller memory.
        // Reuse this process-lifetime send right across all samplers.
        #[allow(deprecated)]
        unsafe {
            libc::mach_host_self()
        }
    })
}

#[cfg(target_os = "macos")]
fn cpu_counters() -> Option<CpuCounters> {
    let mut sample = libc::host_cpu_load_info {
        cpu_ticks: [0; libc::CPU_STATE_MAX as usize],
    };
    let mut count = libc::HOST_CPU_LOAD_INFO_COUNT;
    // SAFETY: The flavor, output structure, and capacity agree with the Mach ABI.
    let status = unsafe {
        libc::host_statistics(
            mach_host(),
            libc::HOST_CPU_LOAD_INFO,
            (&raw mut sample).cast(),
            &mut count,
        )
    };
    if status != libc::KERN_SUCCESS || count != libc::HOST_CPU_LOAD_INFO_COUNT {
        return None;
    }
    let total = sample.cpu_ticks.into_iter().map(u64::from).sum::<u64>();
    let busy = total - u64::from(sample.cpu_ticks[libc::CPU_STATE_IDLE as usize]);
    Some(CpuCounters { busy, total })
}

#[cfg(target_os = "macos")]
fn memory_percent() -> Option<f64> {
    let mut total = 0_u64;
    let mut total_len = std::mem::size_of_val(&total);
    // SAFETY: The read-only sysctl writes at most total_len bytes into total.
    let status = unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&raw mut total).cast(),
            &mut total_len,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 || total_len != std::mem::size_of_val(&total) {
        return None;
    }
    const COUNT: libc::mach_msg_type_number_t = libc::HOST_VM_INFO64_COUNT;
    // SAFETY: This C structure consists of integer fields and can start zeroed.
    let mut sample = unsafe { std::mem::zeroed::<libc::vm_statistics64>() };
    let mut count = COUNT;
    // SAFETY: The flavor, output structure, and capacity agree with the Mach ABI.
    let status = unsafe {
        libc::host_statistics64(
            mach_host(),
            libc::HOST_VM_INFO64,
            (&raw mut sample).cast(),
            &mut count,
        )
    };
    // Older kernels can return fewer trailing VM counters than the SDK struct
    // contains. The fields used below end at compressor_page_count.
    let required = std::mem::offset_of!(libc::vm_statistics64, compressor_page_count)
        + std::mem::size_of::<libc::natural_t>();
    if status != libc::KERN_SUCCESS
        || (count as usize) * std::mem::size_of::<libc::integer_t>() < required
    {
        return None;
    }
    // Match the existing macOS utilization definition used by sysinfo.
    let used_pages = u64::from(sample.active_count)
        + u64::from(sample.wire_count)
        + u64::from(sample.compressor_page_count)
        + u64::from(sample.speculative_count);
    // SAFETY: This sysconf selector reads the current page size without pointers.
    let page_size = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok()?;
    percent(used_pages.checked_mul(page_size)?, total)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn cpu_counters() -> Option<CpuCounters> {
    None
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn memory_percent() -> Option<f64> {
    None
}

/// Long OS name for the API host, bounded so a noisy sysctl string cannot
/// dominate the status bar. Empty and control characters are dropped.
fn platform_name() -> Option<String> {
    System::long_os_version().and_then(|name| bounded_platform(&name))
}

fn bounded_platform(name: &str) -> Option<String> {
    let name: String = name.chars().filter(|c| !c.is_control()).take(64).collect();
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

impl HostSampler {
    fn sample(&mut self, now: Instant, home: &Path) -> &HostMetrics {
        let elapsed = self.sampled_at.map(|previous| now.duration_since(previous));
        // Multiple clients cannot shorten the delta window or cause expensive resampling.
        if elapsed.is_some_and(|elapsed| elapsed < SAMPLE_INTERVAL) {
            return &self.snapshot;
        }
        let cpu_observed_at = Utc::now();
        self.record_cpu(cpu_counters(), cpu_observed_at);
        let memory_observed_at = Utc::now();
        self.memory.record(memory_percent(), memory_observed_at);
        // Disk capacity changes slowly; enumerating mounts can be expensive.
        if self
            .disk_sampled_at
            .is_none_or(|previous| now.duration_since(previous) >= DISK_SAMPLE_INTERVAL)
        {
            self.refresh_disk(now, home);
        }
        self.networks.refresh(true);
        let (interface_count, network_bytes) = self
            .networks
            .iter()
            .filter(|(name, _)| !name.starts_with("lo"))
            .fold((0_u64, 0_u64), |(count, total), (_, network)| {
                (
                    count + 1,
                    total
                        .saturating_add(network.received())
                        .saturating_add(network.transmitted()),
                )
            });
        self.snapshot.cpu_percent = self.cpu.current().map(|reading| reading.value);
        self.snapshot.memory_percent = self.memory.current().map(|reading| reading.value);
        self.snapshot.network_kbps = elapsed
            .filter(|_| interface_count > 0)
            .and_then(|elapsed| network_rate(network_bytes, elapsed));
        if self.snapshot.platform.is_none() {
            self.snapshot.platform = platform_name();
        }
        if self.snapshot.chipset.is_none() {
            self.snapshot.chipset = Some(chipset_label(std::env::consts::ARCH, host_cores()));
        }
        self.sampled_at = Some(now);
        &self.snapshot
    }

    fn record_cpu(&mut self, counters: Option<CpuCounters>, observed_at: DateTime<Utc>) {
        let value = counters.and_then(|current| {
            self.cpu_counters
                .replace(current)
                .and_then(|previous| current.utilization(previous))
        });
        self.cpu.record(value, observed_at);
    }

    fn refresh_disk(&mut self, now: Instant, home: &Path) {
        let observed_at = Utc::now();
        // On Linux, refreshing a retained disk preserves its old capacity if
        // statvfs fails. Start from an empty list so an unreadable volume stays unknown.
        let disks =
            Disks::new_with_refreshed_list_specifics(DiskRefreshKind::nothing().with_storage());
        self.disk_sampled_at = Some(now);
        let value = disk_percent(&disks, home);
        self.disk.record(value, observed_at);
        self.snapshot.disk_percent = self.disk.current().map(|reading| reading.value);
    }

    fn timed_sample(
        &mut self,
        now: Instant,
        home: &Path,
        disk_required_after_ms: Option<i64>,
    ) -> Option<TimedHostMetrics> {
        self.sample(now, home);
        // The dashboard can reuse a disk reading for 30 seconds. An Inbox turn
        // can request a newer reading, still at a shared one-second cadence.
        if disk_required_after_ms.is_some_and(|boundary| {
            self.disk
                .current()
                .is_none_or(|reading| reading.observed_at.timestamp_millis() <= boundary)
        }) && self
            .disk_sampled_at
            .is_none_or(|previous| now.duration_since(previous) >= SAMPLE_INTERVAL)
        {
            self.refresh_disk(now, home);
        }
        let sampled_at = if disk_required_after_ms.is_some() {
            self.disk_sampled_at?
        } else {
            self.sampled_at?
        };
        Some(TimedHostMetrics {
            cpu: self.cpu.current(),
            memory: self.memory.current(),
            disk: self.disk.current(),
            next_sample_at: sampled_at + SAMPLE_INTERVAL,
        })
    }
}

fn disk_percent(disks: &Disks, home: &Path) -> Option<f64> {
    let disk = disks
        .list()
        .iter()
        .filter(|disk| home.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().components().count())?;
    percent(
        disk.total_space().saturating_sub(disk.available_space()),
        disk.total_space(),
    )
}

pub(crate) fn new_sampler() -> SharedSampler {
    std::sync::Arc::new(std::sync::Mutex::new(HostSampler::default()))
}

pub(crate) async fn sample_current(sampler: SharedSampler) -> Result<HostMetrics, ApiError> {
    tokio::task::spawn_blocking(move || {
        let mut sampler = sampler
            .lock()
            .map_err(|_| crate::internal("Host metrics lock poisoned"))?;
        let home = argos_osint_core::paths::home_dir();
        Ok::<_, ApiError>(sampler.sample(Instant::now(), &home).clone())
    })
    .await
    .map_err(crate::internal)?
}

pub(crate) async fn sample_timed(
    sampler: SharedSampler,
    disk_required_after_ms: Option<i64>,
    deadline: tokio::time::Instant,
) -> Option<TimedHostMetrics> {
    let mut sample = tokio::task::spawn_blocking(move || {
        let mut sampler = sampler.try_lock().ok()?;
        let home = argos_osint_core::paths::home_dir();
        sampler.timed_sample(Instant::now(), &home, disk_required_after_ms)
    });
    match tokio::time::timeout_at(deadline, &mut sample).await {
        Ok(result) => result.ok().flatten(),
        Err(_) => {
            sample.abort();
            None
        }
    }
}

pub(crate) async fn read(
    Extension(sampler): Extension<SharedSampler>,
) -> Result<Json<HostMetrics>, ApiError> {
    Ok(Json(sample_current(sampler).await?))
}

fn host_cores() -> usize {
    let mut system = System::new();
    system.refresh_cpu_list(CpuRefreshKind::nothing());
    let counted = system.cpus().len();
    if counted > 0 {
        return counted;
    }
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(0)
}

/// `x86 · 16 cores` — the host architecture name and logical core count.
fn chipset_label(architecture: &str, cores: usize) -> String {
    let architecture = match architecture.trim() {
        "aarch64" | "arm64" | "arm" => "arm",
        "x86_64" | "i686" | "x86" => "x86",
        other => other,
    };
    if cores == 0 {
        return architecture.to_owned();
    }
    let unit = if cores == 1 { "core" } else { "cores" };
    format!("{architecture} · {cores} {unit}")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    #[test]
    fn stalled_sampling_expires_to_unknown_without_losing_a_measured_zero() {
        let now = std::time::Instant::now();
        let reading = super::HostReading {
            metrics: super::HostMetrics {
                cpu_percent: Some(0.0),
                ..Default::default()
            },
            sampled_at: Some(now),
        };
        assert_eq!(reading.current(now).cpu_percent, Some(0.0));
        assert_eq!(
            reading.current(now + Duration::from_secs(16)),
            super::HostMetrics::default()
        );
    }

    #[test]
    fn rapid_reads_preserve_the_prior_sample_without_refreshing_host_counters() {
        let now = std::time::Instant::now();
        let observed_at = chrono::Utc::now();
        let disk_observed_at = observed_at - chrono::Duration::seconds(4);
        let expected = super::HostMetrics {
            cpu_percent: Some(11.0),
            memory_percent: Some(22.0),
            disk_percent: Some(33.0),
            network_kbps: Some(0.0),
            platform: None,
            chipset: None,
        };
        let mut sampler = super::HostSampler {
            sampled_at: Some(now),
            disk_sampled_at: Some(now),
            snapshot: expected.clone(),
            ..super::HostSampler::default()
        };
        sampler.cpu.record(Some(11.0), observed_at);
        sampler.memory.record(Some(22.0), observed_at);
        sampler.disk.record(Some(33.0), disk_observed_at);
        assert_eq!(
            sampler.sample(now + Duration::from_millis(500), std::path::Path::new("")),
            &expected
        );
        let sample = sampler
            .timed_sample(
                now + Duration::from_millis(500),
                std::path::Path::new(""),
                Some(observed_at.timestamp_millis()),
            )
            .unwrap();
        assert_eq!(sample.cpu.unwrap().observed_at, observed_at);
        assert_eq!(sample.memory.unwrap().observed_at, observed_at);
        assert_eq!(sample.disk.unwrap().observed_at, disk_observed_at);
        assert_eq!(sample.next_sample_at, now + super::SAMPLE_INTERVAL);
    }

    #[test]
    fn first_cpu_delta_is_unavailable_and_failed_refresh_keeps_original_samples() {
        use super::CpuCounters;
        use super::HostSampler;
        use super::ObservedPercent;

        let mut sampler = HostSampler::default();
        let first = chrono::DateTime::from_timestamp(20, 0).unwrap();
        let second = chrono::DateTime::from_timestamp(21, 0).unwrap();
        let failed = chrono::DateTime::from_timestamp(22, 0).unwrap();
        let later = chrono::DateTime::from_timestamp(23, 0).unwrap();
        sampler.record_cpu(
            Some(CpuCounters {
                busy: 10,
                total: 20,
            }),
            first,
        );
        assert!(
            sampler.cpu.current().is_none(),
            "CPU needs two actual counter readings"
        );
        sampler.memory.record(Some(45.0), first);

        sampler.record_cpu(
            Some(CpuCounters {
                busy: 10,
                total: 30,
            }),
            second,
        );
        let idle_reading = ObservedPercent {
            value: 0.0,
            observed_at: second,
        };
        assert_eq!(
            sampler.cpu.current(),
            Some(idle_reading),
            "idle is a measured zero"
        );

        sampler.record_cpu(None, failed);
        sampler.memory.record(None, failed);
        sampler.disk.record(Some(80.0), first);
        sampler.disk.record(super::percent(0, 0), failed);
        assert!(sampler.cpu.current().is_none());
        assert!(sampler.memory.current().is_none());
        assert!(sampler.disk.current().is_none());
        assert_eq!(sampler.cpu.last, Some(idle_reading));
        assert_eq!(
            sampler.memory.last,
            Some(ObservedPercent {
                value: 45.0,
                observed_at: first
            })
        );
        assert_eq!(
            sampler.disk.last,
            Some(ObservedPercent {
                value: 80.0,
                observed_at: first
            })
        );

        sampler.record_cpu(
            Some(CpuCounters {
                busy: 15,
                total: 40,
            }),
            later,
        );
        sampler.memory.record(Some(47.0), later);
        assert_eq!(
            sampler.cpu.current(),
            Some(ObservedPercent {
                value: 50.0,
                observed_at: later
            })
        );
        assert_eq!(
            sampler.memory.current(),
            Some(ObservedPercent {
                value: 47.0,
                observed_at: later
            })
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_counters_require_numeric_samples_and_do_not_double_count_guest_ticks() {
        let cpu = super::parse_cpu_counters("cpu 10 20 30 40 5 6 7 8 9 10").unwrap();
        assert_eq!((cpu.busy, cpu.total), (73, 126));
        assert!(super::parse_cpu_counters("cpu 10 20 30 40").is_none());
        assert!(super::parse_cpu_counters("cpu0 10 20 30 40 5 6 7 8").is_none());
        assert_eq!(
            super::parse_memory_percent("MemTotal: 100 kB\nMemAvailable: 100 kB"),
            Some(0.0)
        );
        assert!(super::parse_memory_percent("MemTotal: 100 kB").is_none());
        assert!(super::parse_memory_percent("MemTotal: 100 kB\nMemAvailable: 101 kB").is_none());
    }

    #[test]
    fn percentages_preserve_zero_and_do_not_invent_unavailable_totals() {
        assert_eq!(super::percent(0, 100), Some(0.0));
        assert_eq!(super::percent(40, 100), Some(40.0));
        assert_eq!(super::percent(120, 100), Some(100.0));
        assert_eq!(super::percent(0, 0), None);
    }

    #[test]
    fn network_rate_uses_the_actual_sampling_window_and_both_traffic_directions() {
        assert_eq!(
            super::network_rate(10_240, Duration::from_secs(5)),
            Some(2.0)
        );
        assert_eq!(super::network_rate(0, Duration::from_secs(5)), Some(0.0));
        assert_eq!(super::network_rate(100, Duration::ZERO), None);
    }

    #[test]
    fn chipset_label_names_the_architecture_and_logical_cores() {
        assert_eq!(super::chipset_label("x86_64", 16), "x86 · 16 cores");
        assert_eq!(super::chipset_label("aarch64", 1), "arm · 1 core");
        assert_eq!(super::chipset_label("riscv64", 0), "riscv64");
    }

    #[test]
    fn platform_name_drops_controls_and_stays_bounded() {
        assert_eq!(super::bounded_platform("  \n"), None);
        assert_eq!(
            super::bounded_platform("macOS\n26"),
            Some("macOS26".to_owned())
        );
        let long = "m".repeat(80);
        assert_eq!(super::bounded_platform(&long).unwrap().chars().count(), 64);
    }
}
