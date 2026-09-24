//! Newline-delimited JSON-RPC for the Gmail MCP server (MCP stdio, 2025-06-18).
//! Content-Length framing is also accepted so older clients still connect.
//! Logging belongs on stderr. This module only returns stdout messages.

use serde_json::{json, Value};

use crate::gmail::{self, GmailConfig};

pub const PROTOCOL: &str = "2025-06-18";

/// Handle one JSON-RPC message. `None` means the client sent a notification
/// and no response should be written.
pub fn handle_message(text: &str, cfg: Option<&GmailConfig>) -> Option<String> {
    let msg: Value = serde_json::from_str(text).ok()?;
    let id = msg.get("id").cloned();
    if id.is_none() {
        return None;
    }
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "argos-gmail", "version": env!("CARGO_PKG_VERSION") }
        })),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => call_tool(msg.get("params"), cfg),
        "ping" => Ok(json!({})),
        other => Err(format!("unknown method {other}")),
    };
    let body = match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(message) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32000, "message": message }
        }),
    };
    Some(body.to_string())
}

fn tools() -> Value {
    json!([
        {
            "name": "gmail_list_recent",
            "description": "List recent INBOX headers from the signed-in Gmail account. Read only.",
            "inputSchema": {
                "type": "object",
                "properties": { "limit": { "type": "integer", "minimum": 1, "maximum": 20 } }
            }
        },
        {
            "name": "gmail_search",
            "description": "Search the signed-in Gmail INBOX. Read only. The query cannot contain quotes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20 }
                },
                "required": ["query"]
            }
        }
    ])
}

fn call_tool(params: Option<&Value>, cfg: Option<&GmailConfig>) -> Result<Value, String> {
    let params = params.ok_or("missing params")?;
    let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let limit = args
        .get("limit")
        .and_then(|n| n.as_u64())
        .unwrap_or(10)
        .clamp(1, 20) as usize;
    let cfg = cfg
        .ok_or("Gmail is not configured. Open the Gmail app in Argos and save an app password.")?;
    let hits = match name {
        "gmail_list_recent" => gmail::list_recent(cfg, limit)?,
        "gmail_search" => {
            let query = args.get("query").and_then(|q| q.as_str()).unwrap_or("");
            gmail::search_mail(cfg, query, limit)?
        }
        other => return Err(format!("unknown tool {other}")),
    };
    let text = if hits.is_empty() {
        "No matching messages.".to_string()
    } else {
        hits.iter()
            .map(|h| format!("uid {} | {} | {} | {}", h.uid, h.date, h.from, h.subject))
            .collect::<Vec<_>>()
            .join("\n")
    };
    Ok(json!({
        "content": [{ "type": "text", "text": text }],
        "isError": false
    }))
}

/// Pull one MCP message from a buffer. Supports a bare JSON line and the
/// older `Content-Length` header frame. Returns the JSON body and the number
/// of bytes consumed, or `None` if the buffer is incomplete.
pub fn take_frame(buf: &str) -> Option<(String, usize)> {
    let trimmed = buf.trim_start_matches(['\r', '\n']);
    let skipped = buf.len() - trimmed.len();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('{') {
        let end = trimmed.find('\n')?;
        let line = trimmed[..end].trim().to_string();
        if line.is_empty() {
            return None;
        }
        return Some((line, skipped + end + 1));
    }
    let header_end = trimmed.find("\r\n\r\n").or_else(|| trimmed.find("\n\n"))?;
    let sep_len = if trimmed[header_end..].starts_with("\r\n\r\n") {
        4
    } else {
        2
    };
    let headers = &trimmed[..header_end];
    let mut len = None;
    for line in headers.split(['\n', '\r']) {
        if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            len = rest.trim().parse::<usize>().ok();
        }
    }
    let len = len?;
    let start = header_end + sep_len;
    if trimmed.len() < start + len {
        return None;
    }
    let body = trimmed[start..start + len].to_string();
    Some((body, skipped + start + len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_and_unknown_notification() {
        let init = handle_message(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            None,
        )
        .unwrap();
        assert!(init.contains("argos-gmail"));
        assert!(init.contains(PROTOCOL));
        assert!(handle_message(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            None
        )
        .is_none());
    }

    #[test]
    fn tools_call_without_config_is_an_error() {
        let raw = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"gmail_list_recent","arguments":{}}}"#;
        let out = handle_message(raw, None).unwrap();
        assert!(out.contains("not configured"));
    }

    #[test]
    fn frames_newline_and_content_length() {
        let (body, n) = take_frame("{\"jsonrpc\":\"2.0\",\"id\":1}\n").unwrap();
        assert!(body.contains("jsonrpc"));
        assert_eq!(n, "{\"jsonrpc\":\"2.0\",\"id\":1}\n".len());
        let framed = "Content-Length: 7\r\n\r\n{\"a\":1}TAIL";
        let (body, n) = take_frame(framed).unwrap();
        assert_eq!(body, "{\"a\":1}");
        assert_eq!(&framed[n..], "TAIL");
    }
}
