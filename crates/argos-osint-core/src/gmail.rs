//! Gmail-only mailbox path. IMAP is pinned to `imap.gmail.com:993`.
//! SMTP send is not implemented. Credentials are an app password the user
//! types locally; they are never written into the prompt.

use serde::{Deserialize, Serialize};

use crate::secrets::GmailSecret;

pub const GMAIL_IMAP_HOST: &str = "imap.gmail.com";
pub const GMAIL_IMAP_PORT: u16 = 993;

#[derive(Clone, Debug)]
pub struct GmailConfig {
    pub email: String,
    pub app_password: String,
}

impl From<&GmailSecret> for GmailConfig {
    fn from(value: &GmailSecret) -> Self {
        Self {
            email: value.email.trim().to_string(),
            app_password: value
                .app_password
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MailHit {
    pub uid: String,
    pub from: String,
    pub subject: String,
    pub date: String,
}

pub fn validate(cfg: &GmailConfig) -> Result<(), String> {
    if !cfg.email.contains('@') || cfg.email.contains(char::is_whitespace) {
        return Err("Gmail address looks wrong".into());
    }
    if cfg.app_password.len() < 8 {
        return Err(
            "App password is too short. Create one in the Google account security page.".into(),
        );
    }
    assert_gmail_host(GMAIL_IMAP_HOST)
}

pub fn assert_gmail_host(host: &str) -> Result<(), String> {
    if host.eq_ignore_ascii_case(GMAIL_IMAP_HOST) {
        Ok(())
    } else {
        Err(format!(
            "Gmail path only allows {GMAIL_IMAP_HOST}, not {host}"
        ))
    }
}

pub fn list_recent(cfg: &GmailConfig, limit: usize) -> Result<Vec<MailHit>, String> {
    validate(cfg)?;
    let mut session = login(cfg)?;
    let hits = fetch_headers(&mut session, "ALL", limit)?;
    let _ = session.logout();
    Ok(hits)
}

pub fn search_mail(cfg: &GmailConfig, query: &str, limit: usize) -> Result<Vec<MailHit>, String> {
    validate(cfg)?;
    let query = query.trim();
    if query.is_empty()
        || query
            .chars()
            .any(|c| c == '"' || c == '\n' || c == '\r' || c == '\\')
    {
        return Err(
            "search text must be non-empty and cannot contain quotes or line breaks".into(),
        );
    }
    let mut session = login(cfg)?;
    session.select("INBOX").map_err(|e| e.to_string())?;
    let criteria = format!("TEXT {query}");
    let uids = session.uid_search(&criteria).map_err(|e| e.to_string())?;
    let hits = fetch_uid_set(&mut session, &uids, limit)?;
    let _ = session.logout();
    Ok(hits)
}

pub fn inbox_count(cfg: &GmailConfig) -> Result<u32, String> {
    validate(cfg)?;
    let mut session = login(cfg)?;
    let mailbox = session.select("INBOX").map_err(|e| e.to_string())?;
    let exists = mailbox.exists;
    let _ = session.logout();
    Ok(exists)
}

fn login(
    cfg: &GmailConfig,
) -> Result<imap::Session<native_tls::TlsStream<std::net::TcpStream>>, String> {
    assert_gmail_host(GMAIL_IMAP_HOST)?;
    let tls = native_tls::TlsConnector::builder()
        .build()
        .map_err(|e| e.to_string())?;
    let client = imap::connect((GMAIL_IMAP_HOST, GMAIL_IMAP_PORT), GMAIL_IMAP_HOST, &tls)
        .map_err(|e| format!("imap connect: {e}"))?;
    client
        .login(&cfg.email, &cfg.app_password)
        .map_err(|e| format!("gmail login failed: {}", e.0))
}

fn fetch_headers(
    session: &mut imap::Session<native_tls::TlsStream<std::net::TcpStream>>,
    _criteria: &str,
    limit: usize,
) -> Result<Vec<MailHit>, String> {
    session.select("INBOX").map_err(|e| e.to_string())?;
    let uids = session.uid_search("ALL").map_err(|e| e.to_string())?;
    fetch_uid_set(session, &uids, limit)
}

fn fetch_uid_set(
    session: &mut imap::Session<native_tls::TlsStream<std::net::TcpStream>>,
    uids: &std::collections::HashSet<u32>,
    limit: usize,
) -> Result<Vec<MailHit>, String> {
    let mut ids: Vec<u32> = uids.iter().copied().collect();
    ids.sort_unstable();
    ids.reverse();
    ids.truncate(limit.max(1));
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let set = ids
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let fetches = session
        .uid_fetch(set, "(UID ENVELOPE)")
        .map_err(|e| e.to_string())?;
    let mut hits = Vec::new();
    for fetch in fetches.iter() {
        let env = fetch.envelope();
        let subject = env
            .and_then(|e| e.subject)
            .map(|b| String::from_utf8_lossy(b).to_string())
            .unwrap_or_else(|| "(no subject)".into());
        let from = env
            .and_then(|e| e.from.as_ref())
            .and_then(|a| a.first())
            .map(|addr| format_addr(addr.name, addr.mailbox, addr.host))
            .unwrap_or_else(|| "(unknown)".into());
        let date = env
            .and_then(|e| e.date)
            .map(|b| String::from_utf8_lossy(b).to_string())
            .unwrap_or_default();
        hits.push(MailHit {
            uid: fetch.uid.unwrap_or(0).to_string(),
            from,
            subject,
            date,
        });
    }
    Ok(hits)
}

fn format_addr(name: Option<&[u8]>, mailbox: Option<&[u8]>, host: Option<&[u8]>) -> String {
    let name = name
        .map(|b| String::from_utf8_lossy(b).to_string())
        .unwrap_or_default();
    let mailbox = mailbox
        .map(|b| String::from_utf8_lossy(b).to_string())
        .unwrap_or_default();
    let host = host
        .map(|b| String::from_utf8_lossy(b).to_string())
        .unwrap_or_default();
    let email = if mailbox.is_empty() {
        String::new()
    } else {
        format!("{mailbox}@{host}")
    };
    if name.is_empty() {
        email
    } else if email.is_empty() {
        name
    } else {
        format!("{name} <{email}>")
    }
}

pub fn mcp_config_json(command: &str) -> String {
    serde_json::json!({
        "mcpServers": {
            "gmail": {
                "command": command,
                "args": ["mcp", "gmail"]
            }
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_allowlist_and_validation() {
        assert!(assert_gmail_host("imap.gmail.com").is_ok());
        assert!(assert_gmail_host("imap.example.com").is_err());
        let bad = GmailConfig {
            email: "not-an-email".into(),
            app_password: "abcdefghij".into(),
        };
        assert!(validate(&bad).is_err());
        let short = GmailConfig {
            email: "a@b.com".into(),
            app_password: "short".into(),
        };
        assert!(validate(&short).is_err());
    }

    #[test]
    fn strips_spaces_from_app_password() {
        let secret = GmailSecret {
            email: "a@gmail.com".into(),
            app_password: "abcd efgh ijkl mnop".into(),
        };
        let cfg = GmailConfig::from(&secret);
        assert_eq!(cfg.app_password, "abcdefghijklmnop");
    }
}
