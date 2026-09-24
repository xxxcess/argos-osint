//! Markdown reports written for a case.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::search::SearchHit;
use crate::store::new_id;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReportMeta {
    pub id: String,
    pub case_id: Option<String>,
    pub title: String,
    pub path: String,
    pub created_at: String,
}

pub fn render_report(
    title: &str,
    case_id: Option<&str>,
    question: &str,
    body: &str,
    hits: &[SearchHit],
) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("title: {}\n", yaml_escape(title)));
    if let Some(id) = case_id {
        out.push_str(&format!("case: {id}\n"));
    }
    out.push_str(&format!("sources: {}\n", hits.len()));
    out.push_str(&format!(
        "created: {}\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    ));
    out.push_str("---\n\n");
    out.push_str(&format!("# {title}\n\n"));
    if !question.trim().is_empty() {
        out.push_str("## Question\n\n");
        out.push_str(question.trim());
        out.push_str("\n\n");
    }
    out.push_str("## Findings\n\n");
    out.push_str(body.trim());
    out.push_str("\n\n## Sources\n\n");
    if hits.is_empty() {
        out.push_str("_No public sources were attached to this report._\n");
    } else {
        for (i, hit) in hits.iter().enumerate() {
            out.push_str(&format!(
                "{}. [{}]({})\n",
                i + 1,
                hit.title.replace('\n', " "),
                hit.url
            ));
            if !hit.snippet.trim().is_empty() {
                out.push_str(&format!("   {}\n", hit.snippet.trim().replace('\n', " ")));
            }
        }
    }
    out
}

pub fn source_pack(title: &str, question: &str, hits: &[SearchHit]) -> String {
    let mut body = String::from("Collected from public search. Review the sources before treating any claim as established.\n\n");
    for hit in hits {
        body.push_str(&format!("- **{}** — {}\n", hit.title, hit.snippet.trim()));
    }
    render_report(title, None, question, &body, hits)
}

fn yaml_escape(text: &str) -> String {
    let t = text.replace('\n', " ").trim().to_string();
    if t.chars().any(|c| ":#{}[]&*!|>%@`'\"".contains(c)) {
        format!("\"{}\"", t.replace('"', "\\\""))
    } else {
        t
    }
}

pub fn write_report(
    dir: &Path,
    title: &str,
    case_id: Option<&str>,
    markdown: &str,
) -> std::io::Result<ReportMeta> {
    fs::create_dir_all(dir)?;
    let id = new_id("report");
    let slug = slugify(title);
    let path: PathBuf = dir.join(format!("{id}-{slug}.md"));
    fs::write(&path, markdown)?;
    Ok(ReportMeta {
        id,
        case_id: case_id.map(|s| s.to_string()),
        title: title.to_string(),
        path: path.display().to_string(),
        created_at: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    })
}

fn slugify(title: &str) -> String {
    let mut out = String::new();
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if out.chars().last().is_some_and(|c| c != '-') {
            out.push('-');
        }
        if out.len() >= 40 {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "note".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_contains_front_matter_and_sources() {
        let hits = vec![SearchHit {
            title: "Example".into(),
            url: "https://example.com".into(),
            snippet: "A page".into(),
        }];
        let md = render_report(
            "Harbor",
            Some("case-1"),
            "what changed?",
            "The tide turned.",
            &hits,
        );
        assert!(md.starts_with("---\n"));
        assert!(md.contains("case: case-1"));
        assert!(md.contains("## Findings"));
        assert!(md.contains("https://example.com"));
    }
}
