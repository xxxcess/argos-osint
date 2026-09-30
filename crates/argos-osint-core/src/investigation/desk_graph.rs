//! Typed, case-owned gaps and the operations Desk read model. Never invokes a provider.
use super::*;
use crate::research::{eligible_action, JobState, ResearchConfig};
use rusqlite::params;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapKind {
    Uncollected,
    CollectedAbsent,
    Conflicting,
    Candidate,
}
impl GapKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Uncollected => "uncollected",
            Self::CollectedAbsent => "collected_absent",
            Self::Conflicting => "conflicting",
            Self::Candidate => "candidate",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gap {
    pub id: String,
    pub case_id: String,
    pub entity_id: String,
    pub to: Option<String>,
    pub kind: GapKind,
    pub action: Option<String>,
    pub input: String,
    pub reason: String,
    pub observation_ids: Vec<String>,
    pub job_ids: Vec<String>,
    pub open: bool,
    pub updated_at: String,
}
impl Store {
    pub fn gaps(&self, case_id: &str, include_closed: bool) -> Result<Vec<Gap>> {
        let mut stmt = self
            .conn
            .prepare("SELECT body FROM case_gaps WHERE case_id=?1 ORDER BY id")?;
        let bodies = stmt
            .query_map([case_id], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        bodies
            .into_iter()
            .map(|b| Ok(serde_json::from_str::<Gap>(&b)?))
            .filter(|r| match r {
                Ok(g) => include_closed || g.open,
                Err(_) => true,
            })
            .collect()
    }
    pub(crate) fn record_job_gap(&self, job: &ResearchJob) -> Result<()> {
        let Some(case_id) = job.input.case_id.as_ref() else {
            return Ok(());
        };
        if job.state != JobState::Completed {
            return Ok(());
        }
        let now = job
            .finished_at
            .clone()
            .unwrap_or_else(|| job.created_at.clone());
        for mut gap in self.gaps(case_id, true)?.into_iter().filter(|g| {
            g.open
                && g.action.as_ref() == Some(&job.provider)
                && g.input == job.input.label
                && matches!(g.kind, GapKind::Uncollected | GapKind::CollectedAbsent)
        }) {
            gap.open = false;
            gap.job_ids.push(job.id.clone());
            gap.updated_at = now.clone();
            self.conn.execute(
                "UPDATE case_gaps SET body=?2 WHERE id=?1",
                params![gap.id, serde_json::to_string(&gap)?],
            )?;
        }
        if job.hits.is_empty() {
            let kind = GapKind::CollectedAbsent;
            let gap = Gap {
                id: serde_json::to_string(&(
                    case_id,
                    &job.input.entity_id,
                    &job.provider,
                    &job.input.label,
                    kind.label(),
                ))?,
                case_id: case_id.clone(),
                entity_id: job.input.entity_id.clone(),
                to: None,
                kind,
                action: Some(job.provider.clone()),
                input: job.input.label.clone(),
                reason: format!(
                    "collected_absent {} · not a real-world negative finding",
                    job.provider
                ),
                observation_ids: vec![],
                job_ids: vec![job.id.clone()],
                open: true,
                updated_at: now,
            };
            self.conn.execute("INSERT INTO case_gaps VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET body=excluded.body",params![gap.id,case_id,serde_json::to_string(&gap)?])?;
        }
        Ok(())
    }
    pub(super) fn reconcile_gaps(
        &self,
        case_id: &str,
        data: &CaseProjection,
        configs: &std::collections::BTreeMap<String, ResearchConfig>,
    ) -> Result<Vec<Gap>> {
        let old = self.gaps(case_id, true)?;
        let mut desired = Vec::new();
        let mut inputs = data
            .entities
            .iter()
            .map(|e| (e.id.clone(), e.label.clone()))
            .collect::<Vec<_>>();
        if inputs.is_empty() {
            if let Some(scope) = &data.scope {
                inputs.push((scope.question.clone(), scope.question.clone()));
            }
        }
        for (entity_id, input) in inputs {
            let mut actions = data
                .scope
                .iter()
                .flat_map(|s| s.allowed_actions.clone())
                .chain(
                    data.jobs
                        .iter()
                        .filter(|j| j.input.label == input)
                        .map(|j| j.provider.clone()),
                )
                .collect::<Vec<_>>();
            actions.sort();
            actions.dedup();
            for action in &actions {
                let jobs = data
                    .jobs
                    .iter()
                    .filter(|j| j.provider == *action && j.input.label == input)
                    .collect::<Vec<_>>();
                let absent = jobs
                    .iter()
                    .find(|j| j.state == JobState::Completed)
                    .is_some_and(|j| !data.findings.iter().any(|f| f.observation.job_id == j.id));
                if jobs.is_empty()
                    && configs
                        .get(action)
                        .is_none_or(|c| eligible_action(action, &input, c).is_err())
                {
                    continue;
                }
                let kind = if jobs.is_empty() {
                    Some(GapKind::Uncollected)
                } else if absent {
                    Some(GapKind::CollectedAbsent)
                } else {
                    None
                };
                if let Some(kind) = kind {
                    desired.push(Gap {
                        id: serde_json::to_string(&(
                            case_id,
                            &entity_id,
                            action,
                            &input,
                            kind.label(),
                        ))?,
                        case_id: case_id.into(),
                        entity_id: entity_id.clone(),
                        to: None,
                        kind,
                        action: Some(action.clone()),
                        input: input.clone(),
                        reason: format!(
                            "{} {action}{}{}",
                            kind.label(),
                            if action == "domain" {
                                " (DNS / RDAP / certificates / Wayback)"
                            } else {
                                ""
                            },
                            if kind == GapKind::CollectedAbsent {
                                " · not a real-world negative finding"
                            } else {
                                ""
                            }
                        ),
                        observation_ids: vec![],
                        job_ids: jobs.iter().map(|j| j.id.clone()).collect(),
                        open: true,
                        updated_at: String::new(),
                    });
                }
            }
        }
        for f in &data.findings {
            if !data
                .entities
                .iter()
                .any(|e| e.id == f.observation.entity_id)
                || !f.category.to_lowercase().contains("conflict")
            {
                continue;
            }
            let o = &f.observation;
            let mut ids = vec![o.id.clone()];
            if let Some(id) = o.fields.get("conflicts_with").and_then(|v| v.as_str()) {
                ids.push(id.into());
            }
            desired.push(Gap {
                id: format!("{case_id}:conflict:{}", o.id),
                case_id: case_id.into(),
                entity_id: o.entity_id.clone(),
                to: None,
                kind: GapKind::Conflicting,
                action: None,
                input: String::new(),
                reason: "Conflicting sources · review both observations".into(),
                observation_ids: ids,
                job_ids: vec![o.job_id.clone()],
                open: true,
                updated_at: String::new(),
            });
        }
        for l in &data.links {
            if !l.candidate {
                continue;
            }
            let r = &l.relationship;
            desired.push(Gap {id:serde_json::to_string(&(case_id,&r.from,&r.to,r.kind.label(),r.basis.label()))?,case_id:case_id.into(),entity_id:r.from.clone(),to:Some(r.to.clone()),kind:GapKind::Candidate,action:None,input:String::new(),reason:"Candidate identity / co-occurrence · accepting a username match does not verify identity".into(),observation_ids:l.observations.clone(),job_ids:vec![],open:true,updated_at:String::new()});
        }
        // Stable timestamps prevent rank flapping. Closed records retain their source history.
        let now = chrono::Utc::now().to_rfc3339();
        for gap in &mut desired {
            gap.updated_at = old
                .iter()
                .find(|g| g.id == gap.id)
                .map(|g| g.updated_at.clone())
                .unwrap_or_else(|| now.clone());
            if old
                .iter()
                .find(|g| g.id == gap.id)
                .is_some_and(|g| g != gap)
            {
                gap.updated_at = now.clone();
            }
            self.conn.execute("INSERT INTO case_gaps VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET body=excluded.body",params![gap.id,case_id,serde_json::to_string(gap)?])?;
        }
        for mut gap in old
            .into_iter()
            .filter(|g| g.open && !desired.iter().any(|d| d.id == g.id))
        {
            gap.open = false;
            gap.updated_at = now.clone();
            self.conn.execute(
                "UPDATE case_gaps SET body=?2 WHERE id=?1",
                params![gap.id, serde_json::to_string(&gap)?],
            )?;
        }
        desired.sort_by_key(|g| (g.kind.label(), g.action.clone(), g.id.clone()));
        Ok(desired)
    }
}
impl CaseProjection {
    pub fn pending_for(&self, id: &str) -> usize {
        self.findings
            .iter()
            .filter(|f| {
                f.observation.entity_id == id
                    && (f.decision.is_none() || f.decision == Some(ReviewDecision::Defer))
            })
            .count()
    }
    pub fn why_now(&self, id: &str) -> (u8, String) {
        let pending = self.pending_for(id);
        if pending > 0 {
            return (0, format!("review {pending} findings"));
        }
        for (rank, kind) in [
            (1, GapKind::Uncollected),
            (2, GapKind::Conflicting),
            (3, GapKind::Candidate),
            (4, GapKind::CollectedAbsent),
        ] {
            if let Some(g) = self.gaps.iter().find(|g| {
                g.open && g.kind == kind && (g.entity_id == id || g.to.as_deref() == Some(id))
            }) {
                return (rank, g.reason.clone());
            }
        }
        (5, "accepted evidence · draft when chosen".into())
    }
    pub fn accepted_degree(&self, id: &str) -> usize {
        self.links
            .iter()
            .filter(|l| {
                l.reviewed && !l.candidate && (l.relationship.from == id || l.relationship.to == id)
            })
            .count()
    }
    pub fn rank_leads(&mut self) {
        let ranks = self
            .entities
            .iter()
            .map(|e| {
                (
                    e.id.clone(),
                    (
                        self.why_now(&e.id).0,
                        std::cmp::Reverse(self.accepted_degree(&e.id)),
                        e.label.clone(),
                        e.id.clone(),
                    ),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();
        self.entities.sort_by_key(|e| ranks[&e.id].clone());
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NextWorkKind {
    Review,
    Enrich,
    Gap,
    Product,
}
#[derive(Clone, Debug)]
pub struct NextWorkCard {
    pub kind: NextWorkKind,
    pub case_id: String,
    pub lead_id: Option<String>,
    pub gap_id: Option<String>,
    pub observation_ids: Vec<String>,
    pub action: Option<String>,
    pub reason: String,
    pub rank_key: (u8, String),
}
#[derive(Clone, Debug)]
pub struct QueueRow {
    pub case_id: String,
    pub pending_review: usize,
    pub running_jobs: usize,
    pub gap_counts: [usize; 4],
    pub draft_ready: bool,
}
#[derive(Clone, Debug, Default)]
pub struct DeskProjection {
    pub cards: Vec<NextWorkCard>,
    pub queue: Vec<QueueRow>,
    pub questions: Vec<Gap>,
}
impl DeskProjection {
    pub fn build(cases: &[(String, CaseProjection)]) -> Self {
        let mut desk = Self::default();
        for (case_id, data) in cases {
            let mut counts = [0; 4];
            for g in &data.gaps {
                if g.open {
                    counts[match g.kind {
                        GapKind::Uncollected => 0,
                        GapKind::CollectedAbsent => 1,
                        GapKind::Conflicting => 2,
                        GapKind::Candidate => 3,
                    }] += 1;
                    desk.questions.push(g.clone());
                }
            }
            desk.queue.push(QueueRow {
                case_id: case_id.clone(),
                pending_review: data
                    .findings
                    .iter()
                    .filter(|f| f.decision.is_none() || f.decision == Some(ReviewDecision::Defer))
                    .count(),
                running_jobs: data
                    .jobs
                    .iter()
                    .filter(|j| matches!(j.state, JobState::Queued | JobState::Running))
                    .count(),
                gap_counts: counts,
                draft_ready: data.findings.iter().any(|f| {
                    f.decision == Some(ReviewDecision::Accept)
                        && !f.category.starts_with("Candidate")
                }),
            });
            for e in &data.entities {
                let (rank, reason) = data.why_now(&e.id);
                let gap = data.gaps.iter().find(|g| {
                    g.open
                        && (g.entity_id == e.id || g.to.as_ref() == Some(&e.id))
                        && match rank {
                            1 => g.kind == GapKind::Uncollected,
                            2 => g.kind == GapKind::Conflicting,
                            3 => g.kind == GapKind::Candidate,
                            4 => g.kind == GapKind::CollectedAbsent,
                            _ => false,
                        }
                });
                let ids = data
                    .findings
                    .iter()
                    .filter(|f| {
                        f.observation.entity_id == e.id
                            && if rank == 0 {
                                f.decision.is_none() || f.decision == Some(ReviewDecision::Defer)
                            } else {
                                f.decision == Some(ReviewDecision::Accept)
                                    && !f.category.starts_with("Candidate")
                            }
                    })
                    .map(|f| f.observation.id.clone())
                    .collect::<Vec<_>>();
                if (rank == 5 || rank == 4) && ids.is_empty() && gap.is_none() {
                    continue;
                }
                desk.cards.push(NextWorkCard {
                    kind: match rank {
                        0 => NextWorkKind::Review,
                        1..=4 => NextWorkKind::Gap,
                        _ => NextWorkKind::Product,
                    },
                    case_id: case_id.clone(),
                    lead_id: Some(e.id.clone()),
                    gap_id: gap.map(|g| g.id.clone()),
                    observation_ids: ids,
                    action: gap
                        .filter(|g| g.kind == GapKind::Uncollected)
                        .and_then(|g| g.action.clone()),
                    reason,
                    rank_key: (rank, gap.map(|g| g.updated_at.clone()).unwrap_or_default()),
                });
            }
            if data.entities.is_empty() {
                if let Some(g) = data.gaps.iter().find(|g| g.open) {
                    desk.cards.push(NextWorkCard {
                        kind: NextWorkKind::Gap,
                        case_id: case_id.clone(),
                        lead_id: None,
                        gap_id: Some(g.id.clone()),
                        observation_ids: g.observation_ids.clone(),
                        action: if g.kind == GapKind::Uncollected {
                            g.action.clone()
                        } else {
                            None
                        },
                        reason: g.reason.clone(),
                        rank_key: (
                            if g.kind == GapKind::Uncollected { 1 } else { 4 },
                            g.updated_at.clone(),
                        ),
                    });
                }
            }
        }
        desk.cards
            .sort_by_key(|c| (c.rank_key.clone(), c.case_id.clone(), c.lead_id.clone()));
        desk.cards.truncate(3);
        desk.questions
            .sort_by_key(|g| (g.kind.label(), g.case_id.clone(), g.id.clone()));
        desk
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::{ResearchInput, ResearchJob, Stage};
    fn scoped_domain(store: &Store, title: &str) -> (String, Entity) {
        let case = store.create_case(title).unwrap();
        let entity = normalize_entity("harbor.example", EntityType::Domain).unwrap();
        store
            .put_record(&entity.id, "entity", Some(&case.id), None, &entity)
            .unwrap();
        let scope = InvestigationScope {
            question: entity.label.clone(),
            allowed_actions: vec!["domain".into()],
            ..Default::default()
        };
        store
            .put_record("scope", "investigation_scope", Some(&case.id), None, &scope)
            .unwrap();
        (case.id, entity)
    }
    fn job(case_id: &str, e: &Entity, state: JobState) -> ResearchJob {
        ResearchJob {
            provider_version: None,
            metadata: vec![],
            id: "job".into(),
            run_id: "run".into(),
            input: ResearchInput {
                case_id: Some(case_id.into()),
                report_id: None,
                entity_id: e.id.clone(),
                label: e.label.clone(),
                action: "domain".into(),
                depth: 0,
            },
            provider: "domain".into(),
            stage: Stage::DomainEmail,
            state,
            progress: "done".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            finished_at: Some(chrono::Utc::now().to_rfc3339()),
            elapsed_ms: 0,
            error: None,
            hits: vec![],
        }
    }
    #[test]
    fn persisted_gaps_transition_without_replaying_and_clear_with_case() {
        let store = Store::memory().unwrap();
        let (case, e) = scoped_domain(&store, "Harbor");
        let initial = store.case_projection(&case).unwrap();
        assert_eq!(initial.gaps.len(), 1);
        assert_eq!(initial.gaps[0].kind, GapKind::Uncollected);
        assert_eq!(
            initial.gaps[0].updated_at,
            store.case_projection(&case).unwrap().gaps[0].updated_at
        );
        assert!(store.jobs().unwrap().is_empty());
        let completed = job(&case, &e, JobState::Completed);
        store.save_job("exact-input", &completed).unwrap();
        store.record_job_gap(&completed).unwrap();
        assert!(store
            .gaps(&case, false)
            .unwrap()
            .iter()
            .any(|g| g.kind == GapKind::CollectedAbsent));
        let after = store.case_projection(&case).unwrap();
        assert_eq!(after.gaps[0].kind, GapKind::CollectedAbsent);
        assert!(after.gaps[0]
            .reason
            .contains("not a real-world negative finding"));
        assert!(store
            .gaps(&case, true)
            .unwrap()
            .iter()
            .any(|g| g.kind == GapKind::Uncollected && !g.open));
        assert_eq!(store.jobs().unwrap().len(), 1);
        let observation = Observation {
            id: "o1".into(),
            case_id: Some(case.clone()),
            report_id: None,
            entity_id: e.id,
            job_id: completed.id,
            provider: "rdap".into(),
            provider_version: None,
            retrieved_at: "2026-09-28".into(),
            event_time: None,
            event_uncertainty: None,
            basis: Basis::Observed,
            statement: "Registered organization".into(),
            attribution: "RDAP".into(),
            evidence: vec![],
            fields: serde_json::json!({}),
        };
        store
            .put_record(
                &observation.id,
                "observation",
                Some(&case),
                None,
                &observation,
            )
            .unwrap();
        assert!(store.case_projection(&case).unwrap().gaps.is_empty());
        let plan = store.plan_case_data(&case, false).unwrap();
        store.apply_case_data_plan(&plan).unwrap();
        assert!(store.gaps(&case, true).unwrap().is_empty());
        assert_eq!(store.list_cases().unwrap().len(), 1);
    }
    #[test]
    fn why_now_prioritizes_review_and_scopes_desk_without_collection() {
        let store = Store::memory().unwrap();
        let (case, e) = scoped_domain(&store, "Harbor");
        let accepted = normalize_entity("settled.example", EntityType::Domain).unwrap();
        store
            .put_record(&accepted.id, "entity", Some(&case), None, &accepted)
            .unwrap();
        let o = Observation {
            id: "pending".into(),
            case_id: Some(case.clone()),
            report_id: None,
            entity_id: e.id.clone(),
            job_id: "job".into(),
            provider: "rdap".into(),
            provider_version: None,
            retrieved_at: "2026-09-28".into(),
            event_time: None,
            event_uncertainty: None,
            basis: Basis::Observed,
            statement: "New finding".into(),
            attribution: "RDAP".into(),
            evidence: vec![],
            fields: serde_json::json!({}),
        };
        store
            .put_record(&o.id, "observation", Some(&case), None, &o)
            .unwrap();
        let data = store.case_projection(&case).unwrap();
        assert_eq!(data.entities[0].id, e.id);
        let desk = DeskProjection::build(&[(case.clone(), data)]);
        assert_eq!(desk.cards[0].kind, NextWorkKind::Review);
        assert_eq!(desk.cards[0].lead_id.as_ref(), Some(&e.id));
        assert!(desk.cards.len() <= 3);
        assert_eq!(desk.queue[0].pending_review, 1);
        assert!(store.jobs().unwrap().is_empty());
    }
    #[test]
    fn scope_and_readiness_prevent_invented_holes_and_partial_is_not_absent() {
        let store = Store::memory().unwrap();
        let (case, e) = scoped_domain(&store, "Harbor");
        let mut configs = crate::research::defaults();
        configs.get_mut("domain").unwrap().enabled = false;
        assert!(store
            .case_projection_with_research(&case, &AtomicBool::new(false), &configs)
            .unwrap()
            .gaps
            .is_empty());
        store
            .save_job("failed", &job(&case, &e, JobState::Partial))
            .unwrap();
        assert!(store.case_projection(&case).unwrap().gaps.is_empty());
        let other = store.create_case("Other").unwrap();
        assert!(store.gaps(&other.id, true).unwrap().is_empty());
    }
    #[test]
    fn no_entity_case_has_actionable_question_card() {
        let store = Store::memory().unwrap();
        let case = store.create_case("Harbor inquiry").unwrap();
        let scope = InvestigationScope {
            question: "Harbor inquiry".into(),
            allowed_actions: vec!["search".into()],
            ..Default::default()
        };
        store
            .put_record("scope", "investigation_scope", Some(&case.id), None, &scope)
            .unwrap();
        let data = store.case_projection(&case.id).unwrap();
        assert!(data.entities.is_empty());
        let desk = DeskProjection::build(&[(case.id, data)]);
        assert_eq!(desk.cards[0].action.as_deref(), Some("search"));
        assert!(desk.cards[0].lead_id.is_none());
    }
}
