//! Collection and targeted corpus builders for TNA.

use std::fs;
use std::path::Path;

use anyhow::Result;
use regex::Regex;

use crate::report::ReportMeta;
use crate::store::Store;

use super::types::TnaScope;

/// One report's contribution to a corpus.
#[derive(Clone, Debug)]
pub struct CorpusDoc {
    pub report_id: String,
    pub title: String,
    pub text: String,
}

/// Prepared text inputs for `build_snapshot`.
#[derive(Clone, Debug)]
pub struct TnaCorpus {
    pub scope: TnaScope,
    pub docs: Vec<CorpusDoc>,
}

impl TnaCorpus {
    pub fn is_collection(&self) -> bool {
        matches!(self.scope, TnaScope::Collection)
    }

    /// Desk / collection: every completed report in the store.
    pub fn collection(store: &Store) -> Result<Self> {
        let reports = store.list_reports()?;
        let memories = store.list_memories().unwrap_or_default();
        let mut docs = Vec::new();
        for report in reports {
            let text = report_corpus_text(&report, &memories);
            docs.push(CorpusDoc {
                report_id: report.id.clone(),
                title: report.title.clone(),
                text,
            });
        }
        Ok(Self {
            scope: TnaScope::Collection,
            docs,
        })
    }

    /// Targeted: one report only (no Doc node in the graph).
    pub fn targeted(store: &Store, report_id: &str) -> Result<Self> {
        let reports = store.list_reports()?;
        let Some(report) = reports.into_iter().find(|r| r.id == report_id) else {
            anyhow::bail!("report {report_id} not found");
        };
        let memories = store.list_memories().unwrap_or_default();
        let text = report_corpus_text(&report, &memories);
        Ok(Self {
            scope: TnaScope::Targeted {
                report_id: report.id.clone(),
                title: report.title.clone(),
            },
            docs: vec![CorpusDoc {
                report_id: report.id,
                title: report.title,
                text,
            }],
        })
    }
}

fn report_corpus_text(report: &ReportMeta, memories: &[crate::brain::Memory]) -> String {
    let mut parts = Vec::new();
    parts.push(report.title.clone());

    let body = fs::read_to_string(Path::new(&report.path)).unwrap_or_default();
    if let Some(req) = frontmatter_value(&body, "requirement") {
        parts.push(req);
    }
    if let Some(section) = markdown_section(&body, "Requirement") {
        parts.push(section);
    }
    if let Some(section) = markdown_section(&body, "Evidence") {
        parts.push(section.clone());
        parts.extend(url_hosts_from_markdown(&section));
    }
    parts.extend(url_hosts_from_markdown(&body));
    if let Some(section) = markdown_section(&body, "Analyst note") {
        parts.push(section);
    }
    for memory in memories {
        if memory.report_id.as_deref() == Some(report.id.as_str()) {
            parts.push(memory.text.clone());
        }
    }
    parts.join("\n")
}

fn frontmatter_value(md: &str, key: &str) -> Option<String> {
    let trimmed = md.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    let rest = trimmed.trim_start_matches("---");
    let end = rest.find("\n---")?;
    let block = &rest[..end];
    for line in block.lines() {
        let line = line.trim();
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case(key) {
                let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn markdown_section(md: &str, heading: &str) -> Option<String> {
    let needle = format!("## {heading}");
    let lower = md.to_ascii_lowercase();
    let needle_l = needle.to_ascii_lowercase();
    let start = lower.find(&needle_l)?;
    let after = start + needle.len();
    let rest = &md[after..];
    let rest_l = rest.to_ascii_lowercase();
    let end = rest_l
        .find("\n## ")
        .map(|i| i)
        .unwrap_or(rest.len());
    let body = rest[..end].trim();
    if body.is_empty() {
        None
    } else {
        Some(body.to_string())
    }
}

fn url_hosts_from_markdown(md: &str) -> Vec<String> {
    let re = Regex::new(r#"(?i)\bhttps?://[^\s\)\]>"']+"#).expect("url");
    let mut out = Vec::new();
    for m in re.find_iter(md) {
        if let Some(host) =
            crate::search::public_host_from_url(m.as_str().trim_end_matches(['.', ',', ';', ')']))
        {
            out.push(host);
        }
    }
    out
}
