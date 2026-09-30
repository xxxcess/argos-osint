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
    Product,
}
impl CaseView {
    pub const ALL: [Self; 8] = [
        Self::Leads,
        Self::Focus,
        Self::Evidence,
        Self::Timeline,
        Self::Review,
        Self::Jobs,
        Self::Path,
        Self::Product,
    ];
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|v| *v == self).unwrap_or(0)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaseCenter {
    #[default]
    Workbench,
    Graph,
    Product,
}
#[derive(Clone, Debug, Default)]
pub struct CaseWorkspace {
    pub case_id: String,
    pub question: String,
    pub data: CaseProjection,
    pub snapshot: Option<TnaSnapshot>,
    pub lead_limit: usize,
    pub view: CaseView,
    pub center: CaseCenter,
    pub path_from: Option<String>,
    pub path_to: Option<String>,
    pub path_sel: usize,
    pub hop_sel: usize,
    pub plan_checked: Vec<String>,
    pub plan_focus: bool,
    pub inbox_focus: bool,
    pub gap_focus: bool,
    pub gaps_case_wide: bool,
    pub gap_sel: usize,
    pub product_checked: Vec<String>,
    pub product_case_wide: bool,
    pub global_jobs: bool,
    pub view_links: [usize; 8],
    pub lead_id: Option<String>,
    pub row: usize,
    pub scroll: usize,
    pub link: usize,
    pub source: Option<String>,
    pub actions: Vec<String>,
    pub disabled_actions: Vec<(String, String)>,
    pub action_sel: usize,
    pub expanded: Vec<String>,
    pub loading: bool,
    pub filter: String,
    pub sort_by_date: bool,
    pub evidence_sort: usize,
    pub tab_hits: Vec<(CaseView, Rect)>,
    pub view_positions: [(usize, usize); 8],
}
impl CaseWorkspace {
    pub fn switch_view(&mut self, view: CaseView) {
        self.view_positions[self.view.index()] = (self.row, self.scroll);
        self.view_links[self.view.index()] = self.link;
        self.view = view;
        self.sort_by_date = view == CaseView::Timeline;
        self.center = match view {
            CaseView::Focus | CaseView::Path => CaseCenter::Graph,
            CaseView::Product => CaseCenter::Product,
            _ => CaseCenter::Workbench,
        };
        if self.center != CaseCenter::Graph {
            self.gap_focus = false;
        }
        if self.center != CaseCenter::Workbench {
            self.plan_focus = false;
        }
        self.link = self.view_links[view.index()];
        (self.row, self.scroll) = self.view_positions[view.index()];
        self.source = None;
    }
    pub fn switch_center(&mut self, center: CaseCenter) {
        self.switch_view(match center {
            CaseCenter::Workbench => CaseView::Review,
            CaseCenter::Graph => CaseView::Focus,
            CaseCenter::Product => CaseView::Product,
        });
    }
    pub fn visible_gaps(&self) -> Vec<&argos_osint_core::investigation::Gap> {
        self.data
            .gaps
            .iter()
            .filter(|g| {
                g.open
                    && (self.gaps_case_wide
                        || self.lead_id.as_ref() == Some(&g.entity_id)
                        || g.to
                            .as_ref()
                            .is_some_and(|id| Some(id) == self.lead_id.as_ref())
                        || self.expanded.contains(&g.entity_id)
                        || g.to.as_ref().is_some_and(|id| self.expanded.contains(id))
                        || self.lead_id.is_none())
            })
            .collect()
    }
    pub fn product_findings(&self) -> Vec<&argos_osint_core::evidence::Finding> {
        self.data
            .findings
            .iter()
            .filter(|f| {
                f.decision == Some(argos_osint_core::evidence::ReviewDecision::Accept)
                    && !f.category.starts_with("Candidate")
                    && (self.product_case_wide
                        || Some(&f.observation.entity_id) == self.lead_id.as_ref())
            })
            .collect()
    }
    pub fn jobs(&self) -> Vec<&argos_osint_core::research::ResearchJob> {
        self.data
            .jobs
            .iter()
            .filter(|j| {
                self.global_jobs
                    || self
                        .lead_id
                        .as_ref()
                        .is_none_or(|id| &j.input.entity_id == id)
            })
            .collect()
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
                self.lead_id
                    .as_ref()
                    .is_none_or(|id| &f.observation.entity_id == id)
            })
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
                    || self
                        .filter
                        .split('|')
                        .any(|id| !id.is_empty() && f.observation.id == id)
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
    pub(crate) fn sync_workbench_plan(&mut self) {
        let Some(w) = self.investigation.as_mut() else {
            return;
        };
        let label = w
            .lead()
            .map(|e| e.label.clone())
            .unwrap_or_else(|| w.question.clone());
        w.actions.clear();
        w.disabled_actions.clear();
        let mut configs = self.settings.research.iter().collect::<Vec<_>>();
        configs.sort_by_key(|(name, _)| *name);
        for (name, c) in configs {
            if w.data
                .scope
                .as_ref()
                .is_none_or(|s| !s.allowed_actions.contains(name))
                || argos_osint_core::research::input_matches_action(name, &label).is_err()
            {
                continue;
            }
            let absent = w.data.gaps.iter().any(|g| {
                g.kind == argos_osint_core::investigation::GapKind::CollectedAbsent
                    && g.action.as_ref() == Some(name)
                    && g.input == label
            });
            let reason = if !argos_osint_core::research::collection_available(name) {
                Some("unavailable until contract verified".into())
            } else if absent {
                Some("collected_absent · not a real-world negative finding; history only".into())
            } else if name == "shodan"
                && self
                    .auth
                    .research
                    .get(&c.secret_ref)
                    .is_none_or(|s| s.is_empty())
            {
                Some("credential unavailable".into())
            } else {
                argos_osint_core::research::eligible_action(name, &label, c)
                    .err()
                    .map(|e| e.to_string())
            };
            if let Some(reason) = reason {
                w.disabled_actions.push((name.clone(), reason));
            } else {
                w.actions.push(name.clone());
            }
        }
        w.plan_checked.retain(|a| w.actions.contains(a));
        w.action_sel = w.action_sel.min(w.actions.len().saturating_sub(1));
    }
    pub(super) fn select_investigation_view(&mut self, view: CaseView) {
        if self.pending_case_data.is_some() {
            self.cancel_case_data_plan();
        }
        self.case_source_generation = self.case_source_generation.wrapping_add(1);
        if let Some(w) = self.investigation.as_mut() {
            w.switch_view(view);
            w.plan_focus = false;
            w.gap_focus = false;
            w.inbox_focus = false;
            if view == CaseView::Path {
                self.tna_layout = TnaLayout::Path;
                if w.path_from.is_none() {
                    w.path_from = w.lead_id.clone();
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
        self.desk_cases.remove(&plan.case_id);
        self.desk_generations
            .entry(plan.case_id.clone())
            .and_modify(|g| *g = g.wrapping_add(1))
            .or_insert(1);
        self.desk_refreshing.remove(&plan.case_id);
        let _ = self.reload_lists();
        if !plan.delete_case {
            self.refresh_desk_case(&plan.case_id);
        }
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
            inbox_focus: true,
            view: CaseView::Review,
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
        let mut configs = self.settings.research.clone();
        if let Some(c) = configs.get_mut("shodan") {
            if self
                .auth
                .research
                .get(&c.secret_ref)
                .is_none_or(|s| s.is_empty())
            {
                c.readiness = argos_osint_core::research::Readiness::MissingCredentials;
            }
        }

        if let Some(path) = self.tna_db_path.clone() {
            let tx = self.tx.clone();
            argos_osint_core::workers::spawn_blocking(move || {
                let result = Store::open(&path)
                    .and_then(|s| s.case_projection_with_research(&id, &cancel, &configs))
                    .map_err(|e| e.to_string());
                let _ = tx.send(AppMsg::CaseReady {
                    case_id: id,
                    generation,
                    result,
                });
            });
        } else {
            let result = self
                .store
                .case_projection_with_research(&id, &cancel, &configs)
                .map_err(|e| e.to_string());
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
                .get(w.path_sel)
                .and_then(|p| {
                    p.nodes
                        .windows(2)
                        .nth(w.hop_sel)
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
            let mut chronological = w.clone();
            chronological.sort_by_date = true;
            chronological
                .findings()
                .get(w.row)
                .and_then(|f| f.observation.evidence.first())
                .cloned()
        } else if w.view == CaseView::Product {
            w.product_findings()
                .get(w.row)
                .and_then(|f| f.observation.evidence.first())
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
            if w.view == CaseView::Path {
                w.switch_view(CaseView::Focus);
                return false;
            }
            if w.gap_focus {
                w.gap_focus = false;
                return false;
            }
            if w.plan_focus {
                w.plan_focus = false;
                return false;
            }
            if self.focus == Focus::TableDetail {
                self.focus = Focus::Graph;
                return false;
            }
            self.close_investigation();
            return false;
        }
        if self.focus == Focus::Prompt {
            if key.code == KeyCode::Tab {
                self.focus = Focus::Graph;
                self.investigation.as_mut().unwrap().inbox_focus = true;
                return false;
            }
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
            let w = self.investigation.as_mut().unwrap();
            w.plan_focus = false;
            if w.inbox_focus {
                w.inbox_focus = false;
            } else if self.focus != Focus::TableDetail {
                self.focus = Focus::TableDetail;
            } else {
                self.focus = Focus::Prompt;
            }
            return false;
        }
        if key.code == KeyCode::BackTab {
            self.investigation.as_mut().unwrap().inbox_focus = true;
            self.focus = Focus::Graph;
            return false;
        }
        if let KeyCode::Char(c @ '1'..='7') = key.code {
            if c == '1' {
                self.close_investigation();
            } else {
                self.select_investigation_view(match c {
                    '2' => CaseView::Review,
                    '3' => CaseView::Focus,
                    '4' => CaseView::Product,
                    '5' => CaseView::Review,
                    '6' => CaseView::Jobs,
                    _ => CaseView::Path,
                });
            }
            return false;
        }
        self.sync_workbench_plan();
        if key.code == KeyCode::Char('o') {
            self.selected_case_source();
            return false;
        }
        let paths = self.tna_paths();
        let w = self.investigation.as_mut().unwrap();
        if w.source.is_some() {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => w.scroll += 1,
                KeyCode::Char('k') | KeyCode::Up => w.scroll = w.scroll.saturating_sub(1),
                _ => {}
            }
            return false;
        }
        if key.code == KeyCode::Char('g') {
            if w.gap_focus {
                w.gap_focus = false;
            } else {
                w.switch_center(CaseCenter::Graph);
                w.gap_focus = true;
                w.gaps_case_wide = false;
                w.gap_sel = 0;
                w.inbox_focus = false;
            }
            return false;
        }
        if w.plan_focus {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    w.action_sel = (w.action_sel + 1).min(w.actions.len().saturating_sub(1))
                }
                KeyCode::Char('k') | KeyCode::Up => w.action_sel = w.action_sel.saturating_sub(1),
                KeyCode::Char(' ') => {
                    if let Some(action) = w.actions.get(w.action_sel).cloned() {
                        if w.plan_checked.contains(&action) {
                            w.plan_checked.retain(|a| a != &action);
                        } else {
                            w.plan_checked.push(action);
                        }
                    }
                }
                KeyCode::Enter | KeyCode::Char('e') => {
                    let checked = std::mem::take(&mut w.plan_checked);
                    w.plan_focus = false;
                    for provider in checked {
                        self.submit_enrichment(&provider);
                    }
                }
                _ => {}
            }
            return false;
        }
        if w.view == CaseView::Product && !w.inbox_focus {
            match key.code {
                KeyCode::Char(' ') => {
                    if let Some(id) = w
                        .product_findings()
                        .get(w.row)
                        .map(|f| f.observation.id.clone())
                    {
                        if w.product_checked.contains(&id) {
                            w.product_checked.retain(|x| x != &id);
                        } else {
                            w.product_checked.push(id);
                        }
                    }
                }
                KeyCode::Char('c') => {
                    w.product_case_wide = !w.product_case_wide;
                    w.row = 0;
                }
                KeyCode::Enter => {
                    self.prompt = format!("/draft final {}", w.product_checked.join(" "));
                    self.cursor = self.prompt.chars().count();
                    self.focus = Focus::Prompt;
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    w.row = (w.row + 1).min(w.product_findings().len().saturating_sub(1))
                }
                KeyCode::Char('k') | KeyCode::Up => w.row = w.row.saturating_sub(1),
                _ => {}
            }
            if !matches!(key.code, KeyCode::Char('g') | KeyCode::Char('e')) {
                return false;
            }
        }
        if w.gap_focus {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    w.gap_sel = (w.gap_sel + 1).min(w.visible_gaps().len().saturating_sub(1))
                }
                KeyCode::Char('k') | KeyCode::Up => w.gap_sel = w.gap_sel.saturating_sub(1),
                KeyCode::Enter | KeyCode::Char('e') => {
                    if let Some(gap) = w.visible_gaps().get(w.gap_sel).cloned().cloned() {
                        if gap.kind == argos_osint_core::investigation::GapKind::Uncollected {
                            w.lead_id = Some(gap.entity_id);
                            w.switch_center(CaseCenter::Workbench);
                            w.gap_focus = false;
                            if let Some(action) = gap.action {
                                w.plan_checked = vec![action.clone()];
                                w.actions = vec![action];
                                w.plan_focus = true;
                            }
                        } else if matches!(
                            gap.kind,
                            argos_osint_core::investigation::GapKind::Conflicting
                                | argos_osint_core::investigation::GapKind::Candidate
                        ) {
                            w.lead_id = Some(gap.entity_id);
                            w.filter = gap.observation_ids.join("|");
                            w.switch_view(CaseView::Review);
                            w.gap_focus = false;
                            w.inbox_focus = false;
                            w.row = 0;
                        } else {
                            let history = w
                                .data
                                .jobs
                                .iter()
                                .filter(|j| gap.job_ids.contains(&j.id))
                                .map(|j| {
                                    format!(
                                        "{} · {} {:?}\nInput: {}\n{}\n{}",
                                        j.id,
                                        j.provider,
                                        j.state,
                                        j.input.label,
                                        j.progress,
                                        j.error.as_deref().unwrap_or("")
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            w.source = Some(format!(
                                "{}\n{}\n{history}\nNot a real-world negative finding.",
                                gap.kind.label(),
                                gap.reason
                            ));
                        }
                    }
                }
                KeyCode::Char('g') => w.gap_focus = false,
                _ => {}
            }
            if !matches!(
                key.code,
                KeyCode::Char('x') | KeyCode::Char('z') | KeyCode::Left | KeyCode::Right
            ) {
                return false;
            }
        }
        match key.code {
            KeyCode::Char('f') if matches!(w.view, CaseView::Path | CaseView::Focus) => {
                w.path_from = w.lead_id.clone();
                if w.view == CaseView::Focus {
                    w.switch_view(CaseView::Path);
                }
                w.path_sel = 0;
                w.hop_sel = 0;
            }
            KeyCode::Char('t') if matches!(w.view, CaseView::Path | CaseView::Focus) => {
                w.path_to = w.lead_id.clone();
                if w.view == CaseView::Focus {
                    w.switch_view(CaseView::Path);
                }
                w.path_sel = 0;
                w.hop_sel = 0;
            }
            KeyCode::Char('n') if matches!(w.view, CaseView::Path | CaseView::Focus) => {
                w.path_sel = (w.path_sel + 1).min(paths.len().saturating_sub(1));
                w.hop_sel = 0;
            }
            KeyCode::Char('N') if matches!(w.view, CaseView::Path | CaseView::Focus) => {
                w.path_sel = w.path_sel.saturating_sub(1);
                w.hop_sel = 0;
            }
            KeyCode::Char(']') if w.view == CaseView::Path => {
                w.hop_sel = (w.hop_sel + 1).min(
                    paths
                        .get(w.path_sel)
                        .map(|p| p.nodes.len().saturating_sub(2))
                        .unwrap_or(0),
                );
            }
            KeyCode::Char('[') if w.view == CaseView::Path => {
                w.hop_sel = w.hop_sel.saturating_sub(1)
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if w.inbox_focus || w.view == CaseView::Leads {
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
                        w.row = 0;
                        w.plan_checked.clear();
                        w.actions.clear();
                    }
                } else {
                    if matches!(w.view, CaseView::Focus | CaseView::Path) {
                        w.link = (w.link + 1).min(w.links().len().saturating_sub(1));
                        return false;
                    }
                    let len = match w.view {
                        CaseView::Timeline => w.findings().len(),
                        CaseView::Jobs => w.jobs().len(),
                        CaseView::Evidence => w.evidence_rows().len(),
                        _ => w.findings().len(),
                    };
                    w.row = (w.row + 1).min(len.saturating_sub(1));
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if w.inbox_focus || w.view == CaseView::Leads {
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
                        w.row = 0;
                        w.plan_checked.clear();
                        w.actions.clear();
                    }
                } else {
                    if matches!(w.view, CaseView::Focus | CaseView::Path) {
                        w.link = w.link.saturating_sub(1);
                        return false;
                    }
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
                match w.view {
                    CaseView::Evidence => {
                        w.switch_view(CaseView::Timeline);
                    }
                    CaseView::Timeline => {
                        w.switch_view(CaseView::Review);
                    }
                    _ => {
                        w.switch_view(CaseView::Evidence);
                    }
                }
                w.row = 0;
            }
            KeyCode::Char('J') => {
                w.switch_view(CaseView::Jobs);
                w.inbox_focus = false;
                w.global_jobs = false;
            }
            KeyCode::Char('g') => {
                w.gap_focus = true;
                w.gap_sel = 0;
                w.switch_center(CaseCenter::Graph);
            }
            KeyCode::Char('e') => {
                w.plan_focus = true;
                w.inbox_focus = false;
                w.switch_center(CaseCenter::Workbench);
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
                if let Some(j) = w.jobs().get(w.row) {
                    w.filter = j.id.clone();
                    w.switch_view(CaseView::Review);
                    w.row = 0;
                }
            }
            KeyCode::Char('v') if w.view == CaseView::Focus => {
                if let Some(l) = w.links().get(w.link) {
                    let ids = l.observations.join("|");
                    let lead = w
                        .data
                        .findings
                        .iter()
                        .find(|f| l.observations.contains(&f.observation.id))
                        .map(|f| f.observation.entity_id.clone());
                    w.filter = ids;
                    w.inbox_focus = false;
                    if let Some(lead) = lead {
                        w.lead_id = Some(lead);
                    }
                    w.switch_view(CaseView::Review);
                    w.row = 0;
                }
            }
            KeyCode::Char('p') if matches!(w.view, CaseView::Focus | CaseView::Path) => {
                if w.path_from.is_none() || w.path_to.is_none() {
                    if let Some((from, to)) = w
                        .links()
                        .get(w.link)
                        .map(|l| (l.relationship.from.clone(), l.relationship.to.clone()))
                    {
                        w.path_from = Some(from);
                        w.path_to = Some(to);
                    } else {
                        w.path_from = w.lead_id.clone();
                    }
                }
                w.switch_view(CaseView::Path);
                self.tna_layout = TnaLayout::Path;
            }
            KeyCode::Char('z') => w.expanded.clear(),
            KeyCode::Enter | KeyCode::Char('o') => self.selected_case_source(),
            KeyCode::Char('a') | KeyCode::Char('r') | KeyCode::Char('d') | KeyCode::Char('t')
                if matches!(
                    w.view,
                    CaseView::Review | CaseView::Leads | CaseView::Evidence | CaseView::Timeline
                ) =>
            {
                let selected = if w.view == CaseView::Evidence {
                    w.evidence_rows()
                        .get(w.row)
                        .and_then(|r| r.observation_id.clone())
                } else {
                    w.findings().get(w.row).map(|f| f.observation.id.clone())
                };
                if let Some(id) = selected {
                    let decision = match key.code {
                        KeyCode::Char('a') => "accept",
                        KeyCode::Char('r') => "reject",
                        KeyCode::Char('d') => "defer",
                        _ => "retain",
                    };
                    self.prompt = format!("/review {id} {decision} ");
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
                "Case saved: {}. /case {} opens Inbox + Workbench. Reports can be drafted after review.",
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
        self.status = "Case saved · Inbox + Workbench · e plan · 3 Graph · 4 Product".into();
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
        app.select_investigation_view(CaseView::Review);
        app.select_investigation_view(CaseView::Path);
        assert_eq!(app.tna_paths().len(), 1);
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
    fn three_case_centers_have_clickable_tabs_and_keep_positions() {
        let mut app = fixture();
        for width in [140, 80, 48] {
            render(&mut app, width, 40);
            let hits = app.investigation.as_ref().unwrap().tab_hits.clone();
            assert_eq!(hits.len(), 3);
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
        assert_eq!(app.investigation.as_ref().unwrap().tab_hits.len(), 3);
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
            CaseView::Product,
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
                && (text.contains("does not establish") && text.contains("ownership")),
            "{text}"
        );
        assert!(!text.contains("Strategic"));
    }
    #[test]
    fn desk_opens_highest_priority_review_and_product_prefills_only_selected_ids() {
        let mut app = fixture();
        let id = app.investigation.as_ref().unwrap().case_id.clone();
        let other = app
            .store
            .create_case("Another case without a report")
            .unwrap();
        app.reload_lists().unwrap();
        app.close_investigation();
        app.focus = Focus::Canvas;
        let mut terminal = Terminal::new(TestBackend::new(140, 36)).unwrap();
        terminal
            .draw(|f| super::super::super::ui::draw(f, &mut app))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(
            text.contains("Next Work") && text.contains("Another case"),
            "{text}"
        );
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.investigation.as_ref().unwrap().case_id, id);
        assert_eq!(
            app.investigation.as_ref().unwrap().findings()[0]
                .observation
                .id,
            "o1"
        );
        app.on_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert_eq!(app.prompt, "/review o1 accept ");
        assert_eq!(app.research_active, 0);
        assert!(app.research_jobs.is_empty());
        app.store
            .review_finding("o1", ReviewDecision::Accept, "source verified")
            .unwrap();
        app.refresh_investigation();
        app.select_investigation_view(CaseView::Product);
        app.on_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.prompt, "/draft final o1");
        assert!(app.store.list_reports().unwrap().is_empty());
        assert!(app.cases.iter().any(|c| c.id == other.id));
    }
    #[test]
    fn case_owned_path_and_link_selection_survive_centers_and_escape() {
        let mut app = fixture();
        app.store
            .review_finding("o1", ReviewDecision::Accept, "source verified")
            .unwrap();
        app.refresh_investigation();
        app.select_investigation_view(CaseView::Focus);
        app.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
        let from = app.investigation.as_ref().unwrap().path_from.clone();
        let to = app.investigation.as_ref().unwrap().path_to.clone();
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.investigation.as_ref().unwrap().view, CaseView::Focus);
        app.select_investigation_view(CaseView::Review);
        app.select_investigation_view(CaseView::Focus);
        let w = app.investigation.as_ref().unwrap();
        assert_eq!(w.path_from, from);
        assert_eq!(w.path_to, to);
        assert_eq!(app.tna_paths().len(), 1);
        assert!(app.tna_from.is_none() && app.tna_to.is_none());
    }
    #[test]
    fn plan_starts_unchecked_and_hole_selection_checks_without_collecting() {
        let mut app = fixture();
        let case = app.investigation.as_ref().unwrap().case_id.clone();
        let scope = InvestigationScope {
            question: "harbor.example".into(),
            allowed_actions: vec!["domain".into(), "katana".into()],
            ..Default::default()
        };
        app.store
            .put_record("scope", "investigation_scope", Some(&case), None, &scope)
            .unwrap();
        app.refresh_investigation();
        app.sync_workbench_plan();
        assert!(app.investigation.as_ref().unwrap().plan_checked.is_empty());
        assert!(app
            .investigation
            .as_ref()
            .unwrap()
            .disabled_actions
            .iter()
            .any(|(name, _)| name == "katana"));
        app.on_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        let gap = app
            .investigation
            .as_ref()
            .unwrap()
            .visible_gaps()
            .iter()
            .position(|g| g.kind == argos_osint_core::investigation::GapKind::Uncollected)
            .unwrap();
        app.investigation.as_mut().unwrap().gap_sel = gap;
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let w = app.investigation.as_ref().unwrap();
        assert!(w.plan_focus);
        assert_eq!(w.plan_checked, vec!["domain"]);
        assert_eq!(app.research_active, 0);
        assert!(app.research_jobs.is_empty());
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(app.investigation.as_ref().unwrap().plan_checked.is_empty());
    }
    #[test]
    fn source_inspector_keeps_inbox_and_ctrl_c_leaves_jobs_running() {
        let mut app = fixture();
        app.selected_case_source();
        let text = render(&mut app, 100, 30);
        assert!(text.contains("Lead inbox") && text.contains("Original source"));
        app.research_active = 1;
        app.prompt.clear();
        app.focus = Focus::Graph;
        assert!(!app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert!(!app.quit);
        assert_eq!(app.research_active, 1);
    }
    #[tokio::test]
    async fn checked_plan_queues_separate_bounded_jobs_with_exact_lead_input() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("argos.db");
        let store = Store::open(&path).unwrap();
        let case = store.create_case("Harbor").unwrap();
        let entity = normalize_entity("harbor.example", EntityType::Domain).unwrap();
        store
            .put_record(&entity.id, "entity", Some(&case.id), None, &entity)
            .unwrap();
        let scope = InvestigationScope {
            question: entity.label.clone(),
            allowed_actions: vec!["domain".into(), "search".into()],
            ..Default::default()
        };
        store
            .put_record("scope", "investigation_scope", Some(&case.id), None, &scope)
            .unwrap();
        let mut app = App::from_parts(store, SettingsFile::default(), AuthFile::default()).unwrap();
        let mut inbox = app.take_inbox();
        app.open_investigation(&case.id);
        app.sync_workbench_plan();
        let queue = argos_osint_core::research::ResearchQueue::new(path, 2);
        // Cancel before execution so the real queue persists bounded job records without HTTP.
        queue.cancel.store(true, Ordering::Relaxed);
        app.research_queue = Some(queue);
        app.on_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        {
            let w = app.investigation.as_mut().unwrap();
            w.plan_checked = vec!["domain".into(), "search".into()];
        }
        app.on_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        assert_eq!(app.research_active, 2);
        let mut completed = 0;
        while completed < 2 {
            let msg = tokio::time::timeout(std::time::Duration::from_secs(5), inbox.recv())
                .await
                .unwrap()
                .unwrap();
            if matches!(msg, AppMsg::ResearchJob(_)) {
                completed += 1;
            }
            app.on_msg(msg);
        }
        let jobs = app.store.jobs_for_case(&case.id).unwrap();
        assert_eq!(jobs.len(), 2);
        assert!(jobs.iter().all(|j| j.input.entity_id == entity.id
            && j.input.label == entity.label
            && j.input.depth == 0));
        assert_ne!(jobs[0].id, jobs[1].id);
        assert_eq!(app.research_active, 0);
        assert_eq!(app.investigation.as_ref().unwrap().view, CaseView::Review);
    }
    #[test]
    fn case_expand_shortcut_and_relationship_table_use_cached_supported_sources() {
        let mut app = fixture();
        app.on_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
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
    pub observation_id: Option<String>,
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
            .filter(|f| {
                self.lead_id
                    .as_ref()
                    .is_none_or(|id| &f.observation.entity_id == id)
            })
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
                    observation_id: Some(o.id.clone()),
                }
            })
            .collect::<Vec<_>>();
        rows.extend(self.links().into_iter().map(|l| {
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
                observation_id: o.map(|o| o.id.clone()),
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
