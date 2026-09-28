//! Cached case navigation and background projection updates.
use super::*;
use argos_osint_core::{
    evidence::{Entity, EntityType, EvidenceScope},
    investigation::{normalize_entity, CaseProjection, InvestigationScope},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaseView {
    #[default]
    Leads,
    Focus,
    Evidence,
    Timeline,
    Review,
    Jobs,
    Path,
}
impl CaseView {
    pub const ALL: [Self; 7] = [
        Self::Leads,
        Self::Focus,
        Self::Evidence,
        Self::Timeline,
        Self::Review,
        Self::Jobs,
        Self::Path,
    ];
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|v| *v == self).unwrap_or(0)
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Leads => "Leads",
            Self::Focus => "Focus map",
            Self::Evidence => "Evidence table",
            Self::Timeline => "Timeline",
            Self::Review => "Review",
            Self::Jobs => "Jobs",
            Self::Path => "Path",
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct CaseWorkspace {
    pub case_id: String,
    pub question: String,
    pub data: CaseProjection,
    pub snapshot: Option<TnaSnapshot>,
    pub lead_limit: usize,
    pub view: CaseView,
    pub lead_id: Option<String>,
    pub row: usize,
    pub scroll: usize,
    pub link: usize,
    pub source: Option<String>,
    pub actions: Vec<String>,
    pub action_sel: usize,
    pub expanded: Vec<String>,
    pub loading: bool,
    pub filter: String,
    pub sort_by_date: bool,
    pub evidence_sort: usize,
    pub tab_hits: Vec<(CaseView, Rect)>,
    pub view_positions: [(usize, usize); 7],
}
impl CaseWorkspace {
    pub fn switch_view(&mut self, view: CaseView) {
        self.view_positions[self.view.index()] = (self.row, self.scroll);
        self.view = view;
        (self.row, self.scroll) = self.view_positions[view.index()];
        self.source = None;
        self.actions.clear();
    }
    pub fn lead(&self) -> Option<&Entity> {
        self.data
            .entities
            .iter()
            .find(|e| Some(&e.id) == self.lead_id.as_ref())
    }
    pub fn findings(&self) -> Vec<&argos_osint_core::evidence::Finding> {
        let mut rows = self
            .data
            .findings
            .iter()
            .filter(|f| {
                self.filter.is_empty()
                    || format!(
                        "{} {} {} {} {} {:?}",
                        f.observation.statement,
                        f.observation.provider,
                        f.observation.id,
                        f.observation.job_id,
                        f.category,
                        f.decision
                    )
                    .to_lowercase()
                    .contains(&self.filter.to_lowercase())
            })
            .collect::<Vec<_>>();
        if self.sort_by_date {
            rows.sort_by_key(|f| f.observation.retrieved_at.clone());
        }
        rows
    }
    pub fn links(&self) -> Vec<&argos_osint_core::investigation::CaseLink> {
        self.data
            .links
            .iter()
            .filter(|l| {
                self.lead_id.as_ref().is_some_and(|id| {
                    &l.relationship.from == id
                        || &l.relationship.to == id
                        || self.expanded.contains(&l.relationship.from)
                        || self.expanded.contains(&l.relationship.to)
                })
            })
            .take(16)
            .collect()
    }
}
impl App {
    pub(super) fn select_investigation_view(&mut self, view: CaseView) {
        if self.pending_case_data.is_some() {
            self.cancel_case_data_plan();
        }
        self.case_source_generation = self.case_source_generation.wrapping_add(1);
        if let Some(w) = self.investigation.as_mut() {
            w.switch_view(view);
            if view == CaseView::Path {
                self.tna_layout = TnaLayout::Path;
                if self.tna_from.is_none() {
                    self.tna_from = w.lead_id.clone();
                }
            }
            self.focus = Focus::Graph;
        }
    }
    pub(super) fn show_case_data_controls(&mut self) {
        let text = "Case data controls\n\n/clear-case [case ID or title] — remove investigation data, keep the empty case\n/delete-case [case ID or title] — remove investigation data and the case\n\nBoth show a plan before confirmation. Saved reports, versions, and citations remain accessible, detached from the case. Active research must finish or be cancelled first.\n/cancel-case-data cancels a pending plan.";
        if let Some(w) = self.investigation.as_mut() {
            w.source = Some(text.into());
            w.scroll = 0;
        } else {
            self.push_line("assistant", text);
        }
        self.focus = Focus::Prompt;
    }

    pub(super) fn prepare_case_data(&mut self, arg: &str, delete_case: bool) {
        if self.case_data_busy.is_some() {
            self.status = "Case data operation already running".into();
            return;
        }
        let id = if arg.is_empty() {
            self.chat_case.clone()
        } else {
            session::resolve_case(&self.cases, arg).map(|c| c.id.clone())
        };
        let Some(id) = id else {
            self.status = "Open a case or supply its ID/title".into();
            return;
        };
        if self.case_pending_work.get(&id).copied().unwrap_or(0) > 0 {
            self.status =
                "Finish or cancel this case's pending work before clearing or deleting it".into();
            return;
        }
        self.pending_case_data = None;
        self.case_data_generation = self.case_data_generation.wrapping_add(1);
        let generation = self.case_data_generation;
        self.status = "Preparing case data removal plan".into();
        if let Some(path) = self.tna_db_path.clone() {
            let tx = self.tx.clone();
            argos_osint_core::workers::spawn_blocking(move || {
                let result = Store::open(&path)
                    .and_then(|s| s.plan_case_data(&id, delete_case))
                    .map_err(|e| e.to_string());
                let _ = tx.send(AppMsg::CaseDataPlanned { generation, result });
            });
        } else {
            let result = self
                .store
                .plan_case_data(&id, delete_case)
                .map_err(|e| e.to_string());
            self.on_msg(AppMsg::CaseDataPlanned { generation, result });
        }
    }

    pub(super) fn show_case_data_plan(&mut self, plan: argos_osint_core::store::CaseDataPlan) {
        let text = plan.describe();
        if let Some(w) = self.investigation.as_mut() {
            w.source = Some(text);
            w.scroll = 0;
            w.actions.clear();
        } else {
            self.push_line("assistant", &text);
        }
        self.status = format!(
            "Review removal plan · /confirm-case {} · /cancel-case-data",
            plan.case_id
        );
        self.pending_case_data = Some(plan);
        self.focus = Focus::Prompt;
    }

    pub(super) fn cancel_case_data_plan(&mut self) {
        self.case_data_generation = self.case_data_generation.wrapping_add(1);
        if self.pending_case_data.take().is_some() {
            if let Some(w) = self.investigation.as_mut() {
                w.source = None;
            }
        }
        self.status = "Case data removal plan cancelled".into();
    }

    pub(super) fn confirm_case_data(&mut self, id: &str) {
        let Some(plan) = self.pending_case_data.as_ref() else {
            self.status = "Review /clear-case or /delete-case first".into();
            return;
        };
        if id != plan.case_id {
            self.status = "Confirm with the exact case ID shown in the plan".into();
            return;
        }
        if self.case_pending_work.get(id).copied().unwrap_or(0) > 0 {
            self.status =
                "Finish or cancel this case's pending work before applying this plan".into();
            return;
        }
        if self.case_data_busy.is_some() {
            self.status = "Case data operation already running".into();
            return;
        }
        let plan = self.pending_case_data.take().unwrap();
        self.case_data_busy = Some(plan.case_id.clone());
        self.case_cancel.store(true, Ordering::Relaxed);
        self.case_generation = self.case_generation.wrapping_add(1);
        self.case_source_generation = self.case_source_generation.wrapping_add(1);
        self.status = "Applying reviewed case data removal in background".into();
        if let Some(path) = self.tna_db_path.clone() {
            let tx = self.tx.clone();
            argos_osint_core::workers::spawn_blocking(move || {
                let result = Store::open(&path)
                    .and_then(|s| s.apply_case_data_plan(&plan))
                    .map_err(|e| e.to_string());
                let _ = tx.send(AppMsg::CaseDataApplied { plan, result });
            });
        } else {
            let result = self
                .store
                .apply_case_data_plan(&plan)
                .map_err(|e| e.to_string());
            self.on_msg(AppMsg::CaseDataApplied { plan, result });
        }
    }

    pub(super) fn finish_case_data(
        &mut self,
        plan: argos_osint_core::store::CaseDataPlan,
        result: Result<(), String>,
    ) {
        self.case_data_busy = None;
        if let Err(err) = result {
            self.status = err;
            return;
        }
        let was_open = self.chat_case.as_deref() == Some(&plan.case_id);
        self.transcripts.remove(&plan.case_id);
        self.pending_reports.retain(|p| p.case_id != plan.case_id);
        self.research_jobs
            .retain(|j| j.input.case_id.as_deref() != Some(&plan.case_id));
        self.recommendations.clear();
        self.reviewed_recommendations.clear();
        let _ = self.reload_lists();
        if was_open {
            if self.investigation.is_some() {
                self.close_investigation();
            } else {
                self.chat_case = None;
                self.load_transcript("desk");
            }
            if !plan.delete_case {
                self.open_investigation(&plan.case_id);
            }
        }
        if plan.delete_case && self.evidence_scope == EvidenceScope::Case(plan.case_id.clone()) {
            self.evidence_scope = EvidenceScope::Collection;
        }
        self.status = format!(
            "{} {} · historical reports retained",
            if plan.delete_case {
                "Deleted"
            } else {
                "Cleared"
            },
            plan.title
        );
    }

    pub(super) fn open_investigation(&mut self, case_id: &str) {
        self.case_source_generation = self.case_source_generation.wrapping_add(1);
        let Some(case) = self.cases.iter().find(|c| c.id == case_id).cloned() else {
            self.status = "Case not found".into();
            return;
        };
        if self.investigation.is_none() {
            self.desk_return_prompt = self.prompt.clone();
            self.desk_return_scroll = self.scroll_back;
            self.desk_return_scope = self.evidence_scope.clone();
        }
        self.cancel.store(true, Ordering::Relaxed);
        self.turn_generation = self.turn_generation.wrapping_add(1);
        self.running = false;
        self.chat_report = None;
        self.chat_case = Some(case.id.clone());
        self.evidence_scope = EvidenceScope::Case(case.id.clone());
        self.module = Some(ModuleId::Cases);
        self.case_page = CasePage::Investigation;
        self.focus = Focus::Graph;
        self.prompt.clear();
        self.cursor = 0;
        self.investigation = Some(CaseWorkspace {
            case_id: case.id,
            question: case.title,
            lead_limit: self.settings.analysis.lead_limit,
            ..Default::default()
        });
        self.refresh_investigation();
    }
    pub(super) fn refresh_investigation(&mut self) {
        let Some(w) = self.investigation.as_mut() else {
            return;
        };
        w.loading = true;
        self.case_cancel.store(true, Ordering::Relaxed);
        self.case_cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.case_cancel.clone();
        self.case_generation = self.case_generation.wrapping_add(1);
        let generation = self.case_generation;
        let id = w.case_id.clone();
        if let Some(path) = self.tna_db_path.clone() {
            let tx = self.tx.clone();
            argos_osint_core::workers::spawn_blocking(move || {
                let result = Store::open(&path)
                    .and_then(|s| s.case_projection_cancellable(&id, &cancel))
                    .map_err(|e| e.to_string());
                let _ = tx.send(AppMsg::CaseReady {
                    case_id: id,
                    generation,
                    result,
                });
            });
        } else {
            let result = self.store.case_projection(&id).map_err(|e| e.to_string());
            self.on_msg(AppMsg::CaseReady {
                case_id: id,
                generation,
                result,
            });
        }
    }
    pub(super) fn close_investigation(&mut self) {
        self.case_source_generation = self.case_source_generation.wrapping_add(1);
        self.case_cancel.store(true, Ordering::Relaxed);
        self.cancel.store(true, Ordering::Relaxed);
        self.turn_generation = self.turn_generation.wrapping_add(1);
        self.running = false;
        self.case_generation = self.case_generation.wrapping_add(1);
        self.investigation = None;
        self.chat_case = None;
        self.chat_report = None;
        self.case_page = CasePage::Closed;
        self.module = Some(ModuleId::Cases);
        self.focus = Focus::Prompt;
        self.prompt = std::mem::take(&mut self.desk_return_prompt);
        self.cursor = self.prompt.chars().count();
        self.scroll_back = self.desk_return_scroll;
        self.evidence_scope = self.desk_return_scope.clone();
        self.load_transcript("desk");
    }
    pub(super) fn selected_case_source(&mut self) {
        let Some(w) = self.investigation.as_ref() else {
            return;
        };
        let reference = if w.view == CaseView::Focus {
            w.links()
                .get(w.link)
                .and_then(|l| l.relationship.evidence.first())
                .cloned()
        } else if w.view == CaseView::Path {
            self.tna_paths()
                .get(self.tna_path_sel)
                .and_then(|p| {
                    p.nodes
                        .windows(2)
                        .nth(self.tna_hop_sel)
                        .map(|pair| (pair[0].clone(), pair[1].clone()))
                })
                .and_then(|(from, to)| {
                    w.data.links.iter().find(|l| {
                        !l.candidate
                            && ((l.relationship.from == from && l.relationship.to == to)
                                || (l.relationship.from == to && l.relationship.to == from))
                    })
                })
                .and_then(|l| l.relationship.evidence.first())
                .cloned()
        } else if w.view == CaseView::Timeline {
            w.data
                .timeline
                .get(w.row)
                .and_then(|e| e.evidence.first())
                .cloned()
        } else if w.view == CaseView::Evidence {
            w.evidence_rows()
                .get(w.row)
                .and_then(|row| row.evidence.first())
                .cloned()
        } else {
            w.findings()
                .get(w.row)
                .and_then(|f| f.observation.evidence.first())
                .cloned()
        };
        let Some(reference) = reference else {
            self.status = "No source selected".into();
            return;
        };
        let scope = self.evidence_scope.clone();
        self.case_source_generation = self.case_source_generation.wrapping_add(1);
        let generation = self.case_source_generation;
        let id = w.case_id.clone();
        if let Some(path) = self.tna_db_path.clone() {
            let tx = self.tx.clone();
            argos_osint_core::workers::spawn_blocking(move || {
                let result = Store::open(&path)
                    .and_then(|s| s.resolve_evidence(&reference, &scope))
                    .map_err(|e| e.to_string());
                let _ = tx.send(AppMsg::CaseSource {
                    case_id: id,
                    generation,
                    result,
                });
            });
        } else {
            let result = self
                .store
                .resolve_evidence(&reference, &scope)
                .map_err(|e| e.to_string());
            self.on_msg(AppMsg::CaseSource {
                case_id: id,
                generation,
                result,
            });
        }
    }
    pub(super) fn on_investigation_key(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Esc {
            if self.pending_case_data.is_some() {
                self.cancel_case_data_plan();
                return false;
            }
            let w = self.investigation.as_mut().unwrap();
            if w.source.take().is_some() {
                return false;
            }
            if !w.actions.is_empty() {
                w.actions.clear();
                return false;
            }
            self.close_investigation();
            return false;
        }
        if self.focus == Focus::Prompt {
            return self.on_prompt_key(key);
        }
        if key.code == KeyCode::Char('/') {
            self.focus = Focus::Prompt;
            return self.on_prompt_key(key);
        }
        if key.code == KeyCode::Char('?') {
            self.help = true;
            return false;
        }
        if key.code == KeyCode::Char('D') {
            self.show_case_data_controls();
            return false;
        }
        if key.code == KeyCode::Tab {
            self.focus = Focus::Prompt;
            return false;
        }
        if let KeyCode::Char(c @ '1'..='7') = key.code {
            self.select_investigation_view(CaseView::ALL[(c as u8 - b'1') as usize]);
            return false;
        }
        let w = self.investigation.as_mut().unwrap();
        if w.source.is_some() {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => w.scroll += 1,
                KeyCode::Char('k') | KeyCode::Up => w.scroll = w.scroll.saturating_sub(1),
                _ => {}
            }
            return false;
        }
        if !w.actions.is_empty() {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    w.action_sel = (w.action_sel + 1) % w.actions.len()
                }
                KeyCode::Char('k') | KeyCode::Up => w.action_sel = w.action_sel.saturating_sub(1),
                KeyCode::Enter => {
                    let provider = w.actions[w.action_sel].clone();
                    w.actions.clear();
                    self.submit_enrichment(&provider);
                }
                _ => {}
            }
            return false;
        }
        match key.code {
            KeyCode::Char('f') if w.view == CaseView::Path => {
                self.tna_from = w.lead_id.clone();
                self.tna_path_sel = 0;
                self.tna_hop_sel = 0;
            }
            KeyCode::Char('t') if w.view == CaseView::Path => {
                self.tna_to = w.lead_id.clone();
                self.tna_path_sel = 0;
                self.tna_hop_sel = 0;
            }
            KeyCode::Char('n') if w.view == CaseView::Path => {
                self.tna_path_sel =
                    (self.tna_path_sel + 1).min(self.tna_paths().len().saturating_sub(1));
                self.tna_hop_sel = 0;
            }
            KeyCode::Char('N') if w.view == CaseView::Path => {
                self.tna_path_sel = self.tna_path_sel.saturating_sub(1);
                self.tna_hop_sel = 0;
            }
            KeyCode::Char(']') if w.view == CaseView::Path => {
                self.tna_hop_sel = (self.tna_hop_sel + 1).min(
                    self.tna_paths()
                        .get(self.tna_path_sel)
                        .map(|p| p.nodes.len().saturating_sub(2))
                        .unwrap_or(0),
                );
            }
            KeyCode::Char('[') if w.view == CaseView::Path => {
                self.tna_hop_sel = self.tna_hop_sel.saturating_sub(1)
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if matches!(w.view, CaseView::Leads | CaseView::Focus | CaseView::Path) {
                    if let Some(e) = w.data.entities.get(
                        (w.data
                            .entities
                            .iter()
                            .position(|e| Some(&e.id) == w.lead_id.as_ref())
                            .unwrap_or(0)
                            + 1)
                        .min(w.data.entities.len().saturating_sub(1)),
                    ) {
                        w.lead_id = Some(e.id.clone());
                        w.link = 0;
                    }
                } else {
                    let len = match w.view {
                        CaseView::Timeline => w.data.timeline.len(),
                        CaseView::Jobs => w.data.jobs.len(),
                        CaseView::Evidence => w.evidence_rows().len(),
                        _ => w.findings().len(),
                    };
                    w.row = (w.row + 1).min(len.saturating_sub(1));
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if matches!(w.view, CaseView::Leads | CaseView::Focus | CaseView::Path) {
                    if let Some(e) = w.data.entities.get(
                        w.data
                            .entities
                            .iter()
                            .position(|e| Some(&e.id) == w.lead_id.as_ref())
                            .unwrap_or(0)
                            .saturating_sub(1),
                    ) {
                        w.lead_id = Some(e.id.clone());
                        w.link = 0;
                    }
                } else {
                    w.row = w.row.saturating_sub(1);
                }
            }
            KeyCode::Right => w.link = (w.link + 1).min(w.links().len().saturating_sub(1)),
            KeyCode::Left => w.link = w.link.saturating_sub(1),
            KeyCode::Char('x') => {
                if let Some(l) = w.links().get(w.link) {
                    let r = &l.relationship;
                    let id = if Some(&r.from) == w.lead_id.as_ref() {
                        r.to.clone()
                    } else {
                        r.from.clone()
                    };
                    if !w.expanded.contains(&id) {
                        w.expanded.push(id);
                    }
                }
            }
            KeyCode::Char('s') => {
                if w.view == CaseView::Evidence {
                    w.evidence_sort = (w.evidence_sort + 1) % 7;
                } else {
                    w.sort_by_date = !w.sort_by_date;
                }
                w.row = 0;
            }
            KeyCode::Char('e') => {
                let label = w
                    .lead()
                    .map(|e| e.label.clone())
                    .unwrap_or_else(|| w.question.clone());
                let scope = w.data.scope.as_ref();
                w.actions = self
                    .settings
                    .research
                    .iter()
                    .filter(|(name, c)| {
                        scope.is_none_or(|s| s.allowed_actions.contains(name))
                            && argos_osint_core::research::eligible_action(name, &label, c).is_ok()
                            && (name.as_str() != "shodan"
                                || self
                                    .auth
                                    .research
                                    .get(&c.secret_ref)
                                    .is_some_and(|s| !s.is_empty()))
                    })
                    .map(|(name, _)| name.clone())
                    .collect();
                w.action_sel = 0;
                if w.actions.is_empty() {
                    self.status =
                        "No eligible actions; check investigation scope and Providers → Research"
                            .into();
                }
            }
            KeyCode::Char('i') | KeyCode::Enter if w.view == CaseView::Leads => {
                let mut detail = String::new();
                if let Some(e) = w.lead() {
                    detail.push_str(&format!(
                        "{} · {:?}\nCanonical: {}\nAliases: {}\n\n",
                        e.label,
                        e.kind,
                        e.canonical,
                        e.aliases.join(", ")
                    ));
                    for f in w
                        .data
                        .findings
                        .iter()
                        .filter(|f| f.observation.entity_id == e.id)
                        .take(20)
                    {
                        let o = &f.observation;
                        detail.push_str(&format!(
                            "{} · {:?} · {}\n{}\nAttribution: {} · retrieved {}\nSources: {}\n\n",
                            o.id,
                            f.decision,
                            f.category,
                            o.statement,
                            o.attribution,
                            o.retrieved_at,
                            o.evidence
                                .iter()
                                .filter_map(|r| r.passage_id.as_ref().or(r.artifact_id.as_ref()))
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                    for l in w
                        .data
                        .links
                        .iter()
                        .filter(|l| l.relationship.from == e.id || l.relationship.to == e.id)
                        .take(16)
                    {
                        detail.push_str(&format!(
                            "{:?} · {:?} · {}\n{}\n\n",
                            l.relationship.kind,
                            l.relationship.basis,
                            if l.candidate {
                                "candidate"
                            } else {
                                "supported"
                            },
                            l.relationship.uncertainty
                        ));
                    }
                } else {
                    detail.push_str("No entity selected. The investigation question can be explicitly searched.");
                }
                detail.push_str("Esc returns · e offers eligible focused actions · /source <observation ID> opens the original source.");
                w.source = Some(detail);
                w.scroll = 0;
            }
            KeyCode::Enter if w.view == CaseView::Jobs => {
                if let Some(j) = w.data.jobs.get(w.row) {
                    w.filter = j.id.clone();
                    w.view = CaseView::Review;
                    w.row = 0;
                }
            }
            KeyCode::Char('v') if w.view == CaseView::Focus => {
                if let Some(l) = w.links().get(w.link) {
                    w.filter = l.observations.first().cloned().unwrap_or_default();
                    w.view = CaseView::Review;
                    w.row = 0;
                }
            }
            KeyCode::Char('p') if w.view == CaseView::Focus => {
                if let Some(l) = w.links().get(w.link) {
                    self.tna_from = Some(l.relationship.from.clone());
                    self.tna_to = Some(l.relationship.to.clone());
                    w.switch_view(CaseView::Path);
                    self.tna_layout = TnaLayout::Path;
                    self.tna_path_sel = 0;
                    self.tna_hop_sel = 0;
                }
            }
            KeyCode::Char('z') => w.expanded.clear(),
            KeyCode::Enter | KeyCode::Char('o') => self.selected_case_source(),
            KeyCode::Char('a') | KeyCode::Char('r') | KeyCode::Char('d') | KeyCode::Char('t')
                if w.view == CaseView::Review =>
            {
                if let Some(f) = w.findings().get(w.row) {
                    let decision = match key.code {
                        KeyCode::Char('a') => "accept",
                        KeyCode::Char('r') => "reject",
                        KeyCode::Char('d') => "defer",
                        _ => "retain",
                    };
                    self.prompt = format!("/review {} {decision} ", f.observation.id);
                    self.cursor = self.prompt.chars().count();
                    self.focus = Focus::Prompt;
                    self.status = "Add a review reason and Enter".into();
                }
            }
            _ => {}
        }
        false
    }
    pub(super) fn start_case_workspace(
        &mut self,
        query: String,
        echo: bool,
        plan: SourcePlan,
        existing: Option<String>,
        include: bool,
        allow_sensitive: bool,
        allow_active: bool,
    ) {
        let case = match existing
            .and_then(|id| self.cases.iter().find(|c| c.id == id).cloned())
            .map(Ok)
            .unwrap_or_else(|| self.store.create_case(&query))
        {
            Ok(c) => c,
            Err(e) => {
                self.status = e.to_string();
                return;
            }
        };
        let parsed = argos_osint_core::search::TextQuery::extract(&query);
        let mut seeds = parsed
            .domains
            .into_iter()
            .map(|s| (s, EntityType::Domain))
            .chain(parsed.emails.into_iter().map(|s| (s, EntityType::Email)))
            .chain(parsed.handles.into_iter().map(|s| (s, EntityType::Account)))
            .collect::<Vec<_>>();
        if let Ok(ip) = query.parse::<std::net::IpAddr>() {
            seeds.push((ip.to_string(), EntityType::Ip));
        }
        let mut seed_entities = Vec::new();
        for (label, kind) in seeds {
            if let Ok(e) = normalize_entity(&label, kind) {
                let _ = self
                    .store
                    .put_record(&e.id, "entity", Some(&case.id), None, &e);
                seed_entities.push(e.id);
            }
        }
        let allowed_actions = self
            .settings
            .research
            .keys()
            .filter(|name| {
                (allow_sensitive
                    || !matches!(name.as_str(), "leakcheck" | "xposedornot" | "mosint"))
                    && (allow_active
                        || !matches!(
                            name.as_str(),
                            "contacts" | "katana" | "whatsmyname" | "maigret"
                        ))
            })
            .cloned()
            .collect();
        let scope = InvestigationScope {
            question: query.clone(),
            seed_entities,
            included_evidence: if include {
                self.recommendations
                    .iter()
                    .map(|p| argos_osint_core::evidence::EvidenceRef {
                        report_id: Some(p.report_id.clone()),
                        passage_id: Some(p.id.clone()),
                        ..Default::default()
                    })
                    .collect()
            } else {
                vec![]
            },
            allowed_actions,
        };
        let _ = self.store.put_record(
            &argos_osint_core::store::new_id("case-scope"),
            "investigation_scope",
            Some(&case.id),
            None,
            &scope,
        );
        if echo {
            self.append_to_session("desk", "user", &query);
        }
        self.append_to_session(
            "desk",
            "assistant",
            &format!(
                "Case saved: {}. /case {} opens Leads. Reports can be drafted after review.",
                case.title, case.id
            ),
        );
        let _ = self.reload_lists();
        self.open_investigation(&case.id);
        if let Some(w) = self.investigation.as_mut() {
            w.data.scope = Some(scope);
        }
        // Each selected first action is bounded; discoveries are never recursively submitted.
        if plan.facts || plan.web || plan.news || plan.social {
            self.submit_enrichment_with_plan(&format!("search {query}"), plan.clone());
        }
        let parsed = argos_osint_core::search::TextQuery::extract(&query);
        if plan.domain {
            if let Some(label) = parsed.domains.first() {
                self.submit_enrichment_with_plan(&format!("domain {label}"), plan.clone());
            }
        }
        if plan.identity {
            if let Some(label) = parsed.emails.first().or(parsed.handles.first()) {
                self.submit_enrichment_with_plan(&format!("identity {label}"), plan.clone());
            }
        }
        self.status = "Case saved · Leads · e choose enrichment · 2 Focus · 5 Review".into();
    }
}

impl App {
    pub(super) fn draft_investigation(&mut self, arg: &str) {
        if self.case_data_busy.is_some() {
            self.status = "Wait for case data removal to finish".into();
            return;
        }
        let Some(w) = self.investigation.as_ref() else {
            self.status = "Open a case first".into();
            return;
        };
        let Some(path) = self.tna_db_path.clone() else {
            self.status = "Drafting requires a saved workspace".into();
            return;
        };
        let mut words = arg.split_whitespace();
        let mode = words
            .next()
            .unwrap_or(&self.settings.analysis.report_mode)
            .to_string();
        let selected = words.map(str::to_string).collect::<Vec<_>>();
        if selected.is_empty() {
            self.status =
                "/draft addendum|revision|followup|final <accepted observation IDs>".into();
            return;
        }
        let id = w.case_id.clone();
        let directory = report_dir(&self.settings);
        let tx = self.tx.clone();
        *self.case_pending_work.entry(id.clone()).or_default() += 1;
        argos_osint_core::workers::spawn_blocking(move || {
            let result = Store::open(&path)
                .and_then(|s| s.draft_case_report(&id, &mode, &selected, &directory))
                .map_err(|e| e.to_string());
            let _ = tx.send(AppMsg::CaseDrafted {
                case_id: id,
                result,
            });
        });
        self.status = "Drafting selected reviewed evidence in background".into();
    }
}

impl App {
    pub(super) fn open_observation_source(&mut self, id: &str) {
        let Some(path) = self.tna_db_path.clone() else {
            self.status = "Source opening requires a saved workspace".into();
            return;
        };
        let id = id.to_string();
        let scope = self.evidence_scope.clone();
        let tx = self.tx.clone();
        let report_id = self.chat_report.clone();
        argos_osint_core::workers::spawn_blocking(move || {
            let result = Store::open(&path)
                .and_then(|s| {
                    let f = s
                        .findings_scoped(&scope)?
                        .into_iter()
                        .find(|f| f.observation.id == id)
                        .ok_or_else(|| anyhow::anyhow!("Observation outside selected scope"))?;
                    let source = f
                        .observation
                        .evidence
                        .first()
                        .ok_or_else(|| anyhow::anyhow!("Source unavailable"))?;
                    s.resolve_evidence(source, &scope)
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(AppMsg::EvidenceSurface {
                report_id,
                title: "Original source".into(),
                result,
                scope,
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use argos_osint_core::evidence::*;
    use ratatui::{backend::TestBackend, Terminal};
    fn fixture() -> App {
        let store = Store::memory().unwrap();
        store.ensure_session("desk", "Desk", "desk").unwrap();
        let case = store.create_case("Harbor inquiry").unwrap();
        let a = normalize_entity("harbor.example", EntityType::Domain).unwrap();
        let b = normalize_entity("contact@harbor.example", EntityType::Email).unwrap();
        for e in [&a, &b] {
            store
                .put_record(&e.id, "entity", Some(&case.id), None, e)
                .unwrap();
        }
        let artifact = Artifact {
            id: "artifact".into(),
            source_url: Some("https://harbor.example/".into()),
            retrieved_at: "2026-09-27".into(),
            media_type: "text/plain".into(),
            body: "Original passage publishes contact@harbor.example".into(),
        };
        store
            .put_record(&artifact.id, "artifact", Some(&case.id), None, &artifact)
            .unwrap();
        let observation = Observation {
            id: "o1".into(),
            case_id: Some(case.id.clone()),
            report_id: None,
            entity_id: a.id.clone(),
            job_id: "j1".into(),
            provider: "contacts".into(),
            provider_version: None,
            retrieved_at: "2026-09-27".into(),
            event_time: None,
            event_uncertainty: None,
            basis: Basis::Observed,
            statement: "Published address contact@harbor.example".into(),
            attribution: "harbor.example page".into(),
            evidence: vec![EvidenceRef {
                artifact_id: Some("artifact".into()),
                ..Default::default()
            }],
            fields: serde_json::json!({}),
        };
        store
            .put_record(
                &observation.id,
                "observation",
                Some(&case.id),
                None,
                &observation,
            )
            .unwrap();
        let relationship = Relationship {
            from: a.id,
            to: b.id,
            kind: RelationshipType::AddressPublishedOnPage,
            basis: Basis::Observed,
            evidence: observation.evidence,
            uncertainty: "Publication does not establish ownership".into(),
        };
        store
            .put_record("r1", "relationship", Some(&case.id), None, &relationship)
            .unwrap();
        let mut app = App::from_parts(store, SettingsFile::default(), AuthFile::default()).unwrap();
        app.prompt = "Desk prompt stays".into();
        app.scroll_back = 9;
        app.open_investigation(&case.id);
        app
    }
    fn render(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| super::super::super::ui::draw(frame, app))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }
    #[test]
    fn case_data_controls_require_a_reviewed_exact_id_and_can_cancel() {
        let mut app = fixture();
        let id = app.chat_case.clone().unwrap();
        app.confirm_case_data(&id);
        assert_eq!(app.store.list_cases().unwrap().len(), 1);
        app.prepare_case_data("", false);
        assert!(app.pending_case_data.is_some());
        assert!(app
            .investigation
            .as_ref()
            .unwrap()
            .source
            .as_ref()
            .unwrap()
            .contains("This cannot be undone"));
        app.confirm_case_data("wrong-case");
        assert_eq!(app.store.case_projection(&id).unwrap().entities.len(), 2);
        app.cancel_case_data_plan();
        assert!(app.pending_case_data.is_none());
        app.prepare_case_data("", false);
        app.confirm_case_data(&id);
        assert_eq!(app.store.list_cases().unwrap().len(), 1);
        assert!(app.investigation.as_ref().unwrap().data.entities.is_empty());
        app.prepare_case_data("", true);
        app.confirm_case_data(&id);
        assert!(app.store.list_cases().unwrap().is_empty());
        assert!(app.investigation.is_none());
        assert_eq!(app.prompt, "Desk prompt stays");
    }

    #[test]
    fn focus_pair_opens_only_path_and_accepted_hop_opens_its_original_source() {
        let mut app = fixture();
        app.store
            .review_finding(
                "o1",
                argos_osint_core::evidence::ReviewDecision::Accept,
                "source confirms publication",
            )
            .unwrap();
        app.refresh_investigation();
        app.investigation.as_mut().unwrap().view = CaseView::Focus;
        app.focus = Focus::Graph;
        app.on_investigation_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
        assert_eq!(app.investigation.as_ref().unwrap().view, CaseView::Path);
        assert_eq!(app.tna_paths().len(), 1);
        let view = render(&mut app, 100, 30);
        assert!(view.contains("Path"));
        assert!(
            !view.contains("Cockpit")
                && !view.contains("Matrix")
                && !view.contains("Ribbon")
                && !view.contains("Clusters")
        );
        app.on_investigation_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        assert_eq!(app.investigation.as_ref().unwrap().view, CaseView::Path);
        app.selected_case_source();
        assert!(app
            .investigation
            .as_ref()
            .unwrap()
            .source
            .as_ref()
            .unwrap()
            .contains("Original passage"));
        assert!(app.research_jobs.is_empty());
    }

    #[test]
    fn every_case_view_has_a_clickable_tab_and_keeps_its_position() {
        let mut app = fixture();
        for width in [140, 80, 48] {
            render(&mut app, width, 40);
            let hits = app.investigation.as_ref().unwrap().tab_hits.clone();
            assert_eq!(hits.len(), CaseView::ALL.len());
            for (view, rect) in hits {
                let header = app.case_tab_area;
                assert!(rect.x > header.x && rect.right() < header.right());
                assert!(rect.y > header.y && rect.bottom() < header.bottom());
                app.click(rect.x, rect.y);
                assert_eq!(app.investigation.as_ref().unwrap().view, view);
                assert_eq!(app.focus, Focus::Graph);
            }
        }
        app.select_investigation_view(CaseView::Evidence);
        app.investigation.as_mut().unwrap().row = 3;
        app.investigation.as_mut().unwrap().scroll = 4;
        app.select_investigation_view(CaseView::Jobs);
        app.investigation.as_mut().unwrap().row = 2;
        app.select_investigation_view(CaseView::Evidence);
        let w = app.investigation.as_ref().unwrap();
        assert_eq!((w.row, w.scroll), (3, 4));
        app.select_investigation_view(CaseView::Jobs);
        assert_eq!(app.investigation.as_ref().unwrap().row, 2);
        assert!(app.research_jobs.is_empty());
    }
    #[test]
    fn case_tabs_use_the_top_case_title_panel_and_desk_header_returns_on_close() {
        let mut app = fixture();
        app.investigation.as_mut().unwrap().question = "A different investigation question".into();
        let mut terminal = Terminal::new(TestBackend::new(140, 36)).unwrap();
        terminal
            .draw(|frame| super::super::super::ui::draw(frame, &mut app))
            .unwrap();
        let header = app.case_tab_area;
        assert_eq!(header.y, 0);
        let title = (header.x..header.right())
            .map(|x| terminal.backend().buffer()[(x, header.y)].symbol())
            .collect::<String>();
        assert!(title.contains("Harbor inquiry"));
        assert!(
            !title.contains("Case Desk") && !title.contains("different investigation question")
        );
        assert_eq!(app.investigation.as_ref().unwrap().tab_hits.len(), 7);
        app.close_investigation();
        terminal
            .draw(|frame| super::super::super::ui::draw(frame, &mut app))
            .unwrap();
        let header = app.case_tab_area;
        let title = (header.x..header.right())
            .map(|x| terminal.backend().buffer()[(x, header.y)].symbol())
            .collect::<String>();
        assert!(title.contains("Case Desk"));
        assert!(!app.case_tab_hits.is_empty());
    }
    #[test]
    fn lead_navigation_is_cached_and_escape_restores_desk_state() {
        let mut app = fixture();
        let jobs = app.research_jobs.len();
        let reports = app.reports.len();
        let first = app.investigation.as_ref().unwrap().lead_id.clone();
        app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_ne!(app.investigation.as_ref().unwrap().lead_id, first);
        assert_eq!(app.research_jobs.len(), jobs);
        assert_eq!(app.reports.len(), reports);
        assert_eq!(app.research_active, 0);
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.investigation.is_none());
        assert_eq!(app.prompt, "Desk prompt stays");
        assert_eq!(app.scroll_back, 9);
        assert_eq!(app.evidence_scope, EvidenceScope::Collection);
    }
    #[test]
    fn focus_table_and_timeline_open_the_same_source_and_stale_refresh_is_ignored() {
        let mut app = fixture();
        let id = app.investigation.as_ref().unwrap().case_id.clone();
        let selected = app.investigation.as_ref().unwrap().lead_id.clone();
        for view in [CaseView::Focus, CaseView::Evidence, CaseView::Timeline] {
            let w = app.investigation.as_mut().unwrap();
            w.view = view;
            w.row = 0;
            w.link = 0;
            app.selected_case_source();
            assert!(app
                .investigation
                .as_ref()
                .unwrap()
                .source
                .as_ref()
                .unwrap()
                .contains("Original passage"));
            app.investigation.as_mut().unwrap().source = None;
        }
        app.on_msg(AppMsg::CaseReady {
            case_id: id,
            generation: app.case_generation + 1,
            result: Ok(CaseProjection::default()),
        });
        assert_eq!(app.investigation.as_ref().unwrap().lead_id, selected);
        assert_eq!(app.investigation.as_ref().unwrap().data.entities.len(), 2);
    }
    #[test]
    fn all_case_views_fit_narrow_terminals_and_distinguish_candidates() {
        let mut app = fixture();
        for view in [
            CaseView::Leads,
            CaseView::Focus,
            CaseView::Evidence,
            CaseView::Timeline,
            CaseView::Review,
            CaseView::Jobs,
            CaseView::Path,
        ] {
            app.investigation.as_mut().unwrap().view = view;
            for (width, height) in [(140, 42), (80, 24), (48, 18), (24, 10), (12, 5)] {
                let _ = render(&mut app, width, height);
            }
        }
        app.investigation.as_mut().unwrap().view = CaseView::Focus;
        let text = render(&mut app, 140, 42);
        assert!(
            text.contains("candidate")
                && text.contains("Published address")
                && text.contains("does not establish ownership"),
            "{text}"
        );
        assert!(!text.contains("Strategic"));
    }
    #[test]
    fn case_expand_shortcut_and_relationship_table_use_cached_supported_sources() {
        let mut app = fixture();
        app.on_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(app.store.list_cases().unwrap().len(), 1);
        assert_eq!(app.investigation.as_ref().unwrap().expanded.len(), 1);
        app.investigation.as_mut().unwrap().view = CaseView::Evidence;
        app.investigation.as_mut().unwrap().filter = "Published address".into();
        let rows = app.investigation.as_ref().unwrap().evidence_rows();
        assert!(rows
            .iter()
            .any(|r| r.relationship == "Published address · observed"));
        let index = rows
            .iter()
            .position(|r| r.relationship == "Published address · observed")
            .unwrap();
        app.investigation.as_mut().unwrap().row = index;
        app.selected_case_source();
        assert!(app
            .investigation
            .as_ref()
            .unwrap()
            .source
            .as_ref()
            .unwrap()
            .contains("Original passage"));
    }
    #[test]
    fn desk_gap_creates_a_persisted_case_without_report_or_selected_actions() {
        let store = Store::memory().unwrap();
        let mut app = App::from_parts(store, SettingsFile::default(), AuthFile::default()).unwrap();
        app.retrieve_question("what is known about harbor.example".into());
        assert!(!app.running);
        assert!(app.confirm_query.is_none() && app.scope.is_none());
        assert!(app
            .transcript()
            .iter()
            .any(|l| l.body.contains("Suggested investigation")));
        assert_eq!(app.research_active, 0);
        app.open_scope("harbor.example".into(), false, false);
        let draft = app.scope.as_ref().unwrap();
        assert!(!draft.facts && !draft.web && !draft.domain);
        app.on_scope_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.investigation.is_some());
        assert_eq!(app.store.list_cases().unwrap().len(), 1);
        assert!(app.store.list_reports().unwrap().is_empty());
        assert!(app.store.jobs().unwrap().is_empty());
        assert!(app.investigation.as_ref().unwrap().data.links.is_empty());
    }
    #[test]
    fn research_phase_cards_are_inside_providers_and_opening_never_runs_collection() {
        let mut app = fixture();
        app.open_module(ModuleId::Providers);
        app.select_provider_page(ProviderPage::Research);
        for phase in argos_osint_core::research::ResearchPhase::ALL {
            app.research_phase = phase;
            app.research_sel = 0;
            app.load_research_fields();
            for (width, height) in [(140, 42), (48, 18), (24, 10)] {
                let text = render(&mut app, width, height);
                assert!(!text.contains("secret-value"));
            }
            assert_eq!(app.research_active, 0);
            assert!(app.store.jobs().unwrap().is_empty());
        }
        app.research_phase = argos_osint_core::research::ResearchPhase::Infrastructure;
        app.research_sel = 3;
        app.load_research_fields();
        let text = render(&mut app, 140, 42);
        assert!(
            text.contains("Katana") && text.contains("COLLECTION UNAVAILABLE"),
            "{text}"
        );
        app.research_phase = argos_osint_core::research::ResearchPhase::Analysis;
        app.load_research_fields();
        assert_eq!(app.fields[0].key, "analysis_report_mode");
        app.fields[0].value = "followup".into();
        app.research_field_action("__analysis_save");
        assert_eq!(app.settings.analysis.report_mode, "followup");
    }
}

pub const EVIDENCE_SORTS: [&str; 7] = [
    "source history",
    "entity/type",
    "relationship",
    "source",
    "event date",
    "retrieval date",
    "review state",
];
pub struct EvidenceTableRow {
    pub entity: String,
    pub relationship: String,
    pub state: String,
    pub source: String,
    pub event: Option<String>,
    pub retrieved: String,
    pub statement: String,
    pub evidence: Vec<argos_osint_core::evidence::EvidenceRef>,
}
impl CaseWorkspace {
    pub fn evidence_rows(&self) -> Vec<EvidenceTableRow> {
        let entity_name = |id: &str| {
            self.data
                .entities
                .iter()
                .find(|e| e.id == id)
                .map(|e| format!("{} ({:?})", e.label, e.kind))
                .unwrap_or_else(|| id.into())
        };
        let source = |evidence: &[argos_osint_core::evidence::EvidenceRef]| {
            evidence
                .iter()
                .filter_map(|r| {
                    r.source_url
                        .as_ref()
                        .or(r.passage_id.as_ref())
                        .or(r.artifact_id.as_ref())
                })
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut rows = self
            .data
            .findings
            .iter()
            .map(|f| {
                let o = &f.observation;
                EvidenceTableRow {
                    entity: entity_name(&o.entity_id),
                    relationship: format!("Observation · {:?}", o.basis),
                    state: format!("{:?} · {}", f.decision, f.category),
                    source: format!("{} · {}", o.provider, source(&o.evidence)),
                    event: o.event_time.clone(),
                    retrieved: o.retrieved_at.clone(),
                    statement: format!("{} · {}", o.id, o.statement),
                    evidence: o.evidence.clone(),
                }
            })
            .collect::<Vec<_>>();
        rows.extend(self.data.links.iter().map(|l| {
            let r = &l.relationship;
            let o = self
                .data
                .findings
                .iter()
                .find(|f| l.observations.contains(&f.observation.id))
                .map(|f| &f.observation);
            EvidenceTableRow {
                entity: format!("{} → {}", entity_name(&r.from), entity_name(&r.to)),
                relationship: format!("{} · {}", r.kind.label(), r.basis.label()),
                state: format!(
                    "{} · {}",
                    if l.reviewed { "accepted" } else { "pending" },
                    if l.candidate {
                        "candidate"
                    } else {
                        "supported"
                    }
                ),
                source: source(&r.evidence),
                event: o.and_then(|o| o.event_time.clone()),
                retrieved: o.map(|o| o.retrieved_at.clone()).unwrap_or_default(),
                statement: r.uncertainty.clone(),
                evidence: r.evidence.clone(),
            }
        }));
        rows.retain(|r| {
            format!(
                "{} {} {} {} {:?} {} {}",
                r.entity, r.relationship, r.state, r.source, r.event, r.retrieved, r.statement
            )
            .to_lowercase()
            .contains(&self.filter.to_lowercase())
        });
        rows.sort_by_key(|r| match self.evidence_sort {
            1 => r.entity.clone(),
            2 => r.relationship.clone(),
            3 => r.source.clone(),
            4 => r.event.clone().unwrap_or_else(|| "~undated".into()),
            5 => r.retrieved.clone(),
            6 => r.state.clone(),
            _ => String::new(),
        });
        rows
    }
}
