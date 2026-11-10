//! Exercise the executable entrypoints against a local HTTP fixture.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::ensure;
use anyhow::Result;
use serde_json::json;
use serde_json::Value;
use tokio::io::AsyncBufReadExt;
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
    let fixture = async {
        let (stream, _) = listener.accept().await?;
        let mut stream = BufReader::new(stream);
        let mut request = String::new();
        for _ in 0..32 {
            let mut line = String::new();
            ensure!(
                stream.read_line(&mut line).await? > 0,
                "incomplete HTTP request"
            );
            if line == "\r\n" {
                break;
            }
            request.push_str(&line);
        }
        let body = r#"{"rows":[],"next_cursor":null}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.get_mut().write_all(response.as_bytes()).await?;
        Ok::<_, anyhow::Error>(request)
    };
    let (output, request) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(client.output(), fixture)
    })
    .await?;
    let output = output?;
    let request = request?;
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
