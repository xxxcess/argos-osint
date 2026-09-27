//! Collection and targeted corpus builders for TNA.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

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
        let mut docs = Vec::new();
        for report in reports {
            let text = report_corpus_text(&report)?;
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
        let text = report_corpus_text(&report)?;
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

// Keep the original Markdown so cleanup can retain byte spans and section provenance.
fn report_corpus_text(report: &ReportMeta) -> Result<String> {
    fs::read_to_string(Path::new(&report.path))
        .with_context(|| format!("read TNA report {} ({})", report.id, report.path))
}
