//! Optional ChatGPT Writer via the user's authenticated Codex CLI.
//! Argos never reads, copies, or refreshes Codex credentials. API billing is
//! deliberately not a fallback for this mode.

use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::provider::{ChatMessage, Completion, ToolSpec};
use crate::secrets::ProviderSecret;

fn command() -> Command {
    let mut cmd = Command::new("codex");
    // An environment API key must not silently switch the billing method.
    cmd.env_remove("OPENAI_API_KEY")
        .env_remove("CODEX_API_KEY")
        .env_remove("CODEX_ACCESS_TOKEN")
        .kill_on_drop(true);
    cmd
}

pub fn is_subscription_status(status: &str) -> bool {
    status
        .to_ascii_lowercase()
        .contains("logged in using chatgpt")
}

pub async fn check_login() -> Result<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        command().args(["login", "status"]).output(),
    )
    .await
    .context("Codex sign-in check timed out")?
    .context("Install Codex CLI, then run `codex login` for ChatGPT access")?;
    let status = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() && is_subscription_status(&status) {
        Ok("ChatGPT signed in · Codex manages subscription access".into())
    } else {
        Err(anyhow!("ChatGPT sign-in required. Run `codex login` in another terminal, then check again. An API-key Codex login cannot use subscription mode."))
    }
}

/// Device sign-in keeps terminal ownership with Argos. Only the verification
/// URL/code is displayed; Codex stores and refreshes its own credentials.
pub async fn login(mut on_progress: impl FnMut(&str)) -> Result<String> {
    let mut child = command()
        .args(["login", "--device-auth"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Install Codex CLI to sign in with ChatGPT")?;
    let mut stdout = BufReader::new(child.stdout.take().context("Codex stdout")?).lines();
    let mut stderr = BufReader::new(child.stderr.take().context("Codex stderr")?).lines();
    tokio::time::timeout(Duration::from_secs(300), async {
        let (mut out_done, mut err_done) = (false, false);
        while !out_done || !err_done {
            let (is_out, line) = tokio::select! {
                line = stdout.next_line(), if !out_done => (true, line?),
                line = stderr.next_line(), if !err_done => (false, line?),
            };
            match line {
                Some(line) if !line.trim().is_empty() => {
                    let clean = regex::Regex::new(r"\x1b\[[0-9;]*m")?.replace_all(&line, "");
                    on_progress(&clean.chars().take(300).collect::<String>());
                }
                None if is_out => out_done = true,
                None => err_done = true,
                _ => {}
            }
        }
        if !child.wait().await?.success() {
            return Err(anyhow!("ChatGPT sign-in did not complete. Try again, or run `codex login` in another terminal and Check existing login."));
        }
        check_login().await
    }).await.context("ChatGPT sign-in timed out; select Sign in to try again")?
}

fn answer_event(event: &Value) -> Option<&str> {
    if event.get("type")?.as_str()? != "item.completed" {
        return None;
    }
    let item = event.get("item")?;
    (item.get("type")?.as_str()? == "agent_message")
        .then(|| item.get("text").and_then(Value::as_str))
        .flatten()
}

fn exec_args(model: &str) -> Vec<String> {
    let mut args: Vec<String> = [
        "exec",
        "--ephemeral",
        "--json",
        "--color",
        "never",
        "--skip-git-repo-check",
        "--ignore-user-config",
        "--ignore-rules",
        "--sandbox",
        "read-only",
        "-c",
        "model_provider=\"openai\"",
        "-c",
        "web_search=\"disabled\"",
        "-c",
        "approval_policy=\"never\"",
        "--disable",
        "shell_tool",
        "--disable",
        "apps",
        "--disable",
        "plugins",
        "--disable",
        "multi_agent",
        "--disable",
        "browser_use",
        "--disable",
        "computer_use",
        "--disable",
        "image_generation",
        "--disable",
        "view_image",
        "--disable",
        "memories",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    if !model.trim().is_empty() && model != "codex-default" {
        args.extend(["--model".into(), model.trim().into()]);
    }
    args.push("-".into());
    args
}

pub async fn complete(
    secret: &ProviderSecret,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
    mut on_delta: impl FnMut(&str),
) -> Result<Completion> {
    if !tools.is_empty() {
        return Err(anyhow!(
            "ChatGPT subscription is a Writer connection and does not accept tool calls."
        ));
    }
    check_login().await?;
    // No repository, user instructions, or session history
    // are discovered by the subprocess. Its only evidence is this request.
    let work = tempfile::tempdir().context("create isolated Codex workspace")?;
    let mut child = command()
        .args(exec_args(&secret.model))
        .current_dir(work.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("start Codex Writer; update Codex CLI if an option is unsupported")?;
    let input = serde_json::to_string(
        &messages
            .iter()
            .map(|m| serde_json::json!({"role": m.role, "content": m.content}))
            .collect::<Vec<_>>(),
    )?;
    let mut stdin = child.stdin.take().context("Codex stdin")?;
    stdin.write_all(format!("Answer the enclosed Argos conversation as its assistant. Follow its system instructions. Do not inspect files, use tools, or conduct additional research. Return only the answer.\n{input}\n").as_bytes()).await?;
    stdin.shutdown().await?;
    drop(stdin);
    let stdout = child.stdout.take().context("Codex stdout")?;
    let mut lines = BufReader::new(stdout.take(2 * 1024 * 1024 + 1)).lines();
    let result = tokio::time::timeout(Duration::from_secs(180), async {
        let mut content = String::new();
        let mut received = 0usize;
        while let Some(line) = lines.next_line().await? {
            received += line.len();
            if received > 2 * 1024 * 1024 {
                return Err(anyhow!("Codex Writer output exceeded its limit"));
            }
            let Ok(event) = serde_json::from_str::<Value>(&line) else { continue; };
            if matches!(event.get("type").and_then(Value::as_str), Some("turn.failed" | "error")) {
                return Err(anyhow!("Codex Writer failed; check subscription availability and the selected model with Codex CLI"));
            }
            if let Some(text) = answer_event(&event) {
                // Codex JSONL currently emits complete message chunks.
                if !content.is_empty() {
                    content.push_str("\n\n");
                    on_delta("\n\n");
                }
                content.push_str(text);
                on_delta(text);
            }
        }
        if !child.wait().await?.success() || content.trim().is_empty() {
            return Err(anyhow!("Codex Writer returned no successful answer; update Codex CLI or check ChatGPT sign-in"));
        }
        Ok(Completion { content, tool_calls: Vec::new() })
    }).await.context("Codex Writer timed out")?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_login_cannot_be_mistaken_for_subscription() {
        assert!(is_subscription_status("Logged in using ChatGPT"));
        assert!(!is_subscription_status("Logged in using an API key"));
        assert!(!is_subscription_status("Not logged in"));
    }

    #[test]
    fn writer_is_ephemeral_and_does_not_inherit_tools_or_configuration() {
        let args = exec_args("codex-default");
        for flag in [
            "--ephemeral",
            "--ignore-user-config",
            "--ignore-rules",
            "read-only",
            "shell_tool",
            "apps",
            "plugins",
            "multi_agent",
        ] {
            assert!(args.iter().any(|a| a == flag));
        }
        assert!(!args.iter().any(|a| a == "--model"));
        assert!(exec_args("custom-model")
            .windows(2)
            .any(|pair| pair == ["--model", "custom-model"]));
    }

    #[test]
    fn only_completed_assistant_messages_are_answer_chunks() {
        assert_eq!(
            answer_event(
                &serde_json::json!({"type":"item.completed", "item":{"type":"agent_message", "text":"Evidence answer"}})
            ),
            Some("Evidence answer")
        );
        assert_eq!(
            answer_event(
                &serde_json::json!({"type":"item.completed", "item":{"type":"command_execution", "text":"private output"}})
            ),
            None
        );
        assert_eq!(
            answer_event(&serde_json::json!({"type":"turn.failed"})),
            None
        );
    }
}
