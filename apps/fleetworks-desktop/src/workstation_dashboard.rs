//! Optional gateway host metrics for workstation telemetry.

use serde::Deserialize;

use crate::app::{Load, State};
use crate::connection::RequestTask;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct HostMetrics {
    pub cpu_percent: Option<f64>,
    pub memory_percent: Option<f64>,
    pub disk_percent: Option<f64>,
    pub network_kbps: Option<f64>,
    pub platform: Option<String>,
    pub chipset: Option<String>,
}

fn format_metrics_age(seconds: f64) -> String {
    let seconds = seconds as u64;
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

impl State {
    pub(crate) fn apply_host_metrics(&mut self, result: Result<HostMetrics, String>) {
        match result {
            Ok(metrics) => {
                self.host_metrics = Load::Ready(metrics);
                self.host_metrics_observed_at = Some(self.clock);
                self.host_metrics_error = None;
            }
            Err(error) => {
                // Keep the last good values, but remember that they are no longer current.
                if matches!(self.host_metrics, Load::Ready(_)) {
                    self.host_metrics_error = Some(error);
                } else {
                    self.host_metrics = Load::Failed(error);
                }
            }
        }
    }

    /// Observation age and refresh error for retained toolbar telemetry.
    pub fn host_metrics_stale_notice(&self) -> Option<String> {
        let error = self.host_metrics_error.as_ref()?;
        let age = self
            .host_metrics_observed_at
            .map(|at| format_metrics_age((self.clock - at).max(0.0)));
        Some(match age {
            Some(age) => format!("Stale: last read {age} ago. Latest refresh failed: {error}"),
            None => format!("Stale: latest refresh failed: {error}"),
        })
    }

    pub fn refresh_host_metrics(&mut self) {
        if !self.connected()
            || !self.permission(fleetworks_contracts::context::Permission::ReadBackend)
            || self.host_metrics_request.is_some()
        {
            return;
        }
        let (Some(services), Some(client)) = (&self.services, &self.client) else {
            return;
        };
        let Some(ui) = services.ui.get().cloned() else {
            return;
        };
        let client = client.clone();
        let epoch = self.connection.epoch;
        self.host_metrics_revision = self.host_metrics_revision.wrapping_add(1);
        let revision = self.host_metrics_revision;
        if !matches!(self.host_metrics, Load::Ready(_)) {
            self.host_metrics = Load::Loading;
        }
        let task = services.runtime.spawn(async move {
            let result: Result<HostMetrics, _> = client.get("/api/v1/host/metrics").await;
            ui.update(move |state, _| {
                if state.connection.epoch != epoch || state.host_metrics_revision != revision {
                    return;
                }
                state.host_metrics_request = None;
                state.apply_host_metrics(result.map_err(|error| error.to_string()));
            });
        });
        self.host_metrics_request = Some(RequestTask(task.abort_handle()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_metrics_keep_last_values_and_flag_them_stale_until_recovery() {
        let metrics = || HostMetrics {
            cpu_percent: Some(10.0),
            ..HostMetrics::default()
        };
        let mut state = State::fixture();

        state.clock = 10.0;
        state.apply_host_metrics(Ok(metrics()));
        assert!(state.host_metrics_stale_notice().is_none());

        state.clock = 75.0;
        state.apply_host_metrics(Err("gateway timed out".into()));
        assert!(matches!(&state.host_metrics, Load::Ready(m) if m.cpu_percent == Some(10.0)));
        let notice = state
            .host_metrics_stale_notice()
            .expect("stale after failure");
        assert!(notice.contains("1m 5s"), "{notice}");
        assert!(notice.contains("gateway timed out"), "{notice}");

        state.clock = 80.0;
        state.apply_host_metrics(Ok(metrics()));
        assert!(state.host_metrics_stale_notice().is_none());
        assert_eq!(state.host_metrics_observed_at, Some(80.0));
    }

    #[test]
    fn first_host_metrics_failure_is_a_failed_read_not_stale_values() {
        let mut state = State::fixture();
        state.apply_host_metrics(Err("unreachable".into()));
        assert!(matches!(&state.host_metrics, Load::Failed(_)));
        assert!(state.host_metrics_stale_notice().is_none());
    }
}
