//! Exercise the executable entrypoints against a local HTTP fixture.

use std::path::Path;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;

use anyhow::ensure;
use anyhow::Result;
use serde_json::json;
use serde_json::Value;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::net::TcpListener;

#[test]
fn profile_workspace_is_offline_and_flag_takes_precedence() -> Result<()> {
    let parent = tempfile::tempdir()?;
    let workspace = parent.path().canonicalize()?.join("chosen workspace");
    let output = Command::new(env!("CARGO_BIN_EXE_fleetworks-profile"))
        .env("FLEETWORKS_WORKSPACE", parent.path().join("ignored"))
        .args(["workspace", "--json", "--api-origin", "invalid"])
        .arg("--workspace")
        .arg(&workspace)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout)?,
        json!({"workspace": workspace.display().to_string()})
    );
    assert!(
        !workspace.exists(),
        "the workspace read must not create a directory"
    );
    Ok(())
}

#[tokio::test]
async fn atlas_sends_encoded_query_and_workspace_bearer_to_configured_origin() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    write_private_token(&workspace.path().join("fixture-token"))?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let mut client = tokio::process::Command::new(env!("CARGO_BIN_EXE_fleetworks-atlas"));
    client
        .kill_on_drop(true)
        .arg("--workspace")
        .arg(workspace.path())
        .args([
            "--api-origin",
            &origin,
            "--api-token-file",
            "fixture-token",
            "articles",
            "2026-11-09",
            "--cursor",
            "part &?",
            "--json",
        ]);
    let fixture = respond(&listener, json!({"rows": [], "next_cursor": null}));
    let (output, request) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(client.output(), fixture)
    })
    .await?;
    let output = output?;
    let (request, _) = request?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request_line = request
        .lines()
        .next()
        .expect("HTTP request must have a start line");
    assert!(
        request_line.starts_with("GET /api/v1/articles?"),
        "{request_line}"
    );
    for parameter in [
        "day=2026-11-09",
        "category=all",
        "limit=30",
        "cursor=part+%26%3F",
    ] {
        assert!(
            request_line.contains(parameter),
            "missing {parameter} in {request_line}"
        );
    }
    assert!(
        request
            .lines()
            .any(|line| line.eq_ignore_ascii_case("authorization: Bearer fixture-token")),
        "the configured token must reach the selected API"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout)?,
        json!({"rows":[], "next_cursor":null})
    );
    Ok(())
}

#[tokio::test]
async fn jobs_tui_reads_job_and_logs_and_confirms_mutations() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    write_private_token(&workspace.path().join("fixture-token"))?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let mut client = tokio::process::Command::new(env!("CARGO_BIN_EXE_fleetworks-jobs"));
    client
        .kill_on_drop(true)
        .arg("--workspace")
        .arg(workspace.path())
        .args([
            "--api-origin",
            &origin,
            "--api-token-file",
            "fixture-token",
            "tui",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = async {
        let mut child = client.spawn()?;
        let mut input = child.stdin.take().expect("child stdin must be piped");
        input
            .write_all(b"detail #1\nlogs #1\ncancel #1\nno\nretry #1\nyes\ncancel #1\nyes\nquit\n")
            .await?;
        drop(input);
        Ok::<_, anyhow::Error>(child.wait_with_output().await?)
    };
    let fixture = async {
        let row = json!({
            "id": "atlas-1",
            "app": "atlas",
            "title": "Collect articles",
            "operation": "atlas_cycle",
            "state": "failed",
            "phase": "Fetch sources",
            "progress_done": 1,
            "progress_total": 2,
            "error_summary": "Provider unavailable",
        });
        let requests = [
            (
                "GET /api/v1/jobs HTTP/1.1",
                None,
                json!({"rows": [row.clone()]}),
            ),
            (
                "GET /api/v1/jobs/detail?id=atlas-1 HTTP/1.1",
                None,
                json!({"job": row, "stages": [{"label": "Collect", "row": "Fetch failed"}]}),
            ),
            (
                "GET /api/v1/logs?",
                None,
                json!({"rows": [{"severity": "info", "app": "atlas", "message": "message\u{1b}[31m"}]}),
            ),
            (
                "POST /api/v1/jobs/retry HTTP/1.1",
                Some(json!({"id": "atlas-1"})),
                json!({"requeued": 1}),
            ),
            (
                "POST /api/v1/jobs/cancel HTTP/1.1",
                Some(json!({"id": "atlas-1"})),
                json!({"done": true}),
            ),
        ];
        for (expected_line, expected_body, reply) in requests {
            let (headers, body) = respond(&listener, reply).await?;
            let line = headers.lines().next().expect("HTTP start line");
            if expected_line.ends_with('?') {
                let query = line
                    .strip_prefix(expected_line)
                    .and_then(|line| line.strip_suffix(" HTTP/1.1"))
                    .expect("the log request must include query fields");
                let mut fields: Vec<_> = query.split('&').collect();
                fields.sort_unstable();
                assert_eq!(fields, ["job_id=atlas-1", "limit=50"]);
            } else {
                assert_eq!(line, expected_line);
            }
            assert!(headers
                .lines()
                .any(|line| line.eq_ignore_ascii_case("authorization: Bearer fixture-token")));
            if let Some(expected_body) = expected_body {
                assert_eq!(serde_json::from_slice::<Value>(&body)?, expected_body);
            } else {
                assert!(body.is_empty());
            }
        }
        Ok::<_, anyhow::Error>(())
    };
    let (output, fixture) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(child, fixture)
    })
    .await?;
    fixture?;
    let output = output?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    for expected in [
        "#1  [failed] Collect articles",
        "Progress 1/2",
        "Collect  Fetch failed",
        "message\\u{1b}[31m",
        "No action sent.",
        "Requeued 1 task(s).",
        "Cancellation requested.",
    ] {
        assert!(
            stdout.contains(expected),
            "missing {expected:?} in {stdout:?}"
        );
    }
    assert!(
        !stdout.contains('\u{1b}'),
        "remote text emitted an escape character"
    );
    Ok(())
}

async fn respond(listener: &TcpListener, reply: Value) -> Result<(String, Vec<u8>)> {
    let (stream, _) = listener.accept().await?;
    let mut stream = BufReader::new(stream);
    let mut headers = String::new();
    let mut ended = false;
    for _ in 0..32 {
        let mut line = String::new();
        ensure!(
            stream.read_line(&mut line).await? > 0,
            "incomplete HTTP request"
        );
        if line == "\r\n" {
            ended = true;
            break;
        }
        headers.push_str(&line);
    }
    ensure!(ended, "HTTP request exceeded the fixture header limit");
    let length = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim().parse::<usize>())
        .transpose()?
        .unwrap_or(0);
    ensure!(
        length <= 16384,
        "HTTP request exceeded the fixture body limit"
    );
    let mut body = vec![0; length];
    stream.read_exact(&mut body).await?;
    let reply = serde_json::to_string(&reply)?;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
        reply.len()
    );
    stream.get_mut().write_all(response.as_bytes()).await?;
    Ok((headers, body))
}

fn write_private_token(path: &Path) -> Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(b"fixture-token\n")?;
    Ok(())
}

#[tokio::test]
async fn atlas_tui_pages_reads_occurrences_and_confirms_collection() -> Result<()> {
    let article = json!({
        "run_id": "run-2",
        "id": "article-2",
        "title": "Reporting\u{1b}[31m",
        "description": "Retained description",
        "source_name": "Example source",
    });
    let expected: &[SessionRequest<'_>] = &[
        (
            "GET /api/v1/days",
            &[],
            None,
            json!({"days":["2026-11-09"],"categories":["security"]}),
        ),
        (
            "GET /api/v1/articles",
            &["category=security", "day=2026-11-09", "limit=30"],
            None,
            json!({
                "rows":[{"id":"article-1","run_id":"run-1","title":"First page"}],
                "next_cursor":r#"["2026-11-09","a &?","article-1","run-1"]"#,
            }),
        ),
        (
            "GET /api/v1/articles",
            &[
                "category=security",
                "cursor=%5B%222026-11-09%22%2C%22a+%26%3F%22%2C%22article-1%22%2C%22run-1%22%5D",
                "day=2026-11-09",
                "limit=30",
            ],
            None,
            json!({"rows":[article.clone()],"next_cursor":null}),
        ),
        (
            "GET /api/v1/articles/detail",
            &["id=article-2", "run=run-2"],
            None,
            json!({"article":article,"claims":[{"id":"claim-1","text":"Retained evidence"}]}),
        ),
        (
            "POST /api/v1/atlas/cycles",
            &[],
            Some(json!({"request_id":"confirmed"})),
            json!({"run_id":"fleet-atlas-run-confirmed","job_id":"fleet-atlas-job-confirmed"}),
        ),
    ];
    let stdout = run_session(
        env!("CARGO_BIN_EXE_fleetworks-atlas"),
        b"articles 2026-11-09 security\nnext\nopen #1\nnext\nstart declined\nno\nstart confirmed\nyes\nquit\n",
        expected,
    )
    .await?;
    for expected in [
        "Recent days  2026-11-09",
        "#1  First page",
        "#1  Reporting\\u{1b}[31m",
        "Article run-2/article-2",
        "Claims (1)",
        "Retained evidence",
        "no more articles",
        "No action sent.",
        "Job fleet-atlas-job-confirmed",
    ] {
        assert!(stdout.contains(expected), "missing {expected:?}");
    }
    assert!(!stdout.contains('\u{1b}'));
    Ok(())
}

#[tokio::test]
async fn brain_tui_renders_retained_dossier_brief_and_topology() -> Result<()> {
    let expected: &[SessionRequest<'_>] = &[
        (
            "GET /api/v1/actors/detail",
            &["id=actor-1"],
            None,
            json!({
                "id":"actor-1",
                "display_name":"Sample actor",
                "aliases":["Example alias"],
                "metadata":[{"key":"origin","value":"Example region"}],
                "briefs":[{
                    "node":{"id":"article-node","label":"Associated article"},
                    "relationship":{"label":"mentions"},
                    "article":{"run_id":"run-1","id":"article-1"},
                }],
            }),
        ),
        (
            "GET /api/v1/articles/detail",
            &["id=article-1", "run=run-1"],
            None,
            json!({
                "article":{"run_id":"run-1","id":"article-1","title":"Associated article"},
                "status":"Completed",
                "inferences":[{"id":"inference-1","text":"Retained inference"}],
            }),
        ),
        (
            "GET /api/v1/topology",
            &["seed=brief%3Arun-1%2Farticle-1"],
            None,
            json!({
                "root_label":"Retained topology",
                "nodes":[{"id":"actor-1","kind":"Actor","label":"Sample actor"}],
                "edges":[{"from":"actor-1","to":"article-node","label":"mentions"}],
                "findings":{"gaps":["Incomplete scope"]},
            }),
        ),
    ];
    let stdout = run_session(
        env!("CARGO_BIN_EXE_fleetworks-brain"),
        b"actor actor-1\nbrief run-1 article-1\ntopology brief:run-1/article-1\nquit\n",
        expected,
    )
    .await?;
    for expected in [
        "Actor actor-1",
        "Aliases  Example alias",
        "brief run-1 article-1",
        "Status  Completed",
        "Retained inference",
        "Nodes (1)",
        "actor-1 —mentions→ article-node",
        "Coverage gaps  Incomplete scope",
    ] {
        assert!(stdout.contains(expected), "missing {expected:?}");
    }
    Ok(())
}

type SessionRequest<'a> = (&'a str, &'a [&'a str], Option<Value>, Value);

async fn run_session(
    binary: &str,
    input: &[u8],
    expected: &[SessionRequest<'_>],
) -> Result<String> {
    let workspace = tempfile::tempdir()?;
    write_private_token(&workspace.path().join("fixture-token"))?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let child = async {
        let mut child = tokio::process::Command::new(binary)
            .kill_on_drop(true)
            .arg("--workspace")
            .arg(workspace.path())
            .args([
                "--api-origin",
                &origin,
                "--api-token-file",
                "fixture-token",
                "tui",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stdin = child.stdin.take().expect("child stdin must be piped");
        stdin.write_all(input).await?;
        drop(stdin);
        Ok::<_, anyhow::Error>(child.wait_with_output().await?)
    };
    let fixture = async {
        for (request, query, body, reply) in expected {
            let (headers, received_body) = respond(&listener, reply.clone()).await?;
            let line = headers.lines().next().expect("HTTP start line");
            let line = line.strip_suffix(" HTTP/1.1").expect("HTTP protocol");
            let (route, encoded_query) = line.split_once('?').unwrap_or((line, ""));
            assert_eq!(route, *request);
            let mut fields: Vec<_> = encoded_query
                .split('&')
                .filter(|part| !part.is_empty())
                .collect();
            fields.sort_unstable();
            assert_eq!(fields.as_slice(), *query);
            assert!(headers
                .lines()
                .any(|line| line.eq_ignore_ascii_case("authorization: Bearer fixture-token")));
            if let Some(body) = body {
                assert_eq!(&serde_json::from_slice::<Value>(&received_body)?, body);
            } else {
                assert!(received_body.is_empty());
            }
        }
        Ok::<_, anyhow::Error>(())
    };
    let (output, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::try_join!(child, fixture)
    })
    .await??;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

#[tokio::test]
async fn tools_tui_browses_and_confirms_manual_calls() -> Result<()> {
    let stdout = run_session(
        env!("CARGO_BIN_EXE_fleetworks-tools"),
        br#"detail #1
run #1 declined
{"objective":"Find public reports"}
no
run #1 saved-1
{"objective":"Find public reports"}
yes
result fleet-tool-saved-1
quit
"#,
        &[
            (
                "GET /api/v1/tools",
                &[],
                None,
                json!({"tools":[{
                    "id":"dork_generate","name":"Dork generator","category":"Search",
                    "inputs":["objective"],"restrictions":"Local generation",
                }]}),
            ),
            (
                "POST /api/v1/tools/run",
                &[],
                Some(json!({
                    "request_id":"saved-1","tool_id":"dork_generate",
                    "inputs":{"objective":"Find public reports"},
                })),
                json!({"call_id":"fleet-tool-saved-1"}),
            ),
            (
                "GET /api/v1/tools/result",
                &["id=fleet-tool-saved-1"],
                None,
                json!({
                    "call_id":"fleet-tool-saved-1","tool_id":"dork_generate","status":"completed",
                    "result":{"observations":{"query":"example query"},"credits_charged":0},
                }),
            ),
        ],
    )
    .await?;
    for expected in [
        "#1  Dork generator",
        "Inputs  objective",
        "Restrictions  Local generation",
        "No action sent.",
        "Accepted call fleet-tool-saved-1",
        "Call fleet-tool-saved-1  [completed]",
        "example query",
    ] {
        assert!(stdout.contains(expected), "missing {expected:?}");
    }
    Ok(())
}

#[tokio::test]
async fn models_tui_displays_routes_and_confirms_a_primary_change() -> Result<()> {
    let stdout = run_session(
        env!("CARGO_BIN_EXE_fleetworks-models"),
        b"set atlas_analyst local new-model
yes
clear atlas_analyst
no
quit
",
        &[
            (
                "GET /api/v1/models/defaults",
                &[],
                None,
                json!({"defaults":{"atlas_analyst":{
                    "model":"primary","provider":"fixture","account":"operator",
                    "fallbacks":[{"model":"backup","provider":"fixture"}],
                }}}),
            ),
            (
                "POST /api/v1/models/defaults/set",
                &[],
                Some(json!({
                    "role":"atlas_analyst","fallbacks":null,
                    "primary":{"provider":"local","model":"new-model","account":""},
                })),
                json!({"saved":true}),
            ),
        ],
    )
    .await?;
    for expected in [
        "atlas_analyst",
        "Primary  fixture / primary",
        "Account  operator",
        "Fallback 1  fixture / backup",
        "Saved model defaults.",
        "No action sent.",
    ] {
        assert!(stdout.contains(expected), "missing {expected:?}");
    }
    Ok(())
}

#[tokio::test]
async fn profile_tui_distinguishes_local_workspace_and_remote_health() -> Result<()> {
    let stdout = run_session(
        env!("CARGO_BIN_EXE_fleetworks-profile"),
        b"workspace\nhealth\nshow\nexport portable\nyes\nquit\n",
        &[
            (
                "GET /api/v1/health/ready",
                &[],
                None,
                json!({"service":"fleetworks","api_version":1,"ready":true,"storage":true}),
            ),
            (
                "GET /api/v1/profile",
                &[],
                None,
                json!({
                    "workspace":"/remote/workspace","ready":true,"storage":true,
                    "access_mode":"bearer","instance_id":"fixture-instance",
                }),
            ),
            (
                "POST /api/v1/profile/export",
                &[],
                Some(json!({"name":"portable"})),
                json!({"path":"exports/portable.json","schema_version":3}),
            ),
        ],
    )
    .await?;
    for expected in [
        "Local workspace  ",
        "API version  1",
        "Readiness  Ready",
        "Datastore  Ready",
        "Service workspace  /remote/workspace",
        "Instance  fixture-instance",
        "Saved exports/portable.json in the service workspace.",
    ] {
        assert!(stdout.contains(expected), "missing {expected:?}");
    }
    Ok(())
}
