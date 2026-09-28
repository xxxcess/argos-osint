//! Case projections read the shared evidence store, never report text or providers.
use crate::{evidence::*, research::ResearchJob, store::Store};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!("Case projection cancelled");
    }
    Ok(())
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InvestigationScope {
    pub question: String,
    pub seed_entities: Vec<String>,
    pub included_evidence: Vec<EvidenceRef>,
    pub allowed_actions: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct CaseLink {
    pub relationship: Relationship,
    pub observations: Vec<String>,
    pub reviewed: bool,
    pub candidate: bool,
}
#[derive(Clone, Debug, Default)]
pub struct CaseProjection {
    pub entities: Vec<Entity>,
    pub findings: Vec<Finding>,
    pub links: Vec<CaseLink>,
    pub timeline: Vec<TimelineEntry>,
    pub jobs: Vec<ResearchJob>,
    pub scope: Option<InvestigationScope>,
    pub snapshot: Option<crate::tna::TnaSnapshot>,
}

/// Conservative normalization before a label enters a case view. Originals stay in records.
pub fn normalize_entity(label: &str, kind: EntityType) -> Result<Entity> {
    let original = label;
    let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
    if label.is_empty() || label.len() > 200 || original.chars().any(char::is_control) {
        bail!("Empty, overlong, or control-bearing label");
    }
    let (prefix, label) = match kind {
        EntityType::Domain => {
            let host = crate::search::query::normalize_domain(&label)
                .ok_or_else(|| anyhow::anyhow!("Malformed or non-domain label"))?;
            let tld = host.rsplit('.').next().unwrap_or("");
            if tld != "example"
                && !include_str!("tna/tlds.txt")
                    .lines()
                    .any(|s| s.eq_ignore_ascii_case(tld))
            {
                bail!("Unrecognized domain suffix");
            }
            ("domain", host)
        }
        EntityType::Ip => ("ip", label.parse::<std::net::IpAddr>()?.to_string()),
        EntityType::Email => {
            let (local, host) = label
                .split_once('@')
                .ok_or_else(|| anyhow::anyhow!("Malformed email"))?;
            if local.is_empty() || local.contains([' ', '/', '@']) || host.contains('@') {
                bail!("Malformed email");
            }
            let domain = normalize_entity(host, EntityType::Domain)?.label;
            ("email", format!("{}@{domain}", local.to_ascii_lowercase()))
        }
        EntityType::Account => {
            let value = label.trim_start_matches('@');
            if value.is_empty()
                || !value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
            {
                bail!("Malformed account label");
            }
            ("account", value.to_string())
        }
        EntityType::Person => ("person", label),
        EntityType::Organization => ("org", label),
        EntityType::Theme => ("topic", label),
        EntityType::Document => ("document", label),
        EntityType::BreachEvent => ("breach", label),
    };
    let id = format!("{prefix}:{}", label.to_lowercase());
    Ok(Entity {
        id: id.clone(),
        canonical: id,
        label,
        kind,
        platform: None,
        aliases: vec![original.into()],
    })
}

impl Store {
    pub fn case_projection(&self, case_id: &str) -> Result<CaseProjection> {
        self.case_projection_cancellable(case_id, &AtomicBool::new(false))
    }
    pub fn case_projection_cancellable(
        &self,
        case_id: &str,
        cancel: &AtomicBool,
    ) -> Result<CaseProjection> {
        let transaction = self.conn.unchecked_transaction()?;
        self.check_case_write(case_id, None)?;
        check_cancel(cancel)?;
        let scope = EvidenceScope::Case(case_id.into());
        self.ingest_case_reports_cancellable(case_id, cancel)?;
        check_cancel(cancel)?;
        let findings = self.findings_scoped(&scope)?;
        let mut entities = self
            .records_scoped::<Entity>("entity", &scope)?
            .into_iter()
            .filter(|e| normalize_entity(&e.label, e.kind.clone()).is_ok())
            .collect::<Vec<_>>();
        let mut accepted_counts = std::collections::HashMap::<String, usize>::new();
        let mut seen = std::collections::HashSet::new();
        for f in &findings {
            let o = &f.observation;
            let targets = o
                .evidence
                .iter()
                .filter_map(|r| {
                    r.passage_id
                        .as_ref()
                        .or(r.source_url.as_ref())
                        .or(r.artifact_id.as_ref())
                })
                .cloned()
                .collect::<Vec<_>>();
            if f.decision == Some(ReviewDecision::Accept)
                && seen.insert((
                    o.entity_id.clone(),
                    o.provider.clone(),
                    o.statement.clone(),
                    targets,
                ))
            {
                *accepted_counts.entry(o.entity_id.clone()).or_default() += 1;
            }
        }
        entities.sort_by_key(|e| {
            (
                std::cmp::Reverse(accepted_counts.get(&e.id).copied().unwrap_or(0)),
                e.label.clone(),
            )
        });
        entities.dedup_by(|a, b| a.id == b.id);
        let mut links = Vec::new();
        let mut stmt = self.conn.prepare(
            "SELECT id,body FROM evidence_records WHERE kind='relationship' AND case_id=?1",
        )?;
        let relations = stmt
            .query_map([case_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (record_id, body) in relations {
            check_cancel(cancel)?;
            let mut relationship: Relationship = serde_json::from_str(&body)?;
            // Earlier jobs named relationship records after the generating observation.
            // Recover their source binding without rewriting stored history.
            if relationship
                .evidence
                .iter()
                .all(|r| r.artifact_id.is_none() && r.passage_id.is_none())
            {
                if let Some(f) = findings
                    .iter()
                    .find(|f| record_id.starts_with(&format!("{}:relationship:", f.observation.id)))
                {
                    let id = format!("{}:artifact", f.observation.id);
                    if self
                        .records_scoped::<Artifact>("artifact", &scope)?
                        .iter()
                        .any(|a| a.id == id)
                    {
                        for r in &mut relationship.evidence {
                            r.artifact_id = Some(id.clone());
                        }
                    }
                }
            }
            let supporting = findings
                .iter()
                .filter(|f| {
                    f.observation.evidence.iter().any(|a| {
                        relationship.evidence.iter().any(|b| {
                            a.artifact_id
                                .as_ref()
                                .is_some_and(|id| b.artifact_id.as_ref() == Some(id))
                                || a.passage_id
                                    .as_ref()
                                    .is_some_and(|id| b.passage_id.as_ref() == Some(id))
                                || b.artifact_id.as_deref()
                                    == Some(format!("{}:artifact", f.observation.id).as_str())
                        })
                    })
                })
                .collect::<Vec<_>>();
            // Rejected and deferred evidence cannot promote a link. Pending links stay candidates.
            let reviewed = supporting
                .iter()
                .any(|f| f.decision == Some(ReviewDecision::Accept));
            let pending = supporting
                .iter()
                .any(|f| f.decision.is_none() || f.decision == Some(ReviewDecision::Retain));
            if !reviewed && !pending {
                continue;
            }
            if !relationship
                .evidence
                .iter()
                .any(|r| self.resolve_evidence(r, &scope).is_ok())
            {
                continue;
            }
            let candidate = !reviewed
                || matches!(
                    relationship.kind,
                    RelationshipType::CandidateIdentityAssociation
                        | RelationshipType::TextCooccurrence
                );
            links.push(CaseLink {
                observations: supporting
                    .iter()
                    .map(|f| f.observation.id.clone())
                    .collect(),
                relationship,
                reviewed,
                candidate,
            });
        }
        // One typed link per pair/basis; repeated observations retain all source/history targets.
        let mut unique = Vec::<CaseLink>::new();
        for link in links {
            if let Some(previous) = unique.iter_mut().find(|old| {
                old.relationship.from == link.relationship.from
                    && old.relationship.to == link.relationship.to
                    && old.relationship.kind == link.relationship.kind
                    && old.relationship.basis == link.relationship.basis
            }) {
                previous.observations.extend(link.observations);
                previous.observations.sort();
                previous.observations.dedup();
                for reference in link.relationship.evidence {
                    if !previous.relationship.evidence.contains(&reference) {
                        previous.relationship.evidence.push(reference);
                    }
                }
                previous.reviewed |= link.reviewed;
                previous.candidate &= link.candidate;
                if previous.relationship.uncertainty != link.relationship.uncertainty {
                    previous
                        .relationship
                        .uncertainty
                        .push_str(&format!("; {}", link.relationship.uncertainty));
                }
            } else {
                unique.push(link);
            }
        }
        let mut links = unique;
        // Older provider jobs may have omitted endpoint entities. Materialize only
        // valid source-backed identifiers, preserving their original relationship IDs.
        for link in &links {
            for endpoint in [&link.relationship.from, &link.relationship.to] {
                if !entities.iter().any(|e| &e.id == endpoint) {
                    if let Some((prefix, label)) = endpoint.split_once(':') {
                        let kind = match prefix {
                            "domain" => EntityType::Domain,
                            "ip" => EntityType::Ip,
                            "email" => EntityType::Email,
                            _ => continue,
                        };
                        if let Ok(mut entity) = normalize_entity(label, kind) {
                            entity.id = endpoint.clone();
                            self.put_record(endpoint, "entity", Some(case_id), None, &entity)?;
                            entities.push(entity);
                        }
                    }
                }
            }
        }
        links.retain(|l| {
            entities.iter().any(|e| e.id == l.relationship.from)
                && entities.iter().any(|e| e.id == l.relationship.to)
        });
        let corrections =
            self.records::<EntityCorrection>("entity_correction", None, Some(case_id))?;
        let reversed = corrections
            .iter()
            .filter_map(|c| c.reverses.as_ref())
            .collect::<std::collections::HashSet<_>>();
        for correction in corrections
            .iter()
            .filter(|c| c.reverses.is_none() && !reversed.contains(&c.id))
        {
            if let Some(label) = &correction.replacement {
                if let Some(entity) = entities.iter_mut().find(|e| e.id == correction.entity_id) {
                    entity.aliases.push(entity.label.clone());
                    entity.label = label.clone();
                }
            } else {
                entities.retain(|e| e.id != correction.entity_id);
                links.retain(|l| {
                    l.relationship.from != correction.entity_id
                        && l.relationship.to != correction.entity_id
                });
            }
        }
        let decisions =
            self.records::<IdentityDecision>("identity_resolution", None, Some(case_id))?;
        let reversed = decisions
            .iter()
            .filter_map(|d| d.reverses.as_ref())
            .collect::<std::collections::HashSet<_>>();
        for decision in decisions
            .iter()
            .filter(|d| d.reverses.is_none() && !reversed.contains(&d.id))
        {
            if let Some(canonical) = &decision.canonical_entity {
                let aliases = entities
                    .iter()
                    .filter(|e| decision.entities.contains(&e.id))
                    .flat_map(|e| std::iter::once(e.label.clone()).chain(e.aliases.clone()))
                    .collect::<Vec<_>>();
                if let Some(entity) = entities.iter_mut().find(|e| &e.id == canonical) {
                    entity.aliases.extend(aliases);
                    entity.aliases.sort();
                    entity.aliases.dedup();
                }
                entities.retain(|e| !decision.entities.contains(&e.id) || &e.id == canonical);
                for link in &mut links {
                    if decision.entities.contains(&link.relationship.from) {
                        link.relationship.from = canonical.clone();
                    }
                    if decision.entities.contains(&link.relationship.to) {
                        link.relationship.to = canonical.clone();
                    }
                }
                links.retain(|l| l.relationship.from != l.relationship.to);
            }
        }
        let mut projection = CaseProjection {
            entities,
            findings,
            links,
            timeline: self.timeline_scoped(&scope)?,
            jobs: self
                .jobs()?
                .into_iter()
                .filter(|j| j.input.case_id.as_deref() == Some(case_id))
                .collect(),
            snapshot: None,
            scope: self
                .records::<InvestigationScope>("investigation_scope", None, Some(case_id))?
                .pop(),
        };
        check_cancel(cancel)?;
        let title = self
            .list_cases()?
            .into_iter()
            .find(|c| c.id == case_id)
            .map(|c| c.title)
            .unwrap_or_else(|| "Case evidence".into());
        projection.snapshot = Some(projection.network_snapshot(&title));
        self.upsert_tna_graph(
            &format!("case:{case_id}"),
            projection.snapshot.as_ref().unwrap(),
        )?;
        transaction.commit()?;
        Ok(projection)
    }
    /// Resolve a local immutable source target only after checking its evidence scope.
    pub fn resolve_evidence(
        &self,
        reference: &EvidenceRef,
        scope: &EvidenceScope,
    ) -> Result<String> {
        if let Some(id) = &reference.artifact_id {
            if let Some(a) = self
                .records_scoped::<Artifact>("artifact", scope)?
                .into_iter()
                .find(|a| &a.id == id)
            {
                return Ok(format!(
                    "{}\nRetrieved {}\n{}",
                    a.source_url.as_deref().unwrap_or("local artifact"),
                    a.retrieved_at,
                    a.body
                ));
            }
        }
        if let Some(id) = &reference.passage_id {
            if let Some(p) = self.passage(id)? {
                let reports = self.list_reports()?;
                let allowed = reports
                    .iter()
                    .find(|r| r.id == p.report_id)
                    .is_some_and(|r| match scope {
                        EvidenceScope::Collection => true,
                        EvidenceScope::Desk => r.case_id.is_none(),
                        EvidenceScope::Case(id) => r.case_id.as_ref() == Some(id),
                        EvidenceScope::Report(id) => &r.id == id,
                        EvidenceScope::Reports(ids) => ids.contains(&r.id),
                    });
                let included = if let EvidenceScope::Case(id) = scope {
                    self.records::<InvestigationScope>("investigation_scope", None, Some(id))?
                        .iter()
                        .any(|s| {
                            s.included_evidence
                                .iter()
                                .any(|r| r.passage_id.as_ref() == Some(&p.id))
                        })
                } else {
                    false
                };
                if allowed || included {
                    return Ok(format!("[{}] {}\n{}", p.citation(), p.title, p.text));
                }
            }
        }
        bail!("Source unavailable or outside selected scope")
    }
    pub fn retrieve_reviewed(
        &self,
        query: &str,
        scope: &EvidenceScope,
        limit: usize,
    ) -> Result<Vec<Finding>> {
        let terms = query
            .split_whitespace()
            .map(|t| {
                t.trim_matches(|c: char| !c.is_alphanumeric() && c != '@')
                    .to_lowercase()
            })
            .filter(|t| {
                t.len() > 2
                    && !["what", "who", "where", "does", "the", "about", "evidence"]
                        .contains(&t.as_str())
            })
            .collect::<Vec<_>>();
        let exact = crate::search::TextQuery::extract(query);
        let observables = exact
            .domains
            .into_iter()
            .chain(exact.emails)
            .collect::<Vec<_>>();
        Ok(self
            .findings_scoped(scope)?
            .into_iter()
            .filter(|f| {
                let o = &f.observation;
                let text =
                    format!("{} {} {}", o.entity_id, o.statement, o.attribution).to_lowercase();
                f.decision == Some(ReviewDecision::Accept)
                    && terms.iter().any(|t| text.contains(t))
                    && (observables.is_empty()
                        || observables
                            .iter()
                            .any(|v| crate::evidence::exact_observable(&text, v)))
                    && o.evidence
                        .iter()
                        .any(|r| self.resolve_evidence(r, scope).is_ok())
            })
            .take(limit.min(100))
            .collect())
    }
    pub fn draft_case_report(
        &self,
        case_id: &str,
        mode: &str,
        selected: &[String],
        directory: &std::path::Path,
    ) -> Result<crate::report::ReportMeta> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        self.check_case_write(case_id, None)?;
        if !["addendum", "revision", "followup", "final"].contains(&mode) {
            bail!("Choose addendum, revision, followup, or final");
        }
        let case = self
            .list_cases()?
            .into_iter()
            .find(|c| c.id == case_id)
            .ok_or_else(|| anyhow::anyhow!("Case not found"))?;
        let scope = EvidenceScope::Case(case_id.into());
        let findings = self.findings_scoped(&scope)?;
        let accepted = findings
            .iter()
            .filter(|f| {
                selected.contains(&f.observation.id) && f.decision == Some(ReviewDecision::Accept)
            })
            .collect::<Vec<_>>();
        if accepted.is_empty()
            || selected
                .iter()
                .any(|id| !accepted.iter().any(|f| &f.observation.id == id))
        {
            bail!("Select accepted observation IDs within this case before drafting");
        }
        let mut body = format!("# {mode}: {}\n\n## Reviewed evidence\n", case.title);
        for f in accepted {
            let o = &f.observation;
            body.push_str(&format!(
                "\n{}\n\nAttribution: {} · basis {:?} · event {} · retrieved {}\n",
                o.statement,
                o.attribution,
                o.basis,
                o.event_time.as_deref().unwrap_or("undated"),
                o.retrieved_at
            ));
            for r in &o.evidence {
                body.push_str(&format!(
                    "\n[{}] {}\n",
                    o.id,
                    self.resolve_evidence(r, &scope)?
                ));
            }
        }
        body.push_str("\n## Uncertainties and unresolved gaps\n\nAcceptance records an analyst review, not independent verification. Candidate identity associations remain provisional. Uncollected evidence is not a negative finding.\n");
        for f in findings
            .iter()
            .filter(|f| {
                f.decision != Some(ReviewDecision::Accept)
                    && f.decision != Some(ReviewDecision::Reject)
            })
            .take(30)
        {
            body.push_str(&format!("- Pending {}: {}\n", f.observation.id, f.category));
        }
        let report = crate::report::write_report(
            directory,
            &format!("{mode}: {}", case.title),
            Some(case_id),
            &body,
        )?;
        self.add_report(&report)?;
        transaction.commit()?;
        Ok(report)
    }
}

pub fn distinct_findings<'a>(findings: impl Iterator<Item = &'a Finding>) -> usize {
    findings
        .map(|f| {
            let o = &f.observation;
            let targets = o
                .evidence
                .iter()
                .filter_map(|r| {
                    r.passage_id
                        .as_ref()
                        .or(r.source_url.as_ref())
                        .or(r.artifact_id.as_ref())
                })
                .cloned()
                .collect::<Vec<_>>();
            (
                o.entity_id.clone(),
                o.provider.clone(),
                o.statement.clone(),
                targets,
            )
        })
        .collect::<std::collections::HashSet<_>>()
        .len()
}
impl CaseProjection {
    /// Compatibility projection for Advanced analysis; typed evidence remains in the inspector.
    pub fn network_snapshot(&self, title: &str) -> crate::tna::TnaSnapshot {
        use crate::tna::*;
        let mut snapshot = TnaSnapshot::empty(TnaScope::Selected {
            report_ids: vec![],
            label: title.into(),
        });
        snapshot.nodes = self
            .entities
            .iter()
            .take(160)
            .enumerate()
            .map(|(index, e)| {
                let kind = match e.kind {
                    EntityType::Domain => TnaNodeKind::Domain,
                    EntityType::Ip => TnaNodeKind::Ip,
                    EntityType::Email => TnaNodeKind::Email,
                    EntityType::Account => TnaNodeKind::Handle,
                    EntityType::Person => TnaNodeKind::Person,
                    EntityType::Organization => TnaNodeKind::Org,
                    EntityType::Document => TnaNodeKind::Doc,
                    _ => TnaNodeKind::Topic,
                };
                TnaNode {
                    id: e.id.clone(),
                    label: e.label.clone(),
                    kind,
                    cluster: kind.cluster(),
                    mentions: self
                        .findings
                        .iter()
                        .filter(|f| f.observation.entity_id == e.id)
                        .count() as u32,
                    degree: self
                        .links
                        .iter()
                        .filter(|l| {
                            l.reviewed
                                && !l.candidate
                                && (l.relationship.from == e.id || l.relationship.to == e.id)
                        })
                        .count() as u32,
                    x: 0.5
                        + 0.4
                            * (index as f64 * std::f64::consts::TAU
                                / self.entities.len().clamp(1, 160) as f64)
                                .cos(),
                    y: 0.5
                        + 0.4
                            * (index as f64 * std::f64::consts::TAU
                                / self.entities.len().clamp(1, 160) as f64)
                                .sin(),
                }
            })
            .collect();
        let ids = snapshot
            .nodes
            .iter()
            .map(|n| n.id.clone())
            .collect::<std::collections::HashSet<_>>();
        snapshot.clusters = TnaCluster::all()
            .iter()
            .map(|cluster| TnaClusterSummary {
                cluster: *cluster,
                node_count: snapshot
                    .nodes
                    .iter()
                    .filter(|n| n.cluster == *cluster)
                    .count() as u32,
            })
            .collect();
        snapshot.anchors = snapshot
            .nodes
            .iter()
            .map(|n| TnaAnchor {
                node_id: n.id.clone(),
                degree: n.degree,
                mentions: n.mentions,
            })
            .collect();
        snapshot
            .anchors
            .sort_by_key(|a| std::cmp::Reverse(a.degree));
        snapshot.anchors.truncate(8);
        snapshot.edges = self
            .links
            .iter()
            .filter(|l| {
                l.reviewed
                    && !l.candidate
                    && ids.contains(&l.relationship.from)
                    && ids.contains(&l.relationship.to)
            })
            .map(|l| TnaEdge {
                from: l.relationship.from.clone(),
                to: l.relationship.to.clone(),
                weight: l.observations.len() as u32,
            })
            .collect();
        snapshot
    }
}

impl Store {
    fn ingest_included_passage(&self, case_id: &str, passage_id: &str) -> Result<()> {
        let Some(passage) = self.passage(passage_id)? else {
            return Ok(());
        };
        let marker = format!(
            "{case_id}:included:{passage_id}:{}",
            crate::tna::PIPELINE_VERSION
        );
        if self
            .records::<String>("case_ingestion", Some(&passage.report_id), Some(case_id))?
            .contains(&marker)
        {
            return Ok(());
        }
        let Some(body) = self.report_version(&passage.report_id, Some(passage.version))? else {
            return Ok(());
        };
        let corpus = crate::tna::TnaCorpus {
            scope: crate::tna::TnaScope::Targeted {
                report_id: passage.report_id.clone(),
                title: passage.title.clone(),
            },
            docs: vec![crate::tna::CorpusDoc {
                report_id: passage.report_id.clone(),
                title: passage.title.clone(),
                text: body,
            }],
        };
        let snapshot = crate::tna::build_snapshot(&corpus);
        let reference = EvidenceRef {
            report_id: Some(passage.report_id.clone()),
            passage_id: Some(passage_id.into()),
            ..Default::default()
        };
        let mut entities = Vec::new();
        for d in snapshot
            .decisions
            .iter()
            .filter(|d| d.start >= passage.start && d.end <= passage.end)
        {
            if let (Some(id), Some(label)) = (&d.canonical_id, &d.label) {
                let kind = match d.kind {
                    crate::tna::TnaNodeKind::Domain => EntityType::Domain,
                    crate::tna::TnaNodeKind::Ip => EntityType::Ip,
                    crate::tna::TnaNodeKind::Email => EntityType::Email,
                    crate::tna::TnaNodeKind::Handle => EntityType::Account,
                    crate::tna::TnaNodeKind::Org => EntityType::Organization,
                    crate::tna::TnaNodeKind::Person => EntityType::Person,
                    _ => EntityType::Theme,
                };
                if let Ok(mut e) = normalize_entity(label, kind) {
                    e.id = id.clone();
                    self.put_record(id, "entity", Some(case_id), Some(&passage.report_id), &e)?;
                    entities.push(id.clone());
                    let mention = Mention {
                        entity_id: id.clone(),
                        original: d.original.clone(),
                        start: d.start,
                        end: d.end,
                        evidence: reference.clone(),
                        speaker: None,
                        negated: false,
                        speculative: true,
                    };
                    self.put_record(
                        &format!("{marker}:mention:{}", d.start),
                        "mention",
                        Some(case_id),
                        Some(&passage.report_id),
                        &mention,
                    )?;
                }
            } else {
                self.put_record(
                    &format!("{marker}:rejection:{}", d.start),
                    "label_rejection",
                    Some(case_id),
                    Some(&passage.report_id),
                    d,
                )?;
            }
        }
        entities.sort();
        entities.dedup();
        if entities.is_empty() {
            let mut e = normalize_entity(&passage.title, EntityType::Document)?;
            e.id = format!("document:{passage_id}");
            self.put_record(&e.id, "entity", Some(case_id), Some(&passage.report_id), &e)?;
            entities.push(e.id);
        }
        let observation = Observation {
            id: format!("{case_id}:{passage_id}:ingestion"),
            case_id: Some(case_id.into()),
            report_id: Some(passage.report_id.clone()),
            entity_id: entities[0].clone(),
            job_id: marker.clone(),
            provider: "report_ingestion".into(),
            provider_version: Some(crate::tna::PIPELINE_VERSION.to_string()),
            retrieved_at: chrono::Utc::now().to_rfc3339(),
            event_time: None,
            event_uncertainty: Some("Publication time is not event time".into()),
            basis: Basis::Inferred,
            statement: passage.text,
            attribution: format!("{} · {}", passage.title, passage.section),
            evidence: vec![reference.clone()],
            fields: serde_json::json!({"publication":passage.created_at,"report_version":passage.version}),
        };
        self.put_record(
            &observation.id,
            "observation",
            Some(case_id),
            Some(&passage.report_id),
            &observation,
        )?;
        for (n, a) in entities.iter().enumerate() {
            for b in entities.iter().skip(n + 1).take(16) {
                let relationship = Relationship {
                    from: a.clone(),
                    to: b.clone(),
                    kind: RelationshipType::TextCooccurrence,
                    basis: Basis::Inferred,
                    evidence: vec![reference.clone()],
                    uncertainty: "Same included passage; no ownership or verified identity implied"
                        .into(),
                };
                self.put_record(
                    &format!("{}:relationship:{a}:{b}", observation.id),
                    "relationship",
                    Some(case_id),
                    Some(&passage.report_id),
                    &relationship,
                )?;
            }
        }
        self.put_record(
            &marker,
            "case_ingestion",
            Some(case_id),
            Some(&passage.report_id),
            &marker,
        )
    }
    /// Additive report ingestion. Only saved case associations or explicitly included passages
    /// are used; unassigned historical reports remain unassigned.
    pub fn ingest_case_reports(&self, case_id: &str) -> Result<()> {
        self.ingest_case_reports_cancellable(case_id, &AtomicBool::new(false))
    }
    fn ingest_case_reports_cancellable(&self, case_id: &str, cancel: &AtomicBool) -> Result<()> {
        check_cancel(cancel)?;
        let included = self
            .records::<InvestigationScope>("investigation_scope", None, Some(case_id))?
            .into_iter()
            .flat_map(|s| s.included_evidence)
            .filter_map(|r| r.passage_id)
            .collect::<Vec<_>>();
        for passage_id in &included {
            check_cancel(cancel)?;
            self.ingest_included_passage(case_id, passage_id)?;
        }
        for report in self
            .list_reports()?
            .into_iter()
            .filter(|r| r.case_id.as_deref() == Some(case_id))
        {
            check_cancel(cancel)?;
            if let Ok(body) = std::fs::read_to_string(&report.path) {
                self.index_report(&report, &body)?;
            }
            let Some(body) = self.report_version(&report.id, None)? else {
                continue;
            };
            let version: i64 = self.conn.query_row(
                "SELECT MAX(version) FROM report_versions WHERE report_id=?1",
                [&report.id],
                |r| r.get(0),
            )?;
            let marker = format!(
                "{case_id}:ingested:{}:v{version}:{}",
                report.id,
                crate::tna::PIPELINE_VERSION
            );
            if !self
                .records::<String>("case_ingestion", Some(&report.id), Some(case_id))?
                .iter()
                .any(|s| s == &marker)
            {
                let corpus = crate::tna::TnaCorpus {
                    scope: crate::tna::TnaScope::Targeted {
                        report_id: report.id.clone(),
                        title: report.title.clone(),
                    },
                    docs: vec![crate::tna::CorpusDoc {
                        report_id: report.id.clone(),
                        title: report.title.clone(),
                        text: body,
                    }],
                };
                let snapshot =
                    crate::tna::build_snapshot_corrected(&corpus, &self.corrections(&report.id)?);
                self.persist_snapshot_evidence(&snapshot)?;
                let mut passages = std::collections::BTreeMap::<String, Vec<String>>::new();
                for d in &snapshot.decisions {
                    check_cancel(cancel)?;
                    let mut stmt=self.conn.prepare("SELECT id FROM report_passages WHERE report_id=?1 AND version=?2 AND start<=?3 AND end>?3 LIMIT 1")?;
                    let ids = stmt
                        .query_map(rusqlite::params![report.id, version, d.start], |r| {
                            r.get::<_, String>(0)
                        })?
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    let Some(passage_id) = ids.first() else {
                        continue;
                    };
                    if report.case_id.as_deref() != Some(case_id) && !included.contains(passage_id)
                    {
                        continue;
                    }
                    if let (Some(id), Some(label)) = (&d.canonical_id, &d.label) {
                        let kind = match d.kind {
                            crate::tna::TnaNodeKind::Domain => EntityType::Domain,
                            crate::tna::TnaNodeKind::Ip => EntityType::Ip,
                            crate::tna::TnaNodeKind::Email => EntityType::Email,
                            crate::tna::TnaNodeKind::Handle => EntityType::Account,
                            crate::tna::TnaNodeKind::Person => EntityType::Person,
                            crate::tna::TnaNodeKind::Org => EntityType::Organization,
                            _ => EntityType::Theme,
                        };
                        if let Ok(mut entity) = normalize_entity(label, kind) {
                            entity.id = id.clone();
                            self.put_record(
                                id,
                                "entity",
                                Some(case_id),
                                Some(&report.id),
                                &entity,
                            )?;
                            passages
                                .entry(passage_id.clone())
                                .or_default()
                                .push(id.clone());
                        }
                    } else {
                        self.put_record(
                            &format!("{marker}:rejected:{}", d.start),
                            "label_rejection",
                            Some(case_id),
                            Some(&report.id),
                            d,
                        )?;
                    }
                }
                for (passage_id, mut ids) in passages {
                    check_cancel(cancel)?;
                    ids.sort();
                    ids.dedup();
                    let Some(passage) = self.passage(&passage_id)? else {
                        continue;
                    };
                    let reference = EvidenceRef {
                        report_id: Some(report.id.clone()),
                        passage_id: Some(passage_id.clone()),
                        ..Default::default()
                    };
                    let observation = Observation {
                        id: format!("{case_id}:{passage_id}:ingestion"),
                        case_id: Some(case_id.into()),
                        report_id: Some(report.id.clone()),
                        entity_id: ids[0].clone(),
                        job_id: marker.clone(),
                        provider: "report_ingestion".into(),
                        provider_version: Some(crate::tna::PIPELINE_VERSION.to_string()),
                        retrieved_at: chrono::Utc::now().to_rfc3339(),
                        event_time: None,
                        event_uncertainty: Some(
                            "Report date is publication time, not event time".into(),
                        ),
                        basis: Basis::Inferred,
                        statement: passage.text.clone(),
                        attribution: format!("{} · {}", report.title, passage.section),
                        evidence: vec![reference.clone()],
                        fields: serde_json::json!({"report_version":version,"publication":passage.created_at}),
                    };
                    self.put_record(
                        &observation.id,
                        "observation",
                        Some(case_id),
                        Some(&report.id),
                        &observation,
                    )?;
                    for (n, a) in ids.iter().enumerate() {
                        for b in ids.iter().skip(n + 1).take(16) {
                            let relationship=Relationship{from:a.clone(),to:b.clone(),kind:RelationshipType::TextCooccurrence,basis:Basis::Inferred,evidence:vec![reference.clone()],uncertainty:"Mentioned in the same passage; does not establish ownership, identity, or an observed provider relationship".into()};
                            self.put_record(
                                &format!("{}:relationship:{a}:{b}", observation.id),
                                "relationship",
                                Some(case_id),
                                Some(&report.id),
                                &relationship,
                            )?;
                        }
                    }
                }
                self.put_record(
                    &marker,
                    "case_ingestion",
                    Some(case_id),
                    Some(&report.id),
                    &marker,
                )?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntityCorrection {
    pub id: String,
    pub entity_id: String,
    pub original: String,
    pub replacement: Option<String>,
    pub reason: String,
    pub reverses: Option<String>,
}
impl Store {
    pub fn correct_case_entity(&self, case_id: &str, correction: &EntityCorrection) -> Result<()> {
        if correction.reason.trim().is_empty() {
            bail!("A correction reason is required");
        }
        if let Some(id) = &correction.reverses {
            if !self
                .records::<EntityCorrection>("entity_correction", None, Some(case_id))?
                .iter()
                .any(|c| &c.id == id)
            {
                bail!("Correction outside this case");
            }
        } else {
            let entity = self
                .records_scoped::<Entity>("entity", &EvidenceScope::Case(case_id.into()))?
                .into_iter()
                .find(|e| e.id == correction.entity_id)
                .ok_or_else(|| anyhow::anyhow!("Entity outside this case"))?;
            if entity.label != correction.original {
                bail!("Original entity label no longer matches");
            }
            if let Some(label) = &correction.replacement {
                normalize_entity(label, entity.kind)?;
            }
        }
        self.put_record(
            &correction.id,
            "entity_correction",
            Some(case_id),
            None,
            correction,
        )
    }
    pub fn save_case_identity(&self, case_id: &str, decision: &IdentityDecision) -> Result<()> {
        if decision.reason.trim().is_empty() {
            bail!("An identity decision reason is required");
        }
        if let Some(id) = &decision.reverses {
            if !self
                .records::<IdentityDecision>("identity_resolution", None, Some(case_id))?
                .iter()
                .any(|d| &d.id == id)
            {
                bail!("Identity decision outside this case");
            }
        } else {
            let entities =
                self.records_scoped::<Entity>("entity", &EvidenceScope::Case(case_id.into()))?;
            if decision.entities.len() < 2
                || decision
                    .canonical_entity
                    .as_ref()
                    .is_none_or(|id| !decision.entities.contains(id))
                || decision
                    .entities
                    .iter()
                    .any(|id| !entities.iter().any(|e| &e.id == id))
            {
                bail!("Choose a canonical entity and at least two entities in this case");
            }
            if decision.evidence.is_empty()
                || decision.evidence.iter().any(|r| {
                    self.resolve_evidence(r, &EvidenceScope::Case(case_id.into()))
                        .is_err()
                })
            {
                bail!("Identity decisions need source evidence within this case");
            }
        }
        self.put_record(
            &decision.id,
            "identity_resolution",
            Some(case_id),
            None,
            decision,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn seed(store: &Store, case: &str, id: &str) -> Entity {
        let mut e = normalize_entity(id, EntityType::Domain).unwrap();
        e.id = format!("domain:{id}");
        store
            .put_record(&e.id, "entity", Some(case), None, &e)
            .unwrap();
        e
    }
    fn finding(store: &Store, case: &str, id: &str, entity: &str, url: &str) -> Observation {
        let artifact = Artifact {
            id: format!("{id}:artifact"),
            source_url: Some(url.into()),
            retrieved_at: "2026-09-27".into(),
            media_type: "text/plain".into(),
            body: format!("Original source: {entity} resolves to 8.8.8.8"),
        };
        store
            .put_record(&artifact.id, "artifact", Some(case), None, &artifact)
            .unwrap();
        let o = Observation {
            id: id.into(),
            case_id: Some(case.into()),
            report_id: None,
            entity_id: entity.into(),
            job_id: "job-fixture".into(),
            provider: "doh".into(),
            provider_version: None,
            retrieved_at: "2026-09-27".into(),
            event_time: None,
            event_uncertainty: None,
            basis: Basis::Observed,
            statement: format!("{entity} resolves to 8.8.8.8"),
            attribution: "DNS fixture".into(),
            evidence: vec![EvidenceRef {
                artifact_id: Some(artifact.id),
                source_url: Some(url.into()),
                ..Default::default()
            }],
            fields: serde_json::json!({}),
        };
        store
            .put_record(id, "observation", Some(case), None, &o)
            .unwrap();
        o
    }
    #[test]
    fn cancelled_projection_leaves_evidence_readable() {
        let store = Store::memory().unwrap();
        let c = store.create_case("A").unwrap();
        seed(&store, &c.id, "harbor.example");
        assert!(store
            .case_projection_cancellable(&c.id, &AtomicBool::new(true))
            .is_err());
        assert_eq!(store.case_projection(&c.id).unwrap().entities.len(), 1);
    }
    #[test]
    fn case_without_report_survives_restart_and_has_empty_network() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("case.sqlite");
        let store = Store::open(&db).unwrap();
        let c = store.create_case("Investigate Harbor").unwrap();
        assert!(store.case_projection(&c.id).unwrap().entities.is_empty());
        drop(store);
        let store = Store::open(&db).unwrap();
        assert_eq!(store.list_cases().unwrap()[0].id, c.id);
        assert!(store.list_reports().unwrap().is_empty());
        assert!(store.case_projection(&c.id).unwrap().links.is_empty());
    }
    #[test]
    fn review_changes_network_and_candidate_identity_never_becomes_observed() {
        let store = Store::memory().unwrap();
        let c = store.create_case("Harbor").unwrap();
        let e = seed(&store, &c.id, "harbor.example");
        let o = finding(&store, &c.id, "o1", &e.id, "https://source.example/dns");
        let relationship = Relationship {
            from: e.id,
            to: "ip:8.8.8.8".into(),
            kind: RelationshipType::DomainResolvesToIp,
            basis: Basis::Observed,
            evidence: o.evidence.clone(),
            uncertainty: "Resolution does not establish ownership".into(),
        };
        store
            .put_record("r1", "relationship", Some(&c.id), None, &relationship)
            .unwrap();
        let pending = store.case_projection(&c.id).unwrap();
        assert!(pending.links[0].candidate);
        assert!(pending.network_snapshot("Harbor").edges.is_empty());
        store
            .review_finding("o1", ReviewDecision::Accept, "Verified supporting artifact")
            .unwrap();
        let accepted = store.case_projection(&c.id).unwrap();
        assert!(accepted.links[0].reviewed);
        assert!(!accepted.links[0].candidate);
        assert_eq!(accepted.network_snapshot("Harbor").edges.len(), 1);
        let mut candidate = relationship.clone();
        candidate.kind = RelationshipType::CandidateIdentityAssociation;
        candidate.basis = Basis::Inferred;
        store
            .put_record("r2", "relationship", Some(&c.id), None, &candidate)
            .unwrap();
        let p = store.case_projection(&c.id).unwrap();
        assert!(
            p.links
                .iter()
                .find(|l| l.relationship.kind == RelationshipType::CandidateIdentityAssociation)
                .unwrap()
                .candidate
        );
        assert_eq!(p.network_snapshot("Harbor").edges.len(), 1);
        store
            .review_finding("o1", ReviewDecision::Reject, "Misattributed source")
            .unwrap();
        assert!(store.case_projection(&c.id).unwrap().links.is_empty());
        store
            .review_finding("o1", ReviewDecision::Defer, "Needs corroboration")
            .unwrap();
        assert!(store.case_projection(&c.id).unwrap().links.is_empty());
        assert_eq!(
            store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM finding_decisions WHERE observation_id='o1'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            3
        );
    }
    #[test]
    fn reviewed_retrieval_and_artifacts_obey_case_and_exact_identifier_boundaries() {
        let store = Store::memory().unwrap();
        let a = store.create_case("A").unwrap();
        let b = store.create_case("B").unwrap();
        let ea = seed(&store, &a.id, "harbor.example");
        seed(&store, &b.id, "harbor.example");
        let oa = finding(&store, &a.id, "oa", &ea.id, "https://source.example/a");
        finding(&store, &b.id, "ob", &ea.id, "https://source.example/b");
        assert!(store
            .retrieve_reviewed("harbor.example", &EvidenceScope::Case(a.id.clone()), 8)
            .unwrap()
            .is_empty());
        store
            .review_finding("oa", ReviewDecision::Accept, "Supporting source checked")
            .unwrap();
        assert_eq!(
            store
                .retrieve_reviewed("harbor.example", &EvidenceScope::Case(a.id.clone()), 8)
                .unwrap()
                .len(),
            1
        );
        assert!(store
            .retrieve_reviewed("harbor.example", &EvidenceScope::Case(b.id.clone()), 8)
            .unwrap()
            .is_empty());
        assert!(store
            .retrieve_reviewed("other.example", &EvidenceScope::Case(a.id.clone()), 8)
            .unwrap()
            .is_empty());
        assert!(store
            .resolve_evidence(&oa.evidence[0], &EvidenceScope::Case(b.id.clone()))
            .is_err());
        assert_eq!(
            store
                .records_scoped::<Entity>("entity", &EvidenceScope::Case(b.id))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .records_scoped::<Entity>("entity", &EvidenceScope::Case(a.id))
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    fn malformed_labels_and_reversible_case_corrections() {
        for label in [
            "report.md",
            "not.a.real.invalidsuffix",
            "--broken.com",
            "bad..com",
            "bad\n.com",
        ] {
            assert!(
                normalize_entity(label, EntityType::Domain).is_err(),
                "{label}"
            );
        }
        let store = Store::memory().unwrap();
        let c = store.create_case("A").unwrap();
        let e = seed(&store, &c.id, "harbor.example");
        let correction = EntityCorrection {
            id: "suppress".into(),
            entity_id: e.id.clone(),
            original: e.label.clone(),
            replacement: None,
            reason: "Wrong source identity".into(),
            reverses: None,
        };
        store.correct_case_entity(&c.id, &correction).unwrap();
        assert!(store.case_projection(&c.id).unwrap().entities.is_empty());
        store
            .correct_case_entity(
                &c.id,
                &EntityCorrection {
                    id: "undo".into(),
                    entity_id: e.id,
                    original: e.label,
                    replacement: None,
                    reason: "Reconsidered".into(),
                    reverses: Some("suppress".into()),
                },
            )
            .unwrap();
        assert_eq!(store.case_projection(&c.id).unwrap().entities.len(), 1);
    }
    #[test]
    fn report_ingestion_is_additive_scoped_and_preserves_historic_citations() {
        let store = Store::memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let c = store.create_case("Harbor").unwrap();
        let mut report = crate::report::write_report(
            dir.path(),
            "Harbor",
            Some(&c.id),
            "## Evidence\nHarbor Corporation uses harbor.example and contact@harbor.example.\n",
        )
        .unwrap();
        store.add_report(&report).unwrap();
        let unassigned = crate::report::write_report(
            dir.path(),
            "Unassigned",
            None,
            "## Evidence\nUnassigned Corporation uses unrelated.example.\n",
        )
        .unwrap();
        store.add_report(&unassigned).unwrap();
        let original = store
            .retrieve_passages("harbor.example", &EvidenceScope::Case(c.id.clone()), 8)
            .unwrap()[0]
            .clone();
        let before = store.case_projection(&c.id).unwrap();
        assert!(!before.entities.is_empty());
        assert!(!before.links.is_empty());
        assert!(before
            .links
            .iter()
            .all(|l| l.candidate && l.relationship.kind == RelationshipType::TextCooccurrence));
        assert!(before
            .entities
            .iter()
            .all(|e| !e.label.contains("unrelated")));
        let count = before.findings.len();
        assert_eq!(store.case_projection(&c.id).unwrap().findings.len(), count);
        store
            .index_report(
                &report,
                "## Evidence\nHarbor Corporation uses harbor.example. New text.\n",
            )
            .unwrap();
        assert_eq!(
            store.passage(&original.id).unwrap().unwrap().text,
            original.text
        );
        assert_eq!(
            store
                .list_reports()
                .unwrap()
                .iter()
                .find(|r| r.id == unassigned.id)
                .unwrap()
                .case_id,
            None
        );
        report.case_id = None; // metadata has not been mutated by ingestion
        assert!(store
            .list_reports()
            .unwrap()
            .iter()
            .find(|r| r.id == report.id)
            .unwrap()
            .case_id
            .is_some());
    }
    #[test]
    fn report_drafting_requires_explicit_selected_accepted_case_evidence() {
        let store = Store::memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let c = store.create_case("Harbor").unwrap();
        let e = seed(&store, &c.id, "harbor.example");
        let o = finding(&store, &c.id, "o1", &e.id, "https://source.example/dns");
        assert!(store
            .draft_case_report(&c.id, "final", &[o.id.clone()], dir.path())
            .is_err());
        store
            .review_finding(&o.id, ReviewDecision::Accept, "Source checked")
            .unwrap();
        assert!(store
            .draft_case_report(&c.id, "final", &["outside-scope".into()], dir.path())
            .is_err());
        let r = store
            .draft_case_report(&c.id, "final", &[o.id.clone()], dir.path())
            .unwrap();
        let text = store.report_version(&r.id, None).unwrap().unwrap();
        assert!(
            text.contains("[o1]")
                && text.contains("undated")
                && text.contains("retrieved 2026-09-27")
                && text.contains("Uncertainties")
        );
        assert_eq!(
            store
                .findings_scoped(&EvidenceScope::Case(c.id))
                .unwrap()
                .iter()
                .find(|f| f.observation.id == o.id)
                .unwrap()
                .observation
                .report_id,
            None
        );
    }
    #[test]
    fn case_identity_decisions_preserve_original_entities_and_reverse() {
        let store = Store::memory().unwrap();
        let c = store.create_case("A").unwrap();
        let a = seed(&store, &c.id, "harbor.example");
        let b = seed(&store, &c.id, "harbor.com");
        let o = finding(&store, &c.id, "o", &a.id, "https://source.example/identity");
        let decision = IdentityDecision {
            id: "merge".into(),
            entities: vec![a.id.clone(), b.id.clone()],
            canonical_entity: Some(a.id.clone()),
            reason: "Analyst corroborated sources".into(),
            evidence: o.evidence,
            reverses: None,
        };
        store.save_case_identity(&c.id, &decision).unwrap();
        assert_eq!(store.case_projection(&c.id).unwrap().entities.len(), 1);
        assert_eq!(
            store
                .records_scoped::<Entity>("entity", &EvidenceScope::Case(c.id.clone()))
                .unwrap()
                .len(),
            2
        );
        store
            .save_case_identity(
                &c.id,
                &IdentityDecision {
                    id: "unmerge".into(),
                    entities: vec![],
                    canonical_entity: None,
                    reason: "Reconsidered".into(),
                    evidence: vec![],
                    reverses: Some("merge".into()),
                },
            )
            .unwrap();
        assert_eq!(store.case_projection(&c.id).unwrap().entities.len(), 2);
    }
}

#[cfg(test)]
mod inclusion_tests {
    use super::*;
    #[test]
    fn included_historic_passage_stays_in_scope_without_assigning_its_report() {
        let store = Store::memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let c = store.create_case("Included evidence").unwrap();
        let report=crate::report::write_report(dir.path(),"Unassigned",None,"## Evidence\nHarbor Corporation uses harbor.example.\n\n## Findings\nOther Corporation uses other.example.\n").unwrap();
        store.add_report(&report).unwrap();
        let hit = store
            .retrieve_passages(
                "harbor.example",
                &EvidenceScope::Report(report.id.clone()),
                8,
            )
            .unwrap()[0]
            .clone();
        let scope = InvestigationScope {
            question: "Harbor".into(),
            included_evidence: vec![EvidenceRef {
                report_id: Some(report.id.clone()),
                passage_id: Some(hit.id.clone()),
                ..Default::default()
            }],
            allowed_actions: vec!["search".into()],
            ..Default::default()
        };
        store
            .put_record("scope", "investigation_scope", Some(&c.id), None, &scope)
            .unwrap();
        store
            .index_report(
                &report,
                "## Evidence\nHarbor Corporation replaced the old assertion.\n",
            )
            .unwrap();
        let p = store.case_projection(&c.id).unwrap();
        assert_eq!(p.findings.len(), 1);
        assert_eq!(p.findings[0].observation.statement, hit.text);
        assert!(p.entities.iter().any(|e| e.label == "harbor.example"));
        assert!(p.entities.iter().all(|e| e.label != "other.example"));
        assert!(store
            .resolve_evidence(
                &scope.included_evidence[0],
                &EvidenceScope::Case(c.id.clone())
            )
            .unwrap()
            .contains("harbor.example"));
        assert_eq!(store.list_reports().unwrap()[0].case_id, None);
        let timeline = store.timeline_scoped(&EvidenceScope::Case(c.id)).unwrap();
        assert_eq!(timeline[0].event_time, None);
        assert_eq!(
            timeline[0].published_at.as_deref(),
            Some(report.created_at.as_str())
        );
        assert!(timeline[0].retrieved_at.contains('T'));
    }
}
