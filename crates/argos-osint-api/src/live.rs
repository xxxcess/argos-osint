//! Rust-owned live telemetry and active-brief notifications. @tag:host-metrics @tag:live-updates

use std::sync::Arc;
use std::time::Duration;

use argos_osint_core::intel_recon::brief_canonical::IndexState;
use argos_osint_core::intel_service;
use argos_osint_core::storage;
use axum::extract::ws::CloseFrame;
use axum::extract::ws::Message;
use axum::extract::ws::WebSocket;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::Extension;
use axum::extract::State;
use axum::response::Response;
use serde::Deserialize;
use serde_json::json;
use serde_json::Value;
use tokio::time::MissedTickBehavior;

use crate::host_metrics::HostFeed;
use crate::host_metrics::HostReading;
use crate::ApiError;
use crate::ApiState;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct BriefKey {
    run: String,
    id: String,
    #[serde(default)]
    watch: String,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Subscribe {
        #[serde(default)]
        metrics: bool,
        brief: Option<BriefKey>,
    },
}

struct BriefWatch {
    key: BriefKey,
    active: bool,
    previous: String,
    refresh_key: String,
}

pub(crate) async fn upgrade(
    State(state): State<ApiState>,
    Extension(feed): Extension<Arc<HostFeed>>,
    websocket: WebSocketUpgrade,
) -> Response {
    websocket
        .max_message_size(16 * 1024)
        .max_frame_size(16 * 1024)
        .on_upgrade(move |socket| stream(socket, state, feed))
}

fn stage_open(state: &str) -> bool {
    matches!(
        state,
        "queued" | "running" | "retry_scheduled" | "waiting" | "pending"
    )
}

fn pending(stages_open: bool, indexing: bool) -> bool {
    // A job row left at `running` after its stages finish is not a queued brief.
    // Indexing is reported on its own and does not keep the chain queued.
    stages_open || indexing
}

/// Read only the bounded lifecycle projection, never the full evidence/topology DTO.
async fn progress(state: &ApiState, key: &BriefKey) -> Result<(Value, String, bool), ApiError> {
    let mut response = state.repo.db().query(
        "RETURN (SELECT record::id(id) AS job_id, generation FROM intel_brief_run WHERE article_id = $article ORDER BY generation DESC LIMIT 1)[0].job_id;
         RETURN type::record('intel_brief_index_state', $article).state_json;",
    ).bind(("article", key.id.clone())).await.map_err(crate::internal)?.check().map_err(crate::internal)?;
    let job_id: Option<String> = response.take(0).map_err(crate::internal)?;
    let raw_index: Option<String> = response.take(1).map_err(crate::internal)?;
    let index = raw_index
        .map(|raw| serde_json::from_str::<IndexState>(&raw))
        .transpose()
        .map_err(crate::internal)?
        .filter(|index| Some(&index.job_id) == job_id.as_ref());
    let (stages, job_state) = if let Some(job_id) = &job_id {
        // Heal chains whose tasks finished while the job row stayed `running`.
        let _ = storage::jobs::settle_finished_job(state.repo.db(), job_id).await;
        let mut stages = crate::safe_stages(&state.repo, job_id).await?;
        for stage in &mut stages {
            stage.row = stage.row.chars().take(512).collect();
            stage.detail = stage.detail.chars().take(1024).collect();
        }
        let job = storage::jobs_view::get_job(state.repo.db(), job_id)
            .await
            .map_err(crate::internal)?;
        (stages, job.map(|job| job.state))
    } else {
        (Vec::new(), None)
    };
    let indexing_pending = index
        .as_ref()
        .is_some_and(|index| index.state == "indexing");
    let active = pending(
        stages.iter().any(|stage| stage_open(&stage.state)),
        indexing_pending,
    );
    let refresh_key = json!([
        job_id,
        job_state,
        stages
            .iter()
            .map(|stage| (&stage.operation, &stage.state))
            .collect::<Vec<_>>(),
        index
            .as_ref()
            .map(|index| (&index.state, &index.snapshot_id)),
    ])
    .to_string();
    let payload = json!({
        "type": "brief", "run": key.run, "id": key.id, "job_id": job_id,
        "stages": stages, "complete": !active,
        "indexing": index.map(|index| json!({"state":index.state,"reason":argos_osint_core::provider_diag::sanitize(&index.reason).chars().take(1024).collect::<String>()})),
    });
    Ok((payload, refresh_key, active))
}

async fn send(socket: &mut WebSocket, payload: Value) -> bool {
    tokio::time::timeout(
        Duration::from_secs(5),
        socket.send(Message::Text(payload.to_string().into())),
    )
    .await
    .is_ok_and(|result| result.is_ok())
}

async fn stream(mut socket: WebSocket, state: ApiState, feed: Arc<HostFeed>) {
    let mut metrics: Option<tokio::sync::watch::Receiver<HostReading>> = None;
    let mut brief: Option<BriefWatch> = None;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            message = socket.recv() => {
                let Some(Ok(message)) = message else { break; };
                match message {
                    Message::Text(raw) => {
                        let Ok(Command::Subscribe { metrics: next_metrics, brief: next_brief }) = serde_json::from_str(&raw) else {
                            let _ = socket.send(Message::Close(Some(CloseFrame { code: 1008, reason: "Invalid subscription".into() }))).await;
                            break;
                        };
                        if let Some(key) = &next_brief {
                            if !intel_service::valid_id(&key.run) || !intel_service::valid_id(&key.id) || key.watch.len() > 512 {
                                break;
                            }
                            match intel_service::article(&state.repo, &key.run, &key.id).await {
                                Ok(Some(_)) => {},
                                _ => {
                                    let _ = send(&mut socket, json!({"type":"error","channel":"brief","error":"Article unavailable"})).await;
                                    break;
                                }
                            }
                        }
                        if next_metrics {
                            if metrics.is_none() { metrics = Some(feed.subscribe()); }
                        } else { metrics = None; }
                        brief = next_brief.map(|key| BriefWatch { key, active: true, previous: String::new(), refresh_key: String::new() });
                        tick.reset_immediately();
                    }
                    Message::Close(_) => break,
                    Message::Ping(_) | Message::Pong(_) => {},
                    Message::Binary(_) => break,
                }
            }
            _ = tick.tick() => {
                if let Some(metrics) = &metrics {
                    let snapshot = metrics.borrow().current(std::time::Instant::now());
                    if !send(&mut socket, json!({"type":"metrics","data":snapshot})).await { break; }
                }
                if let Some(watch) = brief.as_mut().filter(|watch| watch.active) {
                    let result = tokio::time::timeout(Duration::from_secs(20), progress(&state, &watch.key)).await;
                    match result {
                        Ok(Ok((mut payload, refresh_key, active))) => {
                            let signature = payload.to_string();
                            if signature != watch.previous || refresh_key != watch.refresh_key {
                                payload["refresh"] = json!(refresh_key != watch.refresh_key);
                                if !send(&mut socket, payload).await { break; }
                                watch.previous = signature;
                            }
                            watch.refresh_key = refresh_key;
                            watch.active = active;
                        }
                        // One slow storage read while a brief is writing is not
                        // a dead subscription. The next tick tries again.
                        Err(_) => {}
                        Ok(Err(_)) => { let _ = send(&mut socket, json!({"type":"error","channel":"brief","error":"Brief progress unavailable"})).await; break; }
                    }
                }
            }
            _ = heartbeat.tick() => {
                if !send(&mut socket, json!({"type":"heartbeat"})).await { break; }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn terminal_failure_does_not_keep_blocked_children_watching() {
        assert!(!super::pending(Some("failed"), true, false));
        assert!(!super::pending(Some("cancelled"), true, true));
        assert!(super::pending(Some("running"), false, false));
        assert!(super::pending(Some("completed"), false, true));
        assert!(!super::pending(Some("completed"), false, false));
    }

    #[test]
    fn only_ongoing_states_keep_a_brief_subscription_active() {
        for state in ["queued", "running", "retry_scheduled", "waiting", "pending"] {
            assert!(super::running(state));
        }
        for state in ["completed", "failed", "cancelled", "blocked"] {
            assert!(!super::running(state));
        }
    }
}
