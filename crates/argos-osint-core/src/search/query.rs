//! Pull person, org, domain, handle, email, and topic out of a question.
//! The stages that need a domain, handle, or topic look at this and skip
//! when the question does not contain one.

use regex::Regex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextQuery {
    pub raw: String,
    pub persons: Vec<String>,
    pub orgs: Vec<String>,
    pub domains: Vec<String>,
    pub handles: Vec<String>,
    pub emails: Vec<String>,
    pub topics: Vec<String>,
}

impl TextQuery {
    pub fn extract(query: &str) -> Self {
        let raw = query.trim().to_string();
        let emails = find_emails(&raw);
        let mut working = blank_substrings(&raw, &emails);
        let mut handles = find_at_handles(&working);
        working = blank_at_handles(&working);
        let (profile_handles, profile_spans) = find_profile_handles(&working);
        for handle in profile_handles {
            push_unique(&mut handles, &handle);
        }
        working = blank_spans(&working, &profile_spans);
        let mut domains = find_domains(&working);
        for email in &emails {
            if let Some(host) = email.split('@').nth(1) {
                if let Some(domain) = normalize_domain(host) {
                    push_unique(&mut domains, &domain);
                }
            }
        }
        let orgs = find_orgs(&raw);
        let persons = find_persons(&raw, &orgs);
        let topics = topics_of(&raw, &emails, &handles, &domains);
        Self {
            raw,
            persons,
            orgs,
            domains,
            handles,
            emails,
            topics,
        }
    }

    pub fn has_social(&self) -> bool {
        !self.topics.is_empty() || !self.handles.is_empty()
    }

    pub fn has_identity(&self) -> bool {
        !self.handles.is_empty() || !self.emails.is_empty()
    }

    /// Names and organizations, or the topic, or the raw question.
    pub fn fact_terms(&self) -> Vec<String> {
        let mut terms = Vec::new();
        terms.extend(self.persons.iter().cloned());
        terms.extend(self.orgs.iter().cloned());
        if terms.is_empty() {
            if let Some(topic) = self.topics.first() {
                terms.push(topic.clone());
            } else if !self.raw.is_empty() {
                terms.push(self.raw.clone());
            }
        }
        terms.truncate(2);
        terms
    }

    pub fn social_query(&self) -> String {
        if let Some(topic) = self.topics.first() {
            return topic.clone();
        }
        self.handles.first().cloned().unwrap_or_default()
    }

    pub fn identity_terms(&self) -> Vec<String> {
        let mut terms = self.handles.clone();
        terms.extend(self.emails.iter().cloned());
        terms.truncate(3);
        terms
    }
}

fn find_emails(text: &str) -> Vec<String> {
    let re = Regex::new(r"(?i)\b[A-Z0-9._%+\-]+@[A-Z0-9.\-]+\.[A-Z]{2,24}\b").expect("email");
    let mut out = Vec::new();
    for found in re.find_iter(text) {
        let email = found.as_str();
        let host = email.split('@').nth(1).unwrap_or("");
        if normalize_domain(host).is_some() {
            push_unique(&mut out, &email.to_ascii_lowercase());
        }
    }
    out
}

fn find_at_handles(text: &str) -> Vec<String> {
    let re = Regex::new(r"(?:^|[^A-Za-z0-9_])@([A-Za-z0-9_]{2,32})\b").expect("handle");
    let mut out = Vec::new();
    for cap in re.captures_iter(text) {
        push_unique(&mut out, &cap[1]);
    }
    out
}

fn find_profile_handles(text: &str) -> (Vec<String>, Vec<(usize, usize)>) {
    let re = Regex::new(
        r"(?i)\b(?:https?://)?(?:www\.)?(?:github\.com|twitter\.com|x\.com)/([A-Za-z0-9_\-]{1,39})\b",
    )
    .expect("profile");
    let reserved = [
        "search",
        "about",
        "settings",
        "login",
        "signup",
        "explore",
        "features",
        "marketplace",
        "notifications",
        "orgs",
        "topics",
        "pulls",
        "issues",
    ];
    let mut handles = Vec::new();
    let mut spans = Vec::new();
    for cap in re.captures_iter(text) {
        let handle = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let whole = cap.get(0).map(|m| (m.start(), m.end())).unwrap_or((0, 0));
        spans.push(whole);
        if reserved
            .iter()
            .any(|name| handle.eq_ignore_ascii_case(name))
        {
            continue;
        }
        push_unique(&mut handles, handle);
    }
    (handles, spans)
}

fn find_domains(text: &str) -> Vec<String> {
    let re = Regex::new(r"(?i)\b(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,24}\b")
        .expect("domain");
    let mut out = Vec::new();
    for found in re.find_iter(text) {
        if let Some(domain) = normalize_domain(found.as_str()) {
            push_unique(&mut out, &domain);
        }
    }
    out
}

fn find_orgs(text: &str) -> Vec<String> {
    let suffixed = Regex::new(
        r"\b([A-Z][A-Za-z0-9&.'’-]*(?:\s+[A-Z][A-Za-z0-9&.'’-]*){0,4})\s+(Inc\.?|LLC|Ltd\.?|Corp\.?|Corporation|GmbH|Company|Foundation|University)\b",
    )
    .expect("org");
    let labeled = Regex::new(
        r"(?i)\b(?:org|organization|organisation|company)\s*:\s*([A-Za-z0-9][^,;\n]{1,80})",
    )
    .expect("org label");
    let mut out = Vec::new();
    for cap in suffixed.captures_iter(text) {
        let name = format!("{} {}", cap[1].trim(), cap[2].trim());
        push_unique(&mut out, &name);
    }
    for cap in labeled.captures_iter(text) {
        let name = trim_name(cap[1].trim());
        if !name.is_empty() {
            push_unique(&mut out, &name);
        }
    }
    out
}

fn find_persons(text: &str, orgs: &[String]) -> Vec<String> {
    let mut working = text.to_string();
    for org in orgs {
        working = working.replace(org, " ");
    }
    let who = Regex::new(
        r"(?i)\bwho\s+is\s+([A-Za-z][A-Za-z'.-]{0,40}(?:\s+[A-Za-z][A-Za-z'.-]{0,40}){0,3})",
    )
    .expect("who is");
    let titled =
        Regex::new(r"\b([A-Z][a-z'.-]{1,40}(?:\s+[A-Z][a-z'.-]{1,40}){1,3})\b").expect("name");
    let mut out = Vec::new();
    for cap in who.captures_iter(&working) {
        let name = trim_name(&cap[1]);
        if name.split_whitespace().count() >= 1 && name.chars().any(|ch| ch.is_alphabetic()) {
            push_unique(&mut out, &name);
        }
    }
    for cap in titled.captures_iter(&working) {
        let name = trim_name(&cap[1]);
        if name.split_whitespace().count() >= 2 {
            push_unique(&mut out, &name);
        }
    }
    out
}

fn topics_of(raw: &str, emails: &[String], handles: &[String], domains: &[String]) -> Vec<String> {
    let mut working = raw.to_string();
    working = blank_substrings(&working, emails);
    for handle in handles {
        working = blank_substrings(&working, &[format!("@{handle}")]);
    }
    working = blank_substrings(&working, domains);
    let collapsed = super::collapse_ws(&working);
    let stop = [
        "who", "is", "the", "and", "for", "from", "with", "that", "this", "about", "what", "when",
        "where", "was", "are", "you", "your", "how", "why", "does", "did", "not", "into", "onto",
        "over", "under", "his", "her", "its", "their", "have", "has",
    ];
    let topical = collapsed.split_whitespace().any(|word| {
        let word = word.trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
        word.len() >= 3 && !stop.contains(&word.to_ascii_lowercase().as_str())
    });
    if topical {
        vec![raw.to_string()]
    } else {
        Vec::new()
    }
}

fn trim_name(name: &str) -> String {
    let stop = [
        "and", "or", "at", "of", "for", "in", "on", "the", "who", "with", "from", "about",
    ];
    let words: Vec<&str> = name.split_whitespace().collect();
    let end = words
        .iter()
        .position(|word| stop.contains(&word.to_ascii_lowercase().as_str()))
        .unwrap_or(words.len());
    words[..end]
        .join(" ")
        .trim_matches(|ch: char| ch == '.' || ch == ',')
        .trim()
        .to_string()
}

pub(crate) fn normalize_domain(raw: &str) -> Option<String> {
    let name = raw
        .trim()
        .trim_end_matches('.')
        .trim_start_matches('.')
        .to_ascii_lowercase();
    if name.len() < 4 || name.len() > 253 || !name.contains('.') || name.contains("..") {
        return None;
    }
    if name.ends_with(".local") || name.ends_with(".internal") || name == "localhost" {
        return None;
    }
    let labels: Vec<&str> = name.split('.').collect();
    if labels.len() < 2 {
        return None;
    }
    let tld = *labels.last()?;
    if tld.len() < 2 || !tld.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return None;
    }
    const FILES: &[&str] = &[
        "pdf", "html", "htm", "png", "jpg", "jpeg", "gif", "svg", "txt", "md", "json", "xml",
        "zip", "csv", "doc", "docx", "xls", "ppt", "mp3", "mp4", "wav", "css", "js",
    ];
    if labels.len() == 2 && FILES.contains(&tld) {
        return None;
    }
    let ok = labels.iter().all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    });
    if ok {
        Some(name)
    } else {
        None
    }
}

fn push_unique(out: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    if out.iter().any(|item| item.eq_ignore_ascii_case(value)) {
        return;
    }
    out.push(value.to_string());
}

fn blank_substrings(text: &str, needles: &[String]) -> String {
    let mut out = text.to_string();
    for needle in needles {
        if needle.is_empty() {
            continue;
        }
        while let Some(index) = out.to_ascii_lowercase().find(&needle.to_ascii_lowercase()) {
            let end = index + needle.len();
            out.replace_range(index..end, &" ".repeat(needle.len()));
        }
    }
    out
}

fn blank_at_handles(text: &str) -> String {
    let re = Regex::new(r"(?:^|[^A-Za-z0-9_])@([A-Za-z0-9_]{2,32})\b").expect("handle");
    let spans: Vec<(usize, usize)> = re
        .captures_iter(text)
        .filter_map(|cap| cap.get(1).map(|m| (m.start().saturating_sub(1), m.end())))
        .collect();
    blank_spans(text, &spans)
}

fn blank_spans(text: &str, spans: &[(usize, usize)]) -> String {
    let mut bytes = text.as_bytes().to_vec();
    for (start, end) in spans {
        let start = (*start).min(bytes.len());
        let end = (*end).min(bytes.len());
        for byte in &mut bytes[start..end] {
            *byte = b' ';
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_a_person_without_a_domain() {
        let query = TextQuery::extract("who is Ada Lovelace");
        assert_eq!(query.persons, vec!["Ada Lovelace".to_string()]);
        assert!(query.domains.is_empty(), "{:?}", query.domains);
        assert!(query.has_social());
        assert!(!query.has_identity());
        assert_eq!(query.fact_terms(), vec!["Ada Lovelace".to_string()]);
    }

    #[test]
    fn extracts_a_domain_email_and_handle() {
        let domain = TextQuery::extract("example.com");
        assert_eq!(domain.domains, vec!["example.com".to_string()]);
        assert!(!domain.has_social());
        assert!(!domain.has_identity());

        let email = TextQuery::extract("ada@example.com");
        assert_eq!(email.emails, vec!["ada@example.com".to_string()]);
        assert_eq!(email.domains, vec!["example.com".to_string()]);
        assert!(email.has_identity());
        assert!(!email.has_social());

        let handle = TextQuery::extract("profile @torvalds");
        assert_eq!(handle.handles, vec!["torvalds".to_string()]);
        assert!(handle.has_social());
        assert!(handle.has_identity());
        assert!(handle.domains.is_empty());
    }

    #[test]
    fn extracts_an_org_and_a_profile_handle() {
        let org = TextQuery::extract("filings for OpenAI Inc");
        assert_eq!(org.orgs, vec!["OpenAI Inc".to_string()]);
        assert!(org.has_social());

        let profile = TextQuery::extract("https://github.com/octocat");
        assert_eq!(profile.handles, vec!["octocat".to_string()]);
        assert!(profile.domains.is_empty(), "{:?}", profile.domains);
    }

    #[test]
    fn lowercase_who_is_and_file_names() {
        let query = TextQuery::extract("who is elon musk?");
        assert_eq!(query.persons, vec!["elon musk".to_string()]);
        assert!(TextQuery::extract("notes in report.pdf").domains.is_empty());
    }
}
