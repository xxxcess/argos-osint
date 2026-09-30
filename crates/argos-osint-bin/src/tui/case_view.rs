//! Progressive case views. Rendering consumes only cached evidence.
use super::*;
use crate::tui::app::case_workspace::{CaseView, CaseWorkspace};
use argos_osint_core::evidence::ReviewDecision;

/// The open case owns the same bordered header area as the regular Desk.
pub(super) fn under_tabs(frame: &mut Frame, app: &mut App, area: Rect) -> Rect {
    app.case_tab_hits.clear();
    let Some(w) = app.investigation.as_ref() else {
        return area;
    };
    let active = match w.view {
        CaseView::Focus | CaseView::Path => CaseView::Focus,
        CaseView::Product => CaseView::Product,
        _ => CaseView::Review,
    };
    let groups = tab_groups(area.width.saturating_sub(2));
    let rows = split_v(
        area,
        &[
            Constraint::Length(groups.len() as u16 + 2),
            Constraint::Min(0),
        ],
    );
    app.case_tab_area = rows[0];
    let title = app
        .cases
        .iter()
        .find(|c| c.id == w.case_id)
        .map(|c| c.title.as_str())
        .unwrap_or(&w.question);
    let title = format!(" {title}{} ", if w.loading { " · refreshing" } else { "" });
    let block = panel(&title);
    let inner = block.inner(rows[0]);
    frame.render_widget(block, rows[0]);
    let mut hits = Vec::new();
    for (row, group) in groups.iter().enumerate() {
        let y = inner.y.saturating_add(row as u16);
        if y >= inner.bottom() {
            break;
        }
        frame.render_widget(
            Tabs::new(
                group
                    .iter()
                    .map(|(_, label)| Line::from(label.clone()))
                    .collect::<Vec<_>>(),
            )
            .select(
                group
                    .iter()
                    .position(|(view, _)| *view == active)
                    .unwrap_or(usize::MAX),
            )
            .style(theme::dim())
            .highlight_style(theme::selected())
            .divider(" │ "),
            Rect::new(inner.x, y, inner.width, 1),
        );
        let mut x = inner.x;
        for (view, label) in group {
            let width = (label.width() as u16 + 2).min(inner.right().saturating_sub(x));
            if width > 0 {
                hits.push((*view, Rect::new(x, y, width, 1)));
            }
            x = x.saturating_add(width + 3);
        }
    }
    app.investigation.as_mut().unwrap().tab_hits = hits;
    rows[1]
}

pub(super) fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    app.sync_workbench_plan();
    app.report_area = Rect::default();
    app.report_line_index.clear();
    app.tna_tab_hits.clear();
    app.tna_table_list_area = Rect::default();
    app.tna_table_detail_area = Rect::default();
    app.canvas_area = area;
    let rows = split_v(area, &[Constraint::Length(1), Constraint::Min(0)]);
    frame.render_widget(
        Paragraph::new("1 Desk · Tab inbox/center/inspector/composer · e plan · g gaps · D data")
            .style(theme::dim()),
        rows[0],
    );
    let w = app.investigation.as_ref().unwrap();
    let regions = if rows[1].width >= 80 {
        split_h(
            rows[1],
            &[Constraint::Percentage(28), Constraint::Percentage(72)],
        )
    } else {
        split_v(rows[1], &[Constraint::Length(7), Constraint::Min(0)])
    };
    inbox(frame, w, regions[0]);
    if let Some(source) = &w.source {
        frame.render_widget(
            Paragraph::new(source.clone())
                .scroll((w.scroll.min(u16::MAX as usize) as u16, 0))
                .wrap(Wrap { trim: false })
                .block(panel(
                    " Original source / job history · j/k scroll · Esc back ",
                )),
            regions[1],
        );
        return;
    }

    match w.view {
        CaseView::Focus => focus(frame, w, regions[1]),
        CaseView::Path => {
            let graph = split_v(
                regions[1],
                &[Constraint::Percentage(45), Constraint::Percentage(55)],
            );
            focus(frame, w, graph[0]);
            case_path(frame, app, graph[1]);
        }
        CaseView::Product => product(frame, w, regions[1]),
        _ => workbench(frame, w, regions[1]),
    }
}

fn tab_groups(_width: u16) -> Vec<Vec<(CaseView, String)>> {
    vec![vec![
        (CaseView::Review, "2 Work".into()),
        (CaseView::Focus, "3 Graph".into()),
        (CaseView::Product, "4 Product".into()),
    ]]
}

fn case_path(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(w) = app.investigation.as_ref() else {
        return;
    };
    let paths = app.tna_paths();
    let mut lines = vec![
        Line::from("Explore connections after reviewing findings").style(theme::accent()),
        Line::from("Accepted supported links only · up to 4 hops · no research runs here")
            .style(theme::dim()),
        Line::from(format!(
            "From: {}",
            w.path_from
                .as_deref()
                .map(|id| name(w, id))
                .unwrap_or_else(|| "select entity and press f".into())
        )),
        Line::from(format!(
            "To:   {}",
            w.path_to
                .as_deref()
                .map(|id| name(w, id))
                .unwrap_or_else(|| "select entity and press t".into())
        )),
        Line::from("Inbox j/k · f from · t to · n/N path · [/] hop · o source · Esc map")
            .style(theme::dim()),
        Line::from(format!(
            "Selected entity: {}",
            w.lead().map(|e| e.label.as_str()).unwrap_or("none")
        ))
        .style(theme::accent()),
    ];
    if paths.is_empty() {
        lines.push(Line::from(if w.path_from.is_none() || w.path_to.is_none() {"Choose two entities to explore their reviewed evidence."} else {"No supported path found within the search limits. This is not evidence of a real-world gap. No recovered path is not proof of a real-world gap."}));
    }
    for (i, path) in paths.iter().enumerate() {
        lines.push(
            Line::from(format!(
                "{} {}",
                if i == w.path_sel { "›" } else { " " },
                path.nodes
                    .iter()
                    .map(|id| name(w, id))
                    .collect::<Vec<_>>()
                    .join(" → ")
            ))
            .style(if i == w.path_sel {
                theme::selected()
            } else {
                theme::text()
            }),
        );
    }
    if let Some(pair) = paths
        .get(w.path_sel)
        .and_then(|p| p.nodes.windows(2).nth(w.hop_sel))
    {
        for link in w.data.links.iter().filter(|l| {
            !l.candidate
                && ((l.relationship.from == pair[0] && l.relationship.to == pair[1])
                    || (l.relationship.from == pair[1] && l.relationship.to == pair[0]))
        }) {
            let r = &link.relationship;
            lines.push(
                Line::from(format!(
                    "\nHop {}: {} · {}",
                    w.hop_sel + 1,
                    r.kind.label(),
                    r.basis.label()
                ))
                .style(theme::accent()),
            );
            lines.push(Line::from(format!(
                "Uncertainty: {}",
                if r.uncertainty.is_empty() {
                    "not supplied"
                } else {
                    &r.uncertainty
                }
            )));
            for reference in &r.evidence {
                lines.push(
                    Line::from(format!(
                        "Source: {}",
                        reference
                            .passage_id
                            .as_ref()
                            .or(reference.artifact_id.as_ref())
                            .or(reference.source_url.as_ref())
                            .map(String::as_str)
                            .unwrap_or("unavailable")
                    ))
                    .style(theme::dim()),
                );
            }
        }
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((w.scroll.min(u16::MAX as usize) as u16, 0))
            .block(panel(" Path · reviewed case evidence ")),
        area,
    );
}
fn name(w: &CaseWorkspace, id: &str) -> String {
    w.data
        .entities
        .iter()
        .find(|e| e.id == id)
        .map(|e| e.label.clone())
        .unwrap_or_else(|| id.to_string())
}
fn inbox(frame: &mut Frame, w: &CaseWorkspace, area: Rect) {
    let selected = w
        .data
        .entities
        .iter()
        .position(|e| Some(&e.id) == w.lead_id.as_ref())
        .unwrap_or(0);
    let visible = ((area.height.saturating_sub(2) / 3).max(1) as usize).min(w.lead_limit.max(1));
    let mut lines = Vec::new();
    for e in w
        .data
        .entities
        .iter()
        .skip(selected.saturating_sub(visible.saturating_sub(1)))
        .take(visible)
    {
        let accepted = w
            .data
            .findings
            .iter()
            .filter(|f| {
                f.observation.entity_id == e.id && f.decision == Some(ReviewDecision::Accept)
            })
            .count();
        lines.push(
            Line::from(format!(
                "{} {} · {:?}",
                if Some(&e.id) == w.lead_id.as_ref() {
                    "›"
                } else {
                    " "
                },
                e.label,
                e.kind
            ))
            .style(if Some(&e.id) == w.lead_id.as_ref() {
                theme::selected()
            } else {
                theme::text()
            }),
        );
        lines.push(Line::from(format!("why: {}", w.data.why_now(&e.id).1)).style(theme::dim()));
        lines.push(
            Line::from(format!(
                "review {} · accepted {accepted} · links {}",
                w.data.pending_for(&e.id),
                w.data.accepted_degree(&e.id)
            ))
            .style(theme::dim()),
        );
    }
    if lines.is_empty() {
        lines.push(Line::from("No leads yet · plan uses the case question"));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel(if w.inbox_focus {
                " Lead inbox · j/k · focused "
            } else {
                " Lead inbox · Shift+Tab focus "
            })),
        area,
    );
}
fn workbench(frame: &mut Frame, w: &CaseWorkspace, area: Rect) {
    let rows = split_v(
        area,
        &[
            Constraint::Length(
                (w.actions.len() + w.disabled_actions.len() + 4).clamp(5, 10) as u16,
            ),
            Constraint::Min(4),
            Constraint::Length(5),
        ],
    );
    let mut plan = vec![Line::from(
        "Space checks · e runs checked set as separate bounded jobs · no auto-check",
    )
    .style(theme::dim())];
    for (i, action) in w
        .actions
        .iter()
        .enumerate()
        .skip(if w.plan_focus {
            w.action_sel.saturating_sub(2)
        } else {
            0
        })
        .take(3)
    {
        plan.push(
            Line::from(format!(
                "{} [{}] {action} · ready · {}",
                if w.plan_focus && i == w.action_sel {
                    "›"
                } else {
                    " "
                },
                if w.plan_checked.contains(action) {
                    "x"
                } else {
                    " "
                },
                argos_osint_core::research::capabilities(action)
            ))
            .style(if w.plan_focus && i == w.action_sel {
                theme::selected()
            } else {
                theme::text()
            }),
        );
    }
    for (action, reason) in w.disabled_actions.iter().take(1) {
        plan.push(Line::from(format!("[ ] {action} · {reason}")).style(theme::dim()));
    }
    for j in w
        .data
        .jobs
        .iter()
        .filter(|j| Some(&j.input.entity_id) == w.lead_id.as_ref() || w.lead_id.is_none())
        .take(2)
    {
        plan.push(
            Line::from(format!("{} {:?} · {}", j.provider, j.state, j.progress))
                .style(theme::dim()),
        );
    }
    if w.actions.is_empty() && w.disabled_actions.is_empty() {
        plan.push(Line::from("No eligible in-scope actions"));
    }
    frame.render_widget(
        Paragraph::new(plan)
            .wrap(Wrap { trim: false })
            .block(panel(" Plan · e focus · Selecting a lead never collects. ")),
        rows[0],
    );
    match w.view {
        CaseView::Jobs => jobs(frame, w, rows[1]),
        CaseView::Evidence => evidence_table(frame, w, rows[1]),
        CaseView::Timeline => timeline(frame, w, rows[1]),
        _ => evidence(frame, w, rows[1]),
    }
    let mut summary = Vec::new();
    if let Some(e) = w.lead() {
        summary.push(Line::from(format!(
            "{} · aliases {} · accepted links {} · candidate {}",
            e.canonical,
            e.aliases.join(", "),
            w.data.accepted_degree(&e.id),
            w.data
                .links
                .iter()
                .filter(
                    |l| l.candidate && (l.relationship.from == e.id || l.relationship.to == e.id)
                )
                .count()
        )));
    }
    for f in w
        .findings()
        .iter()
        .filter(|f| f.decision == Some(ReviewDecision::Accept))
        .take(1)
    {
        summary.push(Line::from(f.observation.statement.clone()));
    }
    for g in w.visible_gaps().iter().take(1) {
        summary.push(Line::from(format!("○ {} · g Graph holes", g.reason)).style(theme::dim()));
    }
    summary.push(
        Line::from("Discovered identifiers become candidate leads without recursive collection.")
            .style(theme::dim()),
    );
    frame.render_widget(
        Paragraph::new(summary)
            .wrap(Wrap { trim: false })
            .block(panel(" So what ")),
        rows[2],
    );
}
fn product(frame: &mut Frame, w: &CaseWorkspace, area: Rect) {
    let mut lines = vec![Line::from(format!(
        "{} · c lead/case · Space select IDs · Enter prefills /draft final",
        if w.product_case_wide {
            "Case-wide"
        } else {
            "Selected lead"
        }
    ))
    .style(theme::dim())];
    for (i, f) in w
        .product_findings()
        .iter()
        .enumerate()
        .skip(w.row.saturating_sub(3))
        .take(8)
    {
        let o = &f.observation;
        lines.push(
            Line::from(format!(
                "{} [{}] {} · {}",
                if i == w.row { "›" } else { " " },
                if w.product_checked.contains(&o.id) {
                    "x"
                } else {
                    " "
                },
                o.id,
                o.statement
            ))
            .style(if i == w.row {
                theme::selected()
            } else {
                theme::text()
            }),
        );
    }
    lines.push(Line::from("Pending gaps remain unresolved:").style(theme::accent()));
    for g in w.data.gaps.iter().filter(|g| g.open).take(6) {
        lines.push(Line::from(format!("○ {} · {}", g.kind.label(), g.reason)).style(theme::dim()));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel(
                " Product · accepted IDs only · choose final/addendum/revision/followup ",
            )),
        area,
    );
}

fn focus(frame: &mut Frame, w: &CaseWorkspace, area: Rect) {
    let regions = if area.width >= 60 {
        split_h(
            area,
            &[Constraint::Percentage(60), Constraint::Percentage(40)],
        )
    } else {
        split_v(
            area,
            &[Constraint::Percentage(60), Constraint::Percentage(40)],
        )
    };
    let links = w.links();
    let mut lines = Vec::new();
    for (i, l) in
        links
            .iter()
            .enumerate()
            .skip(w.link.saturating_sub(
                (regions[0].height.saturating_sub(5) / 3).saturating_sub(1) as usize,
            ))
            .take(if w.gap_focus {
                0
            } else {
                (regions[0].height.saturating_sub(5) / 3).max(1) as usize
            })
    {
        let r = &l.relationship;
        let style = if l.candidate {
            Style::default().fg(theme::WARN)
        } else {
            theme::accent()
        };
        lines.push(
            Line::from(format!(
                "{} {} {} {}",
                if i == w.link { "›" } else { " " },
                name(w, &r.from),
                if l.candidate { "- - →" } else { "──→" },
                name(w, &r.to)
            ))
            .style(style),
        );
        lines.push(
            Line::from(format!(
                "  {} · {} · {}",
                r.kind.label(),
                r.basis.label(),
                if l.candidate { "candidate" } else { "accepted" }
            ))
            .style(style),
        );
    }
    let hole_rows = (regions[0].height.saturating_sub(4) / 3).max(1) as usize;
    for (i, gap) in w
        .visible_gaps()
        .iter()
        .enumerate()
        .skip(if w.gap_focus {
            w.gap_sel.saturating_sub(hole_rows.saturating_sub(1))
        } else {
            0
        })
        .take(if w.gap_focus { hole_rows } else { 1 })
    {
        let style = match gap.kind {
            argos_osint_core::investigation::GapKind::Conflicting => {
                Style::default().fg(theme::RED)
            }
            argos_osint_core::investigation::GapKind::CollectedAbsent => theme::dim(),
            _ => Style::default().fg(theme::WARN),
        };
        lines.push(
            Line::from(format!(
                "{} ○ {} · {}",
                if w.gap_focus && i == w.gap_sel {
                    "›"
                } else {
                    " "
                },
                name(w, &gap.entity_id),
                gap.reason
            ))
            .style(if w.gap_focus && i == w.gap_sel {
                theme::selected()
            } else {
                style
            }),
        );
    }
    if lines.is_empty() {
        lines.push(Line::from(
            "No supported links. Enrich a selected lead, then review the observations.",
        ));
    }
    lines.push(
        Line::from("Missing or uncollected evidence does not establish a real-world gap.")
            .style(theme::dim()),
    );
    if lines.is_empty() {
        lines.push(Line::from(
            "No supported links. Enrich a selected lead, then review the observations.",
        ));
    }
    lines.push(Line::from("x expand · z collapse · p Path · ←/→ link").style(theme::dim()));
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel(" Immediate relationships · limit 16 ")),
        regions[0],
    );
    let mut detail = Vec::new();
    if w.gap_focus {
        if let Some(g) = w.visible_gaps().get(w.gap_sel) {
            detail.push(Line::from(format!(
                "{}\n{}\nJobs {}\nObservations {}",
                g.kind.label(),
                g.reason,
                g.job_ids.join(", "),
                g.observation_ids.join(", ")
            )));
        }
    }
    if let Some(l) = links.get(w.link).filter(|_| !w.gap_focus) {
        let r = &l.relationship;
        detail.push(Line::from(format!(
            "Selected: {} link",
            if l.candidate { "candidate" } else { "reviewed" }
        )));
        detail.push(Line::from(format!(
            "{} · {}",
            r.kind.label(),
            r.basis.label()
        )));
        detail.push(Line::from(r.uncertainty.clone()));
        for source in &r.evidence {
            detail.push(Line::from(
                source
                    .passage_id
                    .as_ref()
                    .or(source.artifact_id.as_ref())
                    .cloned()
                    .unwrap_or_else(|| "Source unavailable".into()),
            ));
        }
        detail.push(Line::from(format!("Review: {}", l.observations.join(", "))));
        for f in w
            .data
            .findings
            .iter()
            .filter(|f| l.observations.contains(&f.observation.id))
            .take(2)
        {
            detail.push(Line::from(f.observation.statement.clone()));
        }
    } else if let Some(e) = w.lead() {
        detail.push(Line::from(format!(
            "{}\n{:?}\nAliases: {}",
            e.canonical,
            e.kind,
            e.aliases.join(", ")
        )));
    }
    detail.push(
        Line::from("o opens source · v review with reason\ne enriches selected lead")
            .style(theme::dim()),
    );
    frame.render_widget(
        Paragraph::new(detail)
            .wrap(Wrap { trim: false })
            .block(panel(" Evidence inspector ")),
        regions[1],
    );
}
fn evidence(frame: &mut Frame, w: &CaseWorkspace, area: Rect) {
    if w.view == CaseView::Evidence {
        evidence_table(frame, w, area);
        return;
    }
    let rows = w.findings();
    let visible = (area.height.saturating_sub(3) / 4).max(1) as usize;
    let start = w.row.saturating_sub(visible.saturating_sub(1));
    let mut lines = vec![Line::from(format!(
        "Filter {} · sort {} · /filter <source/text/state> · s sort",
        w.filter,
        if w.sort_by_date {
            "retrieval"
        } else {
            "source history"
        }
    ))
    .style(theme::dim())];
    for (i, f) in rows.iter().enumerate().skip(start).take(visible) {
        let o = &f.observation;
        lines.push(
            Line::from(format!(
                "{} {} · {:?} · {}\n{} · event {} · retrieved {}\n{}",
                if i == w.row { "›" } else { " " },
                o.id,
                f.decision,
                f.category,
                name(w, &o.entity_id),
                o.event_time.as_deref().unwrap_or("undated"),
                o.retrieved_at,
                o.statement
            ))
            .style(if i == w.row {
                theme::selected()
            } else {
                theme::text()
            }),
        );
    }
    if rows.is_empty() {
        lines.push(Line::from(
            "No observations. Research starts only from an explicit selected action.",
        ));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel(if w.view == CaseView::Review {
                " Intake · a accept · r reject · d defer · t retain · reason required · o source "
            } else {
                " Intake · a/r/d/t with reason · o source "
            })),
        area,
    );
}
fn timeline(frame: &mut Frame, w: &CaseWorkspace, area: Rect) {
    let mut chronological = w.clone();
    chronological.sort_by_date = true;
    chronological.view = CaseView::Review;
    evidence(frame, &chronological, area);
}

fn jobs(frame: &mut Frame, w: &CaseWorkspace, area: Rect) {
    let mut lines = Vec::new();
    for (i, j) in w
        .jobs()
        .iter()
        .enumerate()
        .skip(w.row.saturating_sub(1))
        .take(8)
    {
        lines.push(Line::from(format!(
            "{} {} · {} · {:?}\nInput: {} · {}\n{}\nError: {} · observations {}",
            if i == w.row { "›" } else { " " },
            j.id,
            j.provider,
            j.state,
            j.input.label,
            j.input.action,
            j.progress,
            j.error.as_deref().unwrap_or("none"),
            j.hits.len()
        )));
    }
    if lines.is_empty() {
        lines.push(Line::from("No jobs. e offers eligible focused actions."));
    }
    lines.push(
        Line::from("/cancel-jobs cancels queued/running work · 5 opens resulting observations")
            .style(theme::dim()),
    );
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel(" Research jobs · saved results survive failures ")),
        area,
    );
}

fn evidence_table(frame: &mut Frame, w: &CaseWorkspace, area: Rect) {
    let rows = w.evidence_rows();
    let visible = (area.height.saturating_sub(4) / 5).max(1) as usize;
    let mut lines = vec![Line::from(format!(
        "Sort {} (s) · /filter entity/type/link/source/date/state",
        crate::tui::app::case_workspace::EVIDENCE_SORTS[w.evidence_sort]
    ))
    .style(theme::dim())];
    for (i, row) in rows
        .iter()
        .enumerate()
        .skip(w.row.saturating_sub(visible.saturating_sub(1)))
        .take(visible)
    {
        lines.push(
            Line::from(format!(
                "{} {} · {}\n{} · {}\nevent {} · retrieved {}\n{}",
                if i == w.row { "›" } else { " " },
                row.entity,
                row.relationship,
                row.state,
                row.source,
                row.event.as_deref().unwrap_or("undated"),
                row.retrieved,
                row.statement
            ))
            .style(if i == w.row {
                theme::selected()
            } else {
                theme::text()
            }),
        );
    }
    if rows.is_empty() {
        lines.push(Line::from(
            "No matching evidence rows. /filter clears the filter; e offers focused research.",
        ));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel(
                " Evidence table · j/k select · o original source · s sort ",
            )),
        area,
    );
}
