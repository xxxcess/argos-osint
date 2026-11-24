//! The Embedded scorer admits one pinned worker for a bounded decision.

mod pack;

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use argos_osint_api::inbox_qa::decision;
use argos_osint_api::inbox_qa::decision::DecisionChoice;
use argos_osint_api::inbox_qa::decision::DecisionFuture;
use argos_osint_api::inbox_qa::decision::DecisionMenu;
use argos_osint_api::inbox_qa::decision::DecisionRuntime;
use argos_osint_core::hardware;
use argos_osint_core::storage::inbox_qa;
use argos_osint_core::storage::repo::Repository;
use fleetworks_contracts::chat::QaDecisionAvailabilityReason as Reason;
use fleetworks_contracts::chat::QaDecisionPreference as Preference;
use fleetworks_contracts::chat::QaDecisionSettings;
use fleetworks_contracts::chat::QaPhase;
use reqwest::Client;
use serde::Deserialize;
use tokio::process::Command;
use tokio::sync::watch;
use tokio::sync::Mutex;
use tokio::time;

const MAX_MENU: usize = 8;
const MAX_REPLY: usize = 8192;
const NETWORK_DEADLINE: Duration = Duration::from_secs(3);

#[derive(Clone, Copy)]
struct Observation {
    generation: i64,
    status: QaDecisionSettings,
}

pub(crate) struct Supervisor {
    repo: Repository,
    root: PathBuf,
    admission: Arc<Mutex<()>>,
    observed: watch::Sender<Option<Observation>>,
}

impl Supervisor {
    pub(crate) fn new(repo: Repository, root: PathBuf, admission: Arc<Mutex<()>>) -> Self {
        Self {
            repo,
            root,
            admission,
            observed: watch::channel(None).0,
        }
    }

    async fn current(&self, preference: Preference, generation: i64) -> bool {
        inbox_qa::setting(&self.repo).await.is_ok_and(|setting| {
            setting.enabled
                && setting.decision_generation == generation
                && setting.decision_preference == preference
        })
    }

    async fn score_once(
        &self,
        preference: Preference,
        generation: i64,
        menu: DecisionMenu,
    ) -> Result<DecisionChoice, Reason> {
        let qualification = pack::qualified(preference).ok_or(Reason::AwaitingQualification)?;
        let _admission = self
            .admission
            .try_lock()
            .map_err(|_| Reason::InsufficientResources)?;
        if !self.current(preference, generation).await {
            return Ok(DecisionChoice::Unavailable);
        }
        let root = self.root.clone();
        let available = tokio::task::spawn_blocking(hardware::available_memory_bytes)
            .await
            .map_err(|_| Reason::InsufficientResources)?;
        if available.saturating_sub(crate::qa_worker::WORKSTATION_RESERVE)
            < qualification.resident_bytes
        {
            return Err(Reason::InsufficientResources);
        }
        let verified = tokio::task::spawn_blocking(move || pack::verify(&root, qualification))
            .await
            .map_err(|_| Reason::WorkerStartFailed)??;
        let socket = verified.socket_dir.path().join("score.sock");
        let mut command = worker_command(&verified, &socket, qualification.alias);
        let client = socket_client(&socket).map_err(|_| Reason::WorkerStartFailed)?;
        if !self.current(preference, generation).await {
            return Ok(DecisionChoice::Unavailable);
        }
        let mut worker = command.spawn().map_err(|_| Reason::WorkerStartFailed)?;
        let observation = ReadyObservation {
            observed: &self.observed,
            generation,
        };
        let replaced = async {
            while self.current(preference, generation).await {
                time::sleep(Duration::from_millis(200)).await;
            }
        };
        let session = async {
            wait_ready(&client).await;
            observation.ready(preference);
            let body = ask(&client, &menu).await.ok_or(Reason::WorkerStartFailed)?;
            choose(
                &body,
                qualification.alias,
                menu.candidates.len(),
                qualification.threshold,
                qualification.margin,
            )
            .ok_or(Reason::WorkerStartFailed)
        };
        let result = tokio::select! {
            biased;
            _ = replaced => Ok(DecisionChoice::Unavailable),
            _ = crate::qa_worker::guard_resident(worker.id(), qualification.resident_bytes) => {
                Err(Reason::InsufficientResources)
            }
            result = time::timeout(NETWORK_DEADLINE, session) => result.unwrap_or(Err(Reason::WorkerStartFailed)),
        };
        // Keep the admission guard until the scorer has released its resources.
        let _ = worker.kill().await;
        result
    }
}

impl DecisionRuntime for Supervisor {
    fn status(&self, preference: Preference, generation: i64) -> QaDecisionSettings {
        if preference == Preference::RulesOnly {
            return decision::rules_status(preference, None);
        }
        if pack::qualified(preference).is_none() {
            return decision::rules_status(preference, Some(Reason::AwaitingQualification));
        }
        if let Some(previous) = self.observed.borrow().as_ref().filter(|observation| {
            observation.generation == generation && observation.status.preference == preference
        }) {
            return previous.status;
        }
        QaDecisionSettings::new(preference, Preference::RulesOnly, QaPhase::Starting, None)
    }

    fn score(
        &self,
        preference: Preference,
        generation: i64,
        menu: DecisionMenu,
    ) -> DecisionFuture<'_> {
        Box::pin(async move {
            if !(2..=MAX_MENU).contains(&menu.candidates.len())
                || menu.state.len() > 256
                || menu
                    .candidates
                    .iter()
                    .any(|candidate| candidate.len() > 192)
            {
                return DecisionChoice::Unavailable;
            }
            match self.score_once(preference, generation, menu).await {
                Ok(choice) => choice,
                Err(reason) => {
                    if self.current(preference, generation).await {
                        self.observed.send_replace(Some(Observation {
                            generation,
                            status: decision::rules_status(preference, Some(reason)),
                        }));
                    }
                    DecisionChoice::Unavailable
                }
            }
        })
    }
}

struct ReadyObservation<'a> {
    observed: &'a watch::Sender<Option<Observation>>,
    generation: i64,
}

impl ReadyObservation<'_> {
    fn ready(&self, preference: Preference) {
        self.observed.send_replace(Some(Observation {
            generation: self.generation,
            status: QaDecisionSettings::new(preference, preference, QaPhase::Ready, None),
        }));
    }
}

impl Drop for ReadyObservation<'_> {
    fn drop(&mut self) {
        self.observed.send_if_modified(|current| {
            if current.is_some_and(|observation| {
                observation.generation == self.generation
                    && observation.status.phase == QaPhase::Ready
            }) {
                *current = None;
                return true;
            }
            false
        });
    }
}

fn worker_command(pack: &pack::VerifiedPack, socket: &Path, alias: &str) -> Command {
    let mut command = Command::new(&pack.worker);
    command
        .env_clear()
        .env("HOME", pack.socket_dir.path())
        .env("LC_ALL", "C")
        .current_dir(pack.socket_dir.path())
        .arg("--model")
        .arg(&pack.model)
        .arg("--host")
        .arg(socket)
        .args([
            "--alias",
            alias,
            "--offline",
            "--no-webui",
            "--no-mmproj",
            "--no-cache-prompt",
            "--no-slots",
            "--parallel",
            "1",
            "--threads",
            "1",
            "--threads-batch",
            "1",
            "--threads-http",
            "1",
            "--ctx-size",
            "1024",
            "--n-gpu-layers",
            "0",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(target_os = "linux")]
    command.env(
        "LD_LIBRARY_PATH",
        pack.worker.parent().expect("verified worker directory"),
    );
    crate::qa_worker::limit_embedded_worker(&mut command);
    command
}

fn socket_client(socket: &Path) -> reqwest::Result<Client> {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .unix_socket(socket)
        .timeout(NETWORK_DEADLINE)
        .build()
}

async fn wait_ready(client: &Client) {
    loop {
        if let Ok(reply) = client.get("http://localhost/health").send().await {
            if let Some(body) = response_bytes(reply).await {
                if serde_json::from_slice::<Health>(&body).is_ok_and(|health| health.status == "ok")
                {
                    return;
                }
            }
        }
        time::sleep(Duration::from_millis(75)).await;
    }
}

#[derive(Deserialize)]
struct Health {
    status: String,
}

async fn ask(client: &Client, menu: &DecisionMenu) -> Option<Vec<u8>> {
    let mut criteria = menu
        .candidates
        .iter()
        .enumerate()
        .map(|(index, text)| (format!("c{index}"), text.as_str()))
        .collect::<BTreeMap<_, _>>();
    criteria.insert(
        "none".into(),
        "No listed job supplies a supported relationship.",
    );
    let reply = client.post("http://localhost/v1/systemone").json(&serde_json::json!({
        "state": menu.state,
        "questions": {"related": {
            "type": "choice", "criteria": criteria,
            "instructions": "Choose one recorded relationship to this readiness notice, or none."
        }}
    })).send().await.ok()?;
    response_bytes(reply).await
}

async fn response_bytes(mut reply: reqwest::Response) -> Option<Vec<u8>> {
    if reply.status() != reqwest::StatusCode::OK
        || reply.content_length().is_some_and(|n| n > MAX_REPLY as u64)
    {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = reply.chunk().await.ok()? {
        if chunk.len() > MAX_REPLY - body.len() {
            return None;
        }
        body.extend(chunk);
    }
    Some(body)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    model: String,
    answers: Answers,
    usage: Usage,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Answers {
    related: Related,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Usage {
    input_tokens: u16,
    output_tokens: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Related {
    #[serde(rename = "type")]
    answer_type: String,
    choice: String,
    #[serde(deserialize_with = "probabilities")]
    probabilities: BTreeMap<String, f64>,
    confidence: f64,
}

fn probabilities<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> Result<BTreeMap<String, f64>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = BTreeMap<String, f64>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("bounded distinct decision handles")
        }

        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut weights = BTreeMap::new();
            while let Some((handle, weight)) = map.next_entry()? {
                if weights.len() > MAX_MENU || weights.insert(handle, weight).is_some() {
                    return Err(serde::de::Error::custom(
                        "too many or duplicate decision handles",
                    ));
                }
            }
            Ok(weights)
        }
    }
    decoder.deserialize_map(Visitor)
}

fn choose(
    bytes: &[u8],
    model: &str,
    count: usize,
    threshold: f64,
    margin: f64,
) -> Option<DecisionChoice> {
    let reply: Reply = serde_json::from_slice(bytes).ok()?;
    let related = reply.answers.related;
    let weights = related.probabilities;
    let probability = |value: f64| (0.0..=1.0).contains(&value);
    if reply.model != model
        || !(1..=16 * 1024).contains(&reply.usage.input_tokens)
        || reply.usage.output_tokens != 0
        || related.answer_type != "choice"
        || !probability(related.confidence)
        || weights.len() != count + 1
        || !(0..count).all(|index| weights.contains_key(&format!("c{index}")))
        || !weights.contains_key("none")
        || !weights.values().all(|value| probability(*value))
        || (weights.values().sum::<f64>() - 1.0).abs() > 0.0001
    {
        return None;
    }
    let winning = *weights.get(&related.choice)?;
    let second = weights
        .iter()
        .filter_map(|(handle, weight)| (handle != &related.choice).then_some(*weight))
        .fold(0.0, f64::max);
    if winning < second {
        return None;
    }
    if related.choice == "none" || winning < threshold || winning - second < margin {
        return Some(DecisionChoice::Abstain);
    }
    Some(DecisionChoice::Candidate(
        related.choice.strip_prefix('c')?.parse().ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_a_ready_worker_fences_the_old_observation() {
        let (observed, status) = watch::channel(None);
        let julia = ReadyObservation {
            observed: &observed,
            generation: 1,
        };
        julia.ready(Preference::Julia1Q8);
        assert_eq!(
            status.borrow().as_ref().unwrap().status.effective,
            Preference::Julia1Q8,
        );

        let edge = ReadyObservation {
            observed: &observed,
            generation: 2,
        };
        edge.ready(Preference::VllmD3EdgeQ8);
        drop(julia);
        assert_eq!(
            status.borrow().as_ref().unwrap().status.effective,
            Preference::VllmD3EdgeQ8,
        );
        drop(edge);
        assert!(status.borrow().is_none());
    }

    #[tokio::test]
    async fn qualified_fixture_scores_and_abstains_over_a_private_socket() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("decision.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let host = axum::Router::new()
            .route("/health", axum::routing::get(|| async { axum::Json(serde_json::json!({"status":"ok"})) }))
            .route("/v1/systemone", axum::routing::post(|axum::Json(request): axum::Json<serde_json::Value>| async move {
                let pick = request["state"] == "Choose the first case";
                axum::Json(serde_json::json!({
                    "model": "decision-fixture", "usage": {"input_tokens": 48, "output_tokens": 0},
                    "answers": {"related": {
                        "type": "choice", "choice": if pick {"c0"} else {"none"},
                        "probabilities": {"c0": if pick {0.90} else {0.05}, "c1": 0.05, "none": if pick {0.05} else {0.90}},
                        "confidence": 0.90
                    }}
                }))
            }));
        let server = tokio::spawn(async move { axum::serve(listener, host).await });
        let client = socket_client(&socket).unwrap();
        time::timeout(Duration::from_secs(1), wait_ready(&client))
            .await
            .unwrap();
        let mut menu = DecisionMenu {
            state: "Choose the first case".into(),
            candidates: vec!["Candidate one".into(), "Candidate two".into()],
        };
        let body = ask(&client, &menu).await.unwrap();
        assert!(matches!(
            choose(&body, "decision-fixture", 2, 0.80, 0.10),
            Some(DecisionChoice::Candidate(0))
        ));
        assert!(choose(&body, "other-model", 2, 0.80, 0.10).is_none());
        assert!(matches!(
            choose(&body, "decision-fixture", 2, 0.95, 0.10),
            Some(DecisionChoice::Abstain)
        ));
        menu.state = "No supported relation".into();
        let body = ask(&client, &menu).await.unwrap();
        assert!(matches!(
            choose(&body, "decision-fixture", 2, 0.8, 0.1),
            Some(DecisionChoice::Abstain)
        ));
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        assert!(ask(&client, &menu).await.is_none());
    }
}
