//! Paused-clock tests for the streamed insight timeout policy, SSE framing and
//! typed progress (spec §6, acceptance case 3/4).

use super::*;
use std::sync::Mutex;

#[derive(Debug)]
struct MockErr;
impl std::fmt::Display for MockErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("connection reset")
    }
}
impl std::error::Error for MockErr {}

enum Step {
    Wait(u64),
    Bytes(Vec<u8>),
    Fail,
}

fn script(steps: Vec<Step>) -> impl futures_util::Stream<Item = Result<Vec<u8>, MockErr>> {
    futures_util::stream::unfold(steps.into_iter(), |mut it| async move {
        loop {
            match it.next()? {
                Step::Wait(s) => tokio::time::sleep(Duration::from_secs(s)).await,
                Step::Bytes(b) => return Some((Ok(b), it)),
                Step::Fail => return Some((Err(MockErr), it)),
            }
        }
    })
}

fn delta(text: &str) -> Step {
    Step::Bytes(
        format!(
            "data: {}\n\n",
            serde_json::json!({"choices":[{"delta":{"content":text}}]})
        )
        .into_bytes(),
    )
}
fn reasoning(text: &str) -> Step {
    Step::Bytes(
        format!(
            "data: {}\n\n",
            serde_json::json!({"choices":[{"delta":{"reasoning":text}}]})
        )
        .into_bytes(),
    )
}
fn finish(reason: &str) -> Step {
    Step::Bytes(
        format!(
            "data: {}\n\n",
            serde_json::json!({"choices":[{"delta":{},"finish_reason":reason}]})
        )
        .into_bytes(),
    )
}
fn done() -> Step {
    Step::Bytes(b"data: [DONE]\n\n".to_vec())
}
fn heartbeat() -> Step {
    Step::Bytes(b": keep-alive\n\n".to_vec())
}

async fn read(
    steps: Vec<Step>,
    deadlines: Deadlines,
) -> (Result<Completion, ProviderFailure>, Vec<AttemptProgress>) {
    let events = std::sync::Arc::new(Mutex::new(Vec::new()));
    let shared = events.clone();
    let sink = move |p: AttemptProgress| shared.lock().unwrap().push(p);
    let mut state = StreamState::default();
    let out = read_stream(
        script(steps),
        deadlines,
        Duration::ZERO,
        &mut state,
        None,
        Some(&sink),
    )
    .await;
    let events = events.lock().unwrap().clone();
    (out, events)
}

#[test]
fn insight_policy_matches_spec_and_ordinary_policy_is_unchanged() {
    let p = Deadlines::insight_stream();
    assert_eq!(p.connect, Duration::from_secs(10));
    assert_eq!(p.first_response, Duration::from_secs(60));
    assert_eq!(p.first_output, Some(Duration::from_secs(180)));
    assert_eq!(p.model_inactivity, Some(Duration::from_secs(90)));
    assert_eq!(p.total, Duration::from_secs(600));
    let o = Deadlines::default();
    assert_eq!(
        (o.connect, o.first_response, o.idle, o.total),
        (
            Duration::from_secs(10),
            Duration::from_secs(60),
            Duration::from_secs(30),
            Duration::from_secs(120)
        )
    );
    assert_eq!(o.first_output, None);
    assert_eq!(Deadlines::native_decision(), o);
}

#[tokio::test(start_paused = true)]
async fn active_stream_beyond_120s_completes_under_insight_policy() {
    // 5 minutes of steady output every 20s: the old 120s total would have cut it.
    let mut steps = vec![];
    for i in 0..15 {
        steps.push(Step::Wait(20));
        steps.push(delta(&format!("{i}")));
    }
    steps.push(finish("stop"));
    steps.push(done());
    let started = tokio::time::Instant::now();
    let (out, events) = read(steps, Deadlines::insight_stream()).await;
    let c = out.expect("completes");
    assert_eq!(c.content, "01234567891011121314");
    assert!(started.elapsed() >= Duration::from_secs(300));
    assert_eq!(events.first(), Some(&AttemptProgress::FirstModelOutput));
    assert!(matches!(
        events.last(),
        Some(AttemptProgress::StreamComplete { .. })
    ));
}

#[tokio::test(start_paused = true)]
async fn same_stream_fails_under_ordinary_total_cap() {
    let mut steps = vec![];
    for i in 0..15 {
        steps.push(Step::Wait(20));
        steps.push(delta(&format!("{i}")));
    }
    steps.push(done());
    let (out, _) = read(steps, Deadlines::ordinary()).await;
    let f = out.unwrap_err();
    assert_eq!(f.category, Category::Timeout);
    assert!(f.message.contains("total deadline"), "{}", f.message);
}

#[tokio::test(start_paused = true)]
async fn first_output_inactivity_and_total_limits_are_distinct() {
    // Heartbeats only: transport idle never trips, the first-output clock does.
    let mut steps = vec![];
    for _ in 0..40 {
        steps.push(Step::Wait(10));
        steps.push(heartbeat());
    }
    let (out, events) = read(steps, Deadlines::insight_stream()).await;
    let f = out.unwrap_err();
    assert!(
        f.message.contains("no model output within 180s"),
        "{}",
        f.message
    );
    assert!(events.iter().all(|e| *e == AttemptProgress::Heartbeat));

    // Output then heartbeat-only silence: inactivity (90s) trips, not total.
    let mut steps = vec![delta("{")];
    for _ in 0..40 {
        steps.push(Step::Wait(10));
        steps.push(heartbeat());
    }
    let (out, _) = read(steps, Deadlines::insight_stream()).await;
    let f = out.unwrap_err();
    assert!(
        f.message.contains("model output inactive for 90s"),
        "{}",
        f.message
    );

    // Continuous output every 60s: inactivity never trips, total (600s) does.
    let mut steps = vec![];
    for _ in 0..20 {
        steps.push(Step::Wait(60));
        steps.push(delta("x"));
    }
    let (out, _) = read(steps, Deadlines::insight_stream()).await;
    let f = out.unwrap_err();
    assert!(f.message.contains("total deadline"), "{}", f.message);
}

#[tokio::test(start_paused = true)]
async fn hidden_reasoning_counts_as_activity_without_leaking_text() {
    let mut steps = vec![];
    for _ in 0..4 {
        steps.push(Step::Wait(60));
        steps.push(reasoning("secret thought"));
    }
    steps.push(delta("{\"ok\":true}"));
    steps.push(finish("stop"));
    steps.push(done());
    let (out, events) = read(steps, Deadlines::insight_stream()).await;
    assert_eq!(out.unwrap().content, "{\"ok\":true}");
    let dbg = format!("{events:?}");
    assert!(
        !dbg.contains("secret"),
        "progress must not carry text: {dbg}"
    );
    assert!(events.contains(&AttemptProgress::Receiving {
        answer_chars: 0,
        reasoning_chars: 14
    }));
}

#[tokio::test(start_paused = true)]
async fn split_utf8_mid_stream_errors_length_and_termination() {
    // A multi-byte char split across two network chunks decodes intact.
    let frame = format!(
        "data: {}\n\n",
        serde_json::json!({"choices":[{"delta":{"content":"café — ☃"}}]})
    )
    .into_bytes();
    let cut = frame.iter().position(|b| *b == 0xC3).unwrap() + 1;
    let steps = vec![
        Step::Bytes(frame[..cut].to_vec()),
        Step::Bytes(frame[cut..].to_vec()),
        finish("stop"),
        done(),
        // Anything after [DONE] is never awaited.
        Step::Wait(10_000),
    ];
    let started = tokio::time::Instant::now();
    let (out, _) = read(steps, Deadlines::insight_stream()).await;
    assert_eq!(out.unwrap().content, "café — ☃");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "[DONE] honored promptly"
    );

    let (out, _) = read(
        vec![delta("{\"a\":"), Step::Fail],
        Deadlines::insight_stream(),
    )
    .await;
    assert_eq!(out.unwrap_err().category, Category::StreamInterrupted);

    let (out, _) = read(
        vec![delta("{\"a\":"), finish("length"), done()],
        Deadlines::insight_stream(),
    )
    .await;
    assert_eq!(out.unwrap_err().category, Category::TokenLimit);

    let (out, _) = read(vec![delta("{\"a\":")], Deadlines::insight_stream()).await;
    assert_eq!(out.unwrap_err().category, Category::PrematureEof);
}

#[tokio::test(start_paused = true)]
async fn receiving_progress_is_throttled_to_about_once_per_second() {
    let mut steps = vec![];
    for _ in 0..50 {
        steps.push(delta("a"));
    }
    steps.push(Step::Wait(2));
    steps.push(delta("b"));
    steps.push(done());
    let (_, events) = read(steps, Deadlines::insight_stream()).await;
    let receiving = events
        .iter()
        .filter(|e| matches!(e, AttemptProgress::Receiving { .. }))
        .count();
    assert_eq!(receiving, 2, "{events:?}");
}
