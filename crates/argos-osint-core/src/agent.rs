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

use crate::brain::{self, Memory};
use crate::gmail::{self, GmailConfig};
use crate::prompt::{self, Intent, PromptParts};
use crate::provider::{self, ChatMessage, ToolCall, ToolSpec};
use crate::report::{self, ReportMeta};
use crate::search::{self, SearchHit};
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
    pub provider: Option<ProviderSecret>,
    pub searx_url: Option<String>,
    pub report_dir: PathBuf,
    pub case_id: Option<String>,
    pub gmail: Option<GmailConfig>,
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
        let _ = tx.send(TurnEvent::Memory(Memory {
            id: String::new(),
            text: fact.clone(),
            created_at: String::new(),
        }));
        let _ = tx.send(TurnEvent::Done(format!("Remembered: {fact}")));
        return Ok(());
    }

    let hits_mem = brain::recall(&input.memories, &input.user_text, 6);
    if !hits_mem.is_empty() {
        let line = hits_mem
            .iter()
            .map(|h| h.memory.text.as_str())
            .collect::<Vec<_>>()
            .join(" · ");
        let _ = tx.send(TurnEvent::Note(format!(
            "recalled {n}: {line}",
            n = hits_mem.len()
        )));
    }

    let mut gathered = String::new();
    let mut sources: Vec<SearchHit> = Vec::new();

    if intent == Intent::Investigate {
        let _ = tx.send(TurnEvent::Status("searching public sources".into()));
        match search::web_search(&input.user_text, input.searx_url.as_deref()).await {
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

    if intent == Intent::Gmail {
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

    if intent == Intent::Hardware {
        gathered.push_str(&input.hardware_line);
        gathered.push('\n');
    }

    let provider = match &input.provider {
        Some(p) if !p.base_url.trim().is_empty() && !p.model.trim().is_empty() => p.clone(),
        _ => {
            let answer = offline_answer(intent, &input, &sources, &gathered);
            if intent == Intent::Investigate && !sources.is_empty() {
                let md = report::source_pack(&case_title(&input), &input.user_text, &sources);
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
        memories: &hits_mem,
        modality: &input.modality,
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

    let tools = tool_specs();
    let mut final_text = String::new();
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

    if final_text.trim().is_empty() {
        final_text = "The model returned no text.".into();
    }
    if intent == Intent::Investigate && (!sources.is_empty() || final_text.len() > 400) {
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
            description: "Search the public web. Returns titles, urls, and snippets.".into(),
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

async fn exec_tool(
    input: &TurnInput,
    call: &ToolCall,
    tx: &UnboundedSender<TurnEvent>,
) -> (String, String, Vec<SearchHit>) {
    let args: Value = serde_json::from_str(&call.arguments).unwrap_or(json!({}));
    match call.name.as_str() {
        "web_search" => {
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            match search::web_search(query, input.searx_url.as_deref()).await {
                Ok(hits) => {
                    let body = format_hits(&hits);
                    (
                        format!("web_search {query} ({} hits)", hits.len()),
                        body,
                        hits,
                    )
                }
                Err(err) => (format!("web_search failed: {err}"), err, Vec::new()),
            }
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
                    let _ = tx.send(TurnEvent::Memory(Memory {
                        id: String::new(),
                        text: text.clone(),
                        created_at: String::new(),
                    }));
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
