//! User-centric turn. Recall is injected, the open view is named, and a
//! search or mailbox read can run before the model speaks so a local model
//! without tool-calling still has the material in front of it.

use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;

use crate::brain::Memory;
use crate::gmail::{self, GmailConfig};
use crate::prompt::{self, Intent, PromptParts};
use crate::provider::{self, ChatMessage, ToolCall, ToolSpec};
use crate::report::{self, ReportMeta};
use crate::search::{self, SearchHit, SourcePlan};
use crate::secrets::ProviderSecret;

#[derive(Clone, Debug)]
pub struct HistMsg {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug)]
pub struct TurnInput {
    pub session_id: String,
    pub user_text: String,
    pub history: Vec<HistMsg>,
    pub memories: Vec<Memory>,
    pub view_name: String,
    pub view_context: String,
    pub hardware_line: String,
    pub modality: String,
    /// Writer. Talks to the user from the report list, the open report, and tool results.
    pub provider: Option<ProviderSecret>,
    /// Tool caller. Empty means the writer also calls tools.
    pub tool_provider: Option<ProviderSecret>,
    /// Stages, keys, and extra sources for this turn. Scope cards copy the
    /// global defaults and change only the stage flags.
    pub plan: SourcePlan,
    pub report_dir: PathBuf,
    pub case_id: Option<String>,
    pub gmail: Option<GmailConfig>,
    /// Markdown already on file whose titles match this question.
    pub prior_reports: String,
    /// When set, the turn may not search or leave the supplied report text.
    pub evidence_only: bool,
    /// Answer from fact memories of completed reports, and cite those reports.
    pub from_memory: bool,
}

#[derive(Clone, Debug)]
pub enum TurnEvent {
    Status(String),
    Delta(String),
    Note(String),
    Report(ReportMeta),
    Memory(Memory),
    Done(String),
    Failed(String),
}

pub async fn run_turn(input: TurnInput, tx: UnboundedSender<TurnEvent>, cancel: Arc<AtomicBool>) {
    if let Err(err) = run_turn_inner(input, &tx, &cancel).await {
        let _ = tx.send(TurnEvent::Failed(err));
    }
}

async fn run_turn_inner(
    input: TurnInput,
    tx: &UnboundedSender<TurnEvent>,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    let intent = prompt::classify(&input.user_text);
    if intent == Intent::Remember {
        let fact = prompt::remember_text(&input.user_text);
        if fact.is_empty() {
            return Err("nothing to remember".into());
        }
        let _ = tx.send(TurnEvent::Memory(Memory::fact("", fact.clone(), "")));
        let _ = tx.send(TurnEvent::Done(format!("Remembered: {fact}")));
        return Ok(());
    }

    let mut gathered = String::new();
    let mut sources: Vec<SearchHit> = Vec::new();
    let answering_reports = input.evidence_only || !input.prior_reports.trim().is_empty();
    if answering_reports {
        gathered.push_str(&input.prior_reports);
        gathered.push('\n');
    }

    if intent == Intent::Investigate && !answering_reports {
        let _ = tx.send(TurnEvent::Status("researching public sources".into()));
        match search::research(&input.user_text, &input.plan).await {
            Ok(hits) => {
                sources = hits;
                gathered.push_str(&format_hits(&sources));
                let _ = tx.send(TurnEvent::Note(format!("{} public hits", sources.len())));
            }
            Err(err) => {
                let _ = tx.send(TurnEvent::Note(format!("search: {err}")));
            }
        }
    }

    if intent == Intent::Gmail && !answering_reports {
        if let Some(cfg) = &input.gmail {
            let _ = tx.send(TurnEvent::Status("reading gmail headers".into()));
            match gmail::list_recent(cfg, 8) {
                Ok(mail) => {
                    if mail.is_empty() {
                        gathered.push_str("INBOX has no messages.\n");
                    } else {
                        gathered.push_str("Recent INBOX headers:\n");
                        for m in mail {
                            gathered
                                .push_str(&format!("- {} | {} | {}\n", m.date, m.from, m.subject));
                        }
                    }
                }
                Err(err) => gathered.push_str(&format!("Gmail: {err}\n")),
            }
        } else {
            gathered.push_str("Gmail is not connected. The user can set it up in the Gmail app.\n");
        }
    }

    if intent == Intent::Hardware && !answering_reports {
        gathered.push_str(&input.hardware_line);
        gathered.push('\n');
    }

    let provider = match &input.provider {
        Some(p) if !p.base_url.trim().is_empty() && !p.model.trim().is_empty() => p.clone(),
        _ => {
            let answer = if answering_reports {
                format!(
                    "No text provider is signed in. This is drawn from the matching report already on file.\n\n{}",
                    truncate_chars(gathered.trim(), 1600)
                )
            } else {
                offline_answer(intent, &input, &sources, &gathered)
            };
            if intent == Intent::Investigate && !sources.is_empty() {
                let md = report::source_pack(
                    &case_title(&input),
                    input.case_id.as_deref(),
                    &input.user_text,
                    &sources,
                );
                match report::write_report(
                    &input.report_dir,
                    &case_title(&input),
                    input.case_id.as_deref(),
                    &md,
                ) {
                    Ok(meta) => {
                        let _ = tx.send(TurnEvent::Report(meta.clone()));
                        let _ = tx.send(TurnEvent::Done(format!(
                            "{answer}\n\nReport: {}",
                            meta.path
                        )));
                        return Ok(());
                    }
                    Err(err) => return Err(err.to_string()),
                }
            }
            let _ = tx.send(TurnEvent::Done(answer));
            return Ok(());
        }
    };

    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }

    let system = prompt::system_prompt(&PromptParts {
        view_name: &input.view_name,
        view_context: &input.view_context,
        hardware_line: &input.hardware_line,
        memories: &[],
        modality: &input.modality,
        evidence_only: input.evidence_only || answering_reports,
        from_memory: input.from_memory,
        writer_only: false,
    });

    let mut messages = vec![ChatMessage {
        role: "system".into(),
        content: system,
        tool_call_id: None,
        tool_calls: Vec::new(),
    }];
    for hist in input
        .history
        .iter()
        .rev()
        .take(16)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        messages.push(ChatMessage {
            role: hist.role.clone(),
            content: hist.content.clone(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        });
    }
    let mut user_content = input.user_text.clone();
    if !gathered.trim().is_empty() {
        user_content.push_str("\n\nMATERIAL ALREADY GATHERED:\n");
        user_content.push_str(gathered.trim());
    }
    messages.push(ChatMessage {
        role: "user".into(),
        content: user_content,
        tool_call_id: None,
        tool_calls: Vec::new(),
    });

    let tools = if answering_reports {
        Vec::new()
    } else {
        tool_specs()
    };
    let tool_provider = input
        .tool_provider
        .clone()
        .unwrap_or_else(|| provider.clone());
    // Different models: the tool caller gathers, then the writer speaks.
    // The same model keeps one loop so the reply streams without a second call.
    let split = !answering_reports && !tools.is_empty() && !same_model(&provider, &tool_provider);
    let mut final_text = String::new();
    if split {
        let _ = tx.send(TurnEvent::Status("calling tools".into()));
        let mut tool_messages = messages.clone();
        if let Some(system) = tool_messages.first_mut() {
            system.content = tool_caller_prompt(&input.view_name, &input.view_context);
        }
        let digest = match run_tool_rounds(
            &tool_provider,
            &mut tool_messages,
            &tools,
            &input,
            tx,
            cancel,
        )
        .await
        {
            Ok(outcome) => {
                sources.extend(outcome.hits);
                outcome.digest
            }
            Err(err) if err == "cancelled" => return Err(err),
            Err(err) => {
                let _ = tx.send(TurnEvent::Note(format!("tool caller failed: {err}")));
                "TOOL RESULTS:\nThe tool step failed. The detail is in the System log.\n".into()
            }
        };
        if let Some(system) = messages.first_mut() {
            system.content = prompt::system_prompt(&PromptParts {
                view_name: &input.view_name,
                view_context: &input.view_context,
                hardware_line: &input.hardware_line,
                memories: &[],
                modality: &input.modality,
                evidence_only: false,
                from_memory: false,
                writer_only: true,
            });
        }
        if !digest.trim().is_empty() {
            if let Some(user) = messages.last_mut() {
                user.content.push_str("\n\n");
                user.content.push_str(digest.trim());
            }
        }
        let _ = tx.send(TurnEvent::Status("writing".into()));
        let tx_delta = tx.clone();
        let completion = provider::complete(&provider, &messages, &[], move |delta| {
            let _ = tx_delta.send(TurnEvent::Delta(delta.to_string()));
        })
        .await
        .map_err(|e| e.to_string())?;
        final_text = completion.content;
    } else {
        for round in 0..3 {
            if cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            let _ = tx.send(TurnEvent::Status(if round == 0 {
                "thinking".into()
            } else {
                format!("tool round {}", round + 1)
            }));
            let tx_delta = tx.clone();
            let completion = provider::complete(&provider, &messages, &tools, move |delta| {
                let _ = tx_delta.send(TurnEvent::Delta(delta.to_string()));
            })
            .await
            .map_err(|e| e.to_string())?;
            if completion.tool_calls.is_empty() {
                final_text = completion.content;
                break;
            }
            messages.push(ChatMessage {
                role: "assistant".into(),
                content: completion.content.clone(),
                tool_call_id: None,
                tool_calls: completion.tool_calls.clone(),
            });
            for call in completion.tool_calls {
                if cancel.load(Ordering::Relaxed) {
                    return Err("cancelled".into());
                }
                let (note, body, extra_hits) = exec_tool(&input, &call, tx).await;
                let _ = tx.send(TurnEvent::Note(note));
                sources.extend(extra_hits);
                messages.push(ChatMessage {
                    role: "tool".into(),
                    content: body,
                    tool_call_id: Some(call.id),
                    tool_calls: Vec::new(),
                });
            }
        }
    }

    if final_text.trim().is_empty() {
        final_text = "The model returned no text.".into();
    }
    if intent == Intent::Investigate
        && !answering_reports
        && (!sources.is_empty() || final_text.len() > 400)
    {
        let md = report::render_report(
            &case_title(&input),
            input.case_id.as_deref(),
            &input.user_text,
            &final_text,
            &sources,
        );
        if let Ok(meta) = report::write_report(
            &input.report_dir,
            &case_title(&input),
            input.case_id.as_deref(),
            &md,
        ) {
            let _ = tx.send(TurnEvent::Report(meta.clone()));
            final_text.push_str(&format!("\n\nReport: {}", meta.path));
        }
    }
    let _ = tx.send(TurnEvent::Done(final_text));
    Ok(())
}

fn same_model(writer: &ProviderSecret, tool: &ProviderSecret) -> bool {
    writer.model.trim() == tool.model.trim()
        && writer.base_url.trim().trim_end_matches('/')
            == tool.base_url.trim().trim_end_matches('/')
}

struct ToolOutcome {
    hits: Vec<SearchHit>,
    digest: String,
}

/// The tool caller only sees the question, the view, and prior chat.
/// It does not write the reply the user reads.
fn tool_caller_prompt(view_name: &str, view_context: &str) -> String {
    format!(
        r#"You call tools for Argos. A writer model will speak to the user. You do not.
Call the research tools that fit the question. The view below says whether a report chat is open or the report list is on the desk.
If the user asks you to remember a fact about themselves, call remember.
Do not call write_report. The desk files the report from the writer's answer.
When you have the results, stop. If no tool is useful, stop without calling one.

ACTIVE VIEW: {view_name}
{view_context}
"#,
        view_context = view_context.trim()
    )
}

/// Plain text for the writer. Tool-call messages stay with the tool caller,
/// because the writer is not offered tools.
fn format_tool_digest(notes: &[String], results: &[(String, String)]) -> String {
    let mut out = String::new();
    let notes: Vec<&str> = notes
        .iter()
        .map(|note| note.trim())
        .filter(|note| !note.is_empty())
        .collect();
    if !notes.is_empty() {
        out.push_str("Tool caller note:\n");
        out.push_str(&notes.join("\n"));
        out.push_str("\n\n");
    }
    if results.is_empty() {
        return out;
    }
    out.push_str("TOOL RESULTS:\n");
    for (name, body) in results {
        out.push_str(&format!("\n## {name}\n"));
        out.push_str(truncate_chars(body.trim(), 2500).trim());
        out.push('\n');
    }
    out
}

async fn run_tool_rounds(
    provider: &ProviderSecret,
    messages: &mut Vec<ChatMessage>,
    tools: &[ToolSpec],
    input: &TurnInput,
    tx: &UnboundedSender<TurnEvent>,
    cancel: &Arc<AtomicBool>,
) -> Result<ToolOutcome, String> {
    let mut hits = Vec::new();
    let mut notes = Vec::new();
    let mut results = Vec::new();
    for round in 0..3 {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let _ = tx.send(TurnEvent::Status(format!("tool round {}", round + 1)));
        let completion = provider::complete(provider, messages, tools, |_| {})
            .await
            .map_err(|err| err.to_string())?;
        if !completion.content.trim().is_empty() {
            notes.push(truncate_chars(completion.content.trim(), 600));
        }
        if completion.tool_calls.is_empty() {
            break;
        }
        messages.push(ChatMessage {
            role: "assistant".into(),
            content: completion.content.clone(),
            tool_call_id: None,
            tool_calls: completion.tool_calls.clone(),
        });
        for call in completion.tool_calls {
            if cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            let (note, body, extra_hits) = exec_tool(input, &call, tx).await;
            let _ = tx.send(TurnEvent::Note(note.clone()));
            hits.extend(extra_hits);
            results.push((note, body.clone()));
            messages.push(ChatMessage {
                role: "tool".into(),
                content: body,
                tool_call_id: Some(call.id),
                tool_calls: Vec::new(),
            });
        }
    }
    Ok(ToolOutcome {
        hits,
        digest: format_tool_digest(&notes, &results),
    })
}

fn truncate_chars(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}…")
}

fn case_title(input: &TurnInput) -> String {
    let t = input.user_text.trim().chars().take(72).collect::<String>();
    if t.is_empty() {
        "Argos report".into()
    } else {
        t
    }
}

fn offline_answer(
    intent: Intent,
    input: &TurnInput,
    sources: &[SearchHit],
    gathered: &str,
) -> String {
    match intent {
        Intent::Hardware => input.hardware_line.clone(),
        Intent::Investigate if !sources.is_empty() => {
            "No text provider is signed in, so this is a source pack rather than a written analysis. Open Providers or run `argos login` to draft the narrative.".into()
        }
        Intent::Gmail => {
            if gathered.trim().is_empty() {
                "Gmail is not connected.".into()
            } else {
                gathered.trim().to_string()
            }
        }
        _ => "No text provider is signed in. Open Providers, or run `argos login`. /search, /hardware, /brain, and /gmail still work from the prompt.".into(),
    }
}

fn format_hits(hits: &[SearchHit]) -> String {
    let mut out = String::new();
    for (i, hit) in hits.iter().enumerate() {
        out.push_str(&format!(
            "{}. {} — {}\n   {}\n",
            i + 1,
            hit.title,
            hit.url,
            hit.snippet
        ));
    }
    out
}

fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "web_search".into(),
            description: "Search the public web (SearXNG or DuckDuckGo, plus Brave or Tavily when a key is set). Returns titles, urls, and snippets.".into(),
            parameters: json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
        },
        ToolSpec {
            name: "news_search".into(),
            description: "Search public news (SearXNG news and GDELT). Returns titles, urls, and snippets.".into(),
            parameters: json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
        },
        ToolSpec {
            name: "domain_lookup".into(),
            description: "Look up a domain: RDAP, certificates, DNS, Wayback, and InternetDB. Skips RDAP and certificates when the text has no domain.".into(),
            parameters: json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
        },
        ToolSpec {
            name: "social_search".into(),
            description: "Search public posts on Bluesky, Hacker News, Mastodon, and YouTube when a key is set.".into(),
            parameters: json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
        },
        ToolSpec {
            name: "identity_lookup".into(),
            description: "Look up a public GitHub user or repository for a handle or email.".into(),
            parameters: json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
        },
        ToolSpec {
            name: "fetch_page".into(),
            description:
                "Fetch a public http(s) page and return plain text. Private addresses are refused."
                    .into(),
            parameters: json!({"type":"object","properties":{"url":{"type":"string"}},"required":["url"]}),
        },
        ToolSpec {
            name: "remember".into(),
            description: "Store a durable fact about the user for later recall.".into(),
            parameters: json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}),
        },
        ToolSpec {
            name: "write_report".into(),
            description: "Write a markdown report to disk.".into(),
            parameters: json!({"type":"object","properties":{"title":{"type":"string"},"markdown":{"type":"string"}},"required":["title","markdown"]}),
        },
    ]
}

fn arg_query(args: &Value) -> String {
    args.get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn pack_search(
    name: &str,
    query: &str,
    result: Result<Vec<SearchHit>, String>,
) -> (String, String, Vec<SearchHit>) {
    match result {
        Ok(hits) => {
            let body = format_hits(&hits);
            (format!("{name} {query} ({} hits)", hits.len()), body, hits)
        }
        Err(err) => (format!("{name} failed: {err}"), err, Vec::new()),
    }
}

async fn exec_tool(
    input: &TurnInput,
    call: &ToolCall,
    tx: &UnboundedSender<TurnEvent>,
) -> (String, String, Vec<SearchHit>) {
    let args: Value = serde_json::from_str(&call.arguments).unwrap_or(json!({}));
    match call.name.as_str() {
        "web_search" => {
            let query = arg_query(&args);
            pack_search(
                "web_search",
                &query,
                search::web_search(&query, &input.plan).await,
            )
        }
        "news_search" => {
            let query = arg_query(&args);
            pack_search(
                "news_search",
                &query,
                search::news_search(&query, &input.plan).await,
            )
        }
        "domain_lookup" => {
            let query = arg_query(&args);
            if search::TextQuery::extract(&query).domains.is_empty() {
                return (
                    "domain_lookup skipped: no domain".into(),
                    "No domain name in the query, so RDAP, crt.sh, DNS, Wayback, and InternetDB were not called.".into(),
                    Vec::new(),
                );
            }
            pack_search(
                "domain_lookup",
                &query,
                search::domain_lookup(&query, &input.plan).await,
            )
        }
        "social_search" => {
            let query = arg_query(&args);
            pack_search(
                "social_search",
                &query,
                search::social_search(&query, &input.plan).await,
            )
        }
        "identity_lookup" => {
            let query = arg_query(&args);
            pack_search(
                "identity_lookup",
                &query,
                search::identity_lookup(&query, &input.plan).await,
            )
        }
        "fetch_page" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
            match search::fetch_page(url).await {
                Ok(text) => (format!("fetch_page {url}"), text, Vec::new()),
                Err(err) => (format!("fetch_page refused: {err}"), err, Vec::new()),
            }
        }
        "remember" => {
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if text.is_empty() {
                return ("remember skipped".into(), "empty".into(), Vec::new());
            }
            (
                {
                    let _ = tx.send(TurnEvent::Memory(Memory::fact("", text.clone(), "")));
                    format!("remembered {text}")
                },
                format!("stored: {text}"),
                Vec::new(),
            )
        }
        "write_report" => {
            let title = args
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Report");
            let markdown = args.get("markdown").and_then(|v| v.as_str()).unwrap_or("");
            let md = report::render_report(title, input.case_id.as_deref(), "", markdown, &[]);
            match report::write_report(&input.report_dir, title, input.case_id.as_deref(), &md) {
                Ok(meta) => {
                    let _ = tx.send(TurnEvent::Report(meta.clone()));
                    (format!("report {}", meta.path), meta.path, Vec::new())
                }
                Err(err) => (format!("report failed: {err}"), err.to_string(), Vec::new()),
            }
        }
        other => (
            format!("unknown tool {other}"),
            format!("unknown tool {other}"),
            Vec::new(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{format_tool_digest, same_model};
    use crate::secrets::ProviderSecret;

    #[test]
    fn tool_digest_is_plain_text_for_the_writer() {
        let digest = format_tool_digest(
            &["Looked at the harbor.".into()],
            &[(
                "web_search harbor (2 hits)".into(),
                "1. Port\nhttps://example.com".into(),
            )],
        );
        assert!(digest.contains("TOOL RESULTS:"));
        assert!(digest.contains("web_search harbor"));
        assert!(digest.contains("https://example.com"));
        assert!(digest.contains("Tool caller note:"));
        assert!(!digest.contains("tool_call_id"));
        assert!(format_tool_digest(&[], &[]).is_empty());
    }

    #[test]
    fn same_model_stays_one_loop() {
        let writer = ProviderSecret {
            kind: "openrouter".into(),
            base_url: "https://openrouter.ai/api/v1/".into(),
            model: "writer".into(),
            api_key: None,
            stt_model: None,
            device: None,
        };
        let mut tool = writer.clone();
        tool.base_url = "https://openrouter.ai/api/v1".into();
        assert!(same_model(&writer, &tool));
        tool.model = "tool".into();
        assert!(!same_model(&writer, &tool));
    }
}
