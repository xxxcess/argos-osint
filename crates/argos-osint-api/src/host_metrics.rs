//! Host telemetry sampled outside Tokio workers. @tag:host-metrics

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use axum::extract::Extension;
use axum::Json;
use serde::Serialize;
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
}

/// One sampler per router preserves CPU/network deltas across browser polls.
#[derive(Default)]
pub(crate) struct HostSampler {
    system: System,
    disks: Disks,
    networks: Networks,
    sampled_at: Option<Instant>,
    disk_sampled_at: Option<Instant>,
    snapshot: HostMetrics,
}

pub(crate) type SharedSampler = Arc<Mutex<HostSampler>>;

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
                let mut tick = tokio::time::interval(Duration::from_secs(1));
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

impl HostSampler {
    fn sample(&mut self, now: Instant, home: PathBuf) -> HostMetrics {
        let elapsed = self.sampled_at.map(|previous| now.duration_since(previous));
        // Multiple clients cannot shorten the delta window or cause expensive resampling.
        if elapsed.is_some_and(|elapsed| elapsed < Duration::from_secs(1)) {
            return self.snapshot.clone();
        }
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        // Disk capacity changes slowly; enumerating mounts can be expensive.
        if self
            .disk_sampled_at
            .is_none_or(|previous| now.duration_since(previous) >= Duration::from_secs(30))
        {
            self.disks.refresh(true);
            self.disk_sampled_at = Some(now);
        }
        self.networks.refresh(true);
        let cpu = f64::from(self.system.global_cpu_usage());
        let disk = self
            .disks
            .list()
            .iter()
            .filter(|disk| home.starts_with(disk.mount_point()))
            .max_by_key(|disk| disk.mount_point().components().count());
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
        self.snapshot = HostMetrics {
            cpu_percent: elapsed
                .filter(|_| !self.system.cpus().is_empty() && cpu.is_finite())
                .map(|_| cpu.clamp(0.0, 100.0)),
            memory_percent: percent(self.system.used_memory(), self.system.total_memory()),
            disk_percent: disk.and_then(|disk| {
                percent(
                    disk.total_space().saturating_sub(disk.available_space()),
                    disk.total_space(),
                )
            }),
            network_kbps: elapsed
                .filter(|_| interface_count > 0)
                .and_then(|elapsed| network_rate(network_bytes, elapsed)),
        };
        self.sampled_at = Some(now);
        self.snapshot.clone()
    }
}

pub(crate) async fn read(
    Extension(sampler): Extension<SharedSampler>,
) -> Result<Json<HostMetrics>, ApiError> {
    let snapshot = tokio::task::spawn_blocking(move || {
        let mut sampler = sampler
            .lock()
            .map_err(|_| crate::internal("Host metrics lock poisoned"))?;
        let home = argos_osint_core::paths::home_dir();
        Ok::<_, ApiError>(sampler.sample(Instant::now(), home))
    })
    .await
    .map_err(crate::internal)??;
    Ok(Json(snapshot))
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
        let expected = super::HostMetrics {
            cpu_percent: Some(11.0),
            memory_percent: Some(22.0),
            disk_percent: None,
            network_kbps: Some(0.0),
        };
        let mut sampler = super::HostSampler {
            sampled_at: Some(now),
            snapshot: expected.clone(),
            ..super::HostSampler::default()
        };
        assert_eq!(
            sampler.sample(now + Duration::from_millis(500), std::path::PathBuf::new()),
            expected
        );
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
}
