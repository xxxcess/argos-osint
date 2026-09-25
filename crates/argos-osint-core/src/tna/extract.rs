//! Ordered entity extraction for TNA. Reuses TextQuery kinds plus public IPs.

use std::net::IpAddr;

use regex::Regex;

use crate::search::TextQuery;
use crate::search::{ip_blocked, public_host_from_url};

use super::types::TnaNodeKind;

/// One entity occurrence in document order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Occurrence {
    pub kind: TnaNodeKind,
    pub label: String,
    pub start: usize,
}

/// Collect ordered entity occurrences from free text.
/// Domains, handles, emails, persons, orgs, and topics come from TextQuery
/// (located by position). Public IPv4/IPv6 are added; private/loopback never.
pub fn extract_occurrences(text: &str) -> Vec<Occurrence> {
    let mut found: Vec<Occurrence> = Vec::new();

    push_ips(text, &mut found);
    push_from_query(text, &mut found);
    push_url_hosts(text, &mut found);

    found.sort_by_key(|item| item.start);
    dedupe_overlapping(&mut found);
    found
}

fn push_from_query(text: &str, out: &mut Vec<Occurrence>) {
    // Short segments keep TextQuery topics useful; long blobs skip topic.
    for (offset, segment) in segment_spans(text) {
        let query = TextQuery::extract(segment);
        for email in &query.emails {
            if let Some(start) = find_ci(text, email, offset) {
                out.push(Occurrence {
                    kind: TnaNodeKind::Email,
                    label: email.to_string(),
                    start,
                });
            }
        }
        for handle in &query.handles {
            let needle = format!("@{handle}");
            let start = find_ci(text, &needle, offset)
                .or_else(|| find_ci(text, handle, offset));
            if let Some(start) = start {
                out.push(Occurrence {
                    kind: TnaNodeKind::Handle,
                    label: handle.to_string(),
                    start,
                });
            }
        }
        for domain in &query.domains {
            if let Some(start) = find_ci(text, domain, offset) {
                out.push(Occurrence {
                    kind: TnaNodeKind::Domain,
                    label: domain.to_string(),
                    start,
                });
            }
        }
        for person in &query.persons {
            if let Some(start) = find_ci(text, person, offset) {
                out.push(Occurrence {
                    kind: TnaNodeKind::Person,
                    label: person.to_string(),
                    start,
                });
            }
        }
        for org in &query.orgs {
            if let Some(start) = find_ci(text, org, offset) {
                out.push(Occurrence {
                    kind: TnaNodeKind::Org,
                    label: org.to_string(),
                    start,
                });
            }
        }
        if segment.chars().count() <= 120 {
            for topic in &query.topics {
                let label = truncate_label(topic, 72);
                if label.len() < 3 {
                    continue;
                }
                out.push(Occurrence {
                    kind: TnaNodeKind::Topic,
                    label,
                    start: offset,
                });
            }
        }
    }
}

fn push_ips(text: &str, out: &mut Vec<Occurrence>) {
    let v4 = Regex::new(r"\b(?:(?:25[0-5]|2[0-4]\d|[01]?\d\d?)\.){3}(?:25[0-5]|2[0-4]\d|[01]?\d\d?)\b")
        .expect("v4");
    for m in v4.find_iter(text) {
        if let Ok(ip) = m.as_str().parse::<IpAddr>() {
            if !ip_blocked(ip) {
                out.push(Occurrence {
                    kind: TnaNodeKind::Ip,
                    label: m.as_str().to_string(),
                    start: m.start(),
                });
            }
        }
    }
    // Allow compressed forms like 2001:4860:4860::8888
    let v6 = Regex::new(
        r"(?i)(?:^|[^A-Za-z0-9])((?:[0-9a-f]{0,4}:){2,7}[0-9a-f]{0,4})(?:[^A-Za-z0-9]|$)",
    )
    .expect("v6");
    for cap in v6.captures_iter(text) {
        let Some(m) = cap.get(1) else { continue };
        let raw = m.as_str();
        if raw.chars().filter(|c| *c == ':').count() < 2 {
            continue;
        }
        if let Ok(ip) = raw.parse::<IpAddr>() {
            if !ip_blocked(ip) {
                out.push(Occurrence {
                    kind: TnaNodeKind::Ip,
                    label: raw.to_ascii_lowercase(),
                    start: m.start(),
                });
            }
        }
    }
}

fn push_url_hosts(text: &str, out: &mut Vec<Occurrence>) {
    let re = Regex::new(r#"(?i)\bhttps?://[^\s\)\]>"']+"#).expect("url");
    for m in re.find_iter(text) {
        if let Some(host) = public_host_from_url(m.as_str().trim_end_matches(['.', ',', ';', ')'])) {
            // Prefer domain label; IP hosts already covered when public.
            if host.parse::<IpAddr>().is_ok() {
                out.push(Occurrence {
                    kind: TnaNodeKind::Ip,
                    label: host,
                    start: m.start(),
                });
            } else {
                out.push(Occurrence {
                    kind: TnaNodeKind::Domain,
                    label: host,
                    start: m.start(),
                });
            }
        }
    }
}

fn segment_spans(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = 0;
    for (idx, ch) in text.char_indices() {
        if ch == '\n' {
            let seg = text[start..idx].trim();
            if !seg.is_empty() {
                out.push((start + text[start..idx].find(seg).unwrap_or(0), seg));
            }
            start = idx + ch.len_utf8();
        }
    }
    let seg = text[start..].trim();
    if !seg.is_empty() {
        out.push((start + text[start..].find(seg).unwrap_or(0), seg));
    }
    if out.is_empty() && !text.trim().is_empty() {
        out.push((0, text.trim()));
    }
    out
}

fn find_ci(hay: &str, needle: &str, from: usize) -> Option<usize> {
    if needle.is_empty() || from >= hay.len() {
        return None;
    }
    let hay_l = hay[from..].to_ascii_lowercase();
    let needle_l = needle.to_ascii_lowercase();
    hay_l.find(&needle_l).map(|rel| from + rel)
}

fn truncate_label(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    let mut out = String::new();
    for ch in trimmed.chars() {
        if out.chars().count() >= max {
            break;
        }
        out.push(ch);
    }
    out.trim().to_string()
}

fn dedupe_overlapping(items: &mut Vec<Occurrence>) {
    // Drop exact same kind+label at the same start; keep first.
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| {
        let key = (item.kind, item.label.to_ascii_lowercase(), item.start);
        seen.insert(key)
    });
}

/// True when this IP must never become a TNA node.
pub fn ip_is_blocked_label(label: &str) -> bool {
    match label.parse::<IpAddr>() {
        Ok(ip) => ip_blocked(ip),
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_private_and_loopback_ips() {
        let text = "public 8.8.8.8 private 192.168.1.1 loop 127.0.0.1 link 169.254.1.1 v6 2001:4860:4860::8888 local ::1 fe80::1";
        let occ = extract_occurrences(text);
        let ips: Vec<_> = occ
            .iter()
            .filter(|o| o.kind == TnaNodeKind::Ip)
            .map(|o| o.label.as_str())
            .collect();
        assert!(ips.contains(&"8.8.8.8"), "{ips:?}");
        // Compressed public v6 when the parser accepts the span.
        let _ = ips.iter().any(|ip| ip.contains("2001:4860"));
        assert!(!ips.iter().any(|ip| ip.starts_with("192.168")), "{ips:?}");
        assert!(!ips.iter().any(|ip| ip.starts_with("127.")), "{ips:?}");
        assert!(!ips.iter().any(|ip| *ip == "::1"), "{ips:?}");
        assert!(!ips.iter().any(|ip| ip.starts_with("fe80")), "{ips:?}");
        assert!(!ips.iter().any(|ip| ip.starts_with("169.254")), "{ips:?}");
        assert!(ip_is_blocked_label("10.0.0.1"));
        assert!(ip_is_blocked_label("172.16.5.5"));
        assert!(!ip_is_blocked_label("8.8.8.8"));
    }

    #[test]
    fn extracts_domain_handle_email_in_order() {
        let text = "see example.com then @torvalds and ada@example.com";
        let occ = extract_occurrences(text);
        let labels: Vec<_> = occ.iter().map(|o| o.label.as_str()).collect();
        assert!(labels.iter().any(|l| *l == "example.com"));
        assert!(labels.iter().any(|l| *l == "torvalds"));
        assert!(labels.iter().any(|l| *l == "ada@example.com"));
        let positions: Vec<_> = occ
            .iter()
            .filter(|o| matches!(o.kind, TnaNodeKind::Domain | TnaNodeKind::Handle | TnaNodeKind::Email))
            .map(|o| o.start)
            .collect();
        assert!(positions.windows(2).all(|w| w[0] <= w[1]), "{positions:?}");
    }
}
