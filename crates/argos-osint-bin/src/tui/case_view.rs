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
    let active = w.view;
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
    app.report_area = Rect::default();
    app.report_line_index.clear();
    app.tna_tab_hits.clear();
    app.tna_table_list_area = Rect::default();
    app.tna_table_detail_area = Rect::default();
    app.canvas_area = area;
    let rows = split_v(area, &[Constraint::Length(1), Constraint::Min(0)]);
    frame.render_widget(
        Paragraph::new("D Case data · 1–7 select tab · e enrich · o source").style(theme::dim()),
        rows[0],
    );
    let w = app.investigation.as_ref().unwrap();
    if let Some(source) = &w.source {
        frame.render_widget(
            Paragraph::new(source.clone())
                .scroll((w.scroll.min(u16::MAX as usize) as u16, 0))
                .wrap(Wrap { trim: false })
                .block(panel(" Original source · j/k scroll · Esc back ")),
            rows[1],
        );
        return;
    }
    if !w.actions.is_empty() {
        let items = w
            .actions
            .iter()
            .enumerate()
            .map(|(i, name)| {
                Line::from(format!(
                    "{} {} · {}",
                    if i == w.action_sel { "›" } else { " " },
                    name,
                    argos_osint_core::research::capabilities(name)
                ))
                .style(if i == w.action_sel {
                    theme::selected()
                } else {
                    theme::text()
                })
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(items)
                .wrap(Wrap { trim: false })
                .block(panel(
                    " Enrich selection · chosen action only · Enter submit · Esc cancel ",
                )),
            rows[1],
        );
        return;
    }
    match w.view {
        CaseView::Leads => leads(frame, w, rows[1]),
        CaseView::Focus => focus(frame, w, rows[1]),
        CaseView::Evidence | CaseView::Review => evidence(frame, w, rows[1]),
        CaseView::Timeline => timeline(frame, w, rows[1]),
        CaseView::Jobs => jobs(frame, w, rows[1]),
        CaseView::Path => case_path(frame, app, rows[1]),
    }
}

fn tab_groups(width: u16) -> Vec<Vec<(CaseView, String)>> {
    let mut groups = vec![Vec::new()];
    let mut used = 0;
    for (i, view) in CaseView::ALL.iter().enumerate() {
        let title = if width < 55 {
            ["Leads", "Focus", "Table", "Time", "Review", "Jobs", "Path"][i]
        } else {
            view.title()
        };
        let label = format!("{} {title}", i + 1);
        let wanted = label.width() as u16 + 2;
        let gap = if used > 0 { 3 } else { 0 };
        if used > 0 && used + gap + wanted > width {
            groups.push(Vec::new());
            used = 0;
        }
        if used > 0 {
            used += 3;
        }
        used += wanted;
        groups.last_mut().unwrap().push((*view, label));
    }
    groups
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
            app.tna_from
                .as_deref()
                .map(|id| name(w, id))
                .unwrap_or_else(|| "select entity and press f".into())
        )),
        Line::from(format!(
            "To:   {}",
            app.tna_to
                .as_deref()
                .map(|id| name(w, id))
                .unwrap_or_else(|| "select entity and press t".into())
        )),
        Line::from("j/k entity · f from · t to · n/N path · [/] hop · o source · 2 Focus")
            .style(theme::dim()),
        Line::from(format!(
            "Selected entity: {}",
            w.lead().map(|e| e.label.as_str()).unwrap_or("none")
        ))
        .style(theme::accent()),
    ];
    if paths.is_empty() {
        lines.push(Line::from(if app.tna_from.is_none() || app.tna_to.is_none() {"Choose two entities to explore their reviewed evidence."} else {"No supported path found within the search limits. This is not evidence of a real-world gap."}));
    }
    for (i, path) in paths.iter().enumerate() {
        lines.push(
            Line::from(format!(
                "{} {}",
                if i == app.tna_path_sel { "›" } else { " " },
                path.nodes
                    .iter()
                    .map(|id| name(w, id))
                    .collect::<Vec<_>>()
                    .join(" → ")
            ))
            .style(if i == app.tna_path_sel {
                theme::selected()
            } else {
                theme::text()
            }),
        );
    }
    if let Some(pair) = paths
        .get(app.tna_path_sel)
        .and_then(|p| p.nodes.windows(2).nth(app.tna_hop_sel))
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
                    app.tna_hop_sel + 1,
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
fn leads(frame: &mut Frame, w: &CaseWorkspace, area: Rect) {
    let cols = if area.width >= 80 {
        split_h(
            area,
            &[Constraint::Percentage(45), Constraint::Percentage(55)],
        )
    } else {
        split_v(
            area,
            &[Constraint::Percentage(50), Constraint::Percentage(50)],
        )
    };
    let selected = w
        .data
        .entities
        .iter()
        .position(|e| Some(&e.id) == w.lead_id.as_ref())
        .unwrap_or(0);
    let start = selected.saturating_sub(w.lead_limit.saturating_sub(1));
    let mut lines = Vec::new();
    for e in w.data.entities.iter().skip(start).take(w.lead_limit.max(1)) {
        let evidence = argos_osint_core::investigation::distinct_findings(
            w.data
                .findings
                .iter()
                .filter(|f| f.observation.entity_id == e.id),
        );
        let accepted =
            argos_osint_core::investigation::distinct_findings(w.data.findings.iter().filter(
                |f| f.observation.entity_id == e.id && f.decision == Some(ReviewDecision::Accept),
            ));
        let degree = w
            .data
            .links
            .iter()
            .filter(|l| l.reviewed && (l.relationship.from == e.id || l.relationship.to == e.id))
            .count();
        lines.push(
            Line::from(format!(
                "{} {}",
                if Some(&e.id) == w.lead_id.as_ref() {
                    "›"
                } else {
                    " "
                },
                e.label
            ))
            .style(if Some(&e.id) == w.lead_id.as_ref() {
                theme::selected()
            } else {
                theme::text()
            }),
        );
        lines.push(
            Line::from(format!(
                "  {:?} · evidence {evidence} · accepted {accepted} · links {degree}",
                e.kind
            ))
            .style(theme::dim()),
        );
    }
    if lines.is_empty() {
        lines.push(Line::from("No entity leads yet. /investigate search <focused question> collects only after selection."));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel(
                " Ranked leads · reviewed evidence first · j/k select · e enrich ",
            )),
        cols[0],
    );
    let mut lines = vec![Line::from(format!(
        "Pending review: {}",
        w.data
            .findings
            .iter()
            .filter(|f| f.decision.is_none() || f.decision == Some(ReviewDecision::Defer))
            .count()
    ))];
    for f in w.data.findings.iter().rev().take(3) {
        lines.push(Line::from(format!(
            "{} · {}",
            f.category, f.observation.statement
        )));
    }
    lines.push(
        Line::from(
            "Unresolved: missing or uncollected evidence does not establish a real-world gap.",
        )
        .style(theme::dim()),
    );
    for j in w.data.jobs.iter().rev().take(2) {
        lines.push(Line::from(format!(
            "{} {:?} · {}",
            j.provider, j.state, j.progress
        )));
    }
    if let Some(e) = w.lead() {
        lines.push(Line::from(format!(
            "Selected: {} · {:?}\nAliases: {}",
            e.canonical,
            e.kind,
            e.aliases.join(", ")
        )));
    }
    lines.push(
        Line::from("/draft final <accepted observation IDs> creates a report when chosen.")
            .style(theme::dim()),
    );
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel(" Recent changes · review · gaps · jobs ")),
        cols[1],
    );
}
fn focus(frame: &mut Frame, w: &CaseWorkspace, area: Rect) {
    let regions = if area.width >= 80 {
        split_h(
            area,
            &[
                Constraint::Percentage(25),
                Constraint::Percentage(40),
                Constraint::Percentage(35),
            ],
        )
    } else if area.width >= 60 {
        let rows = split_v(
            area,
            &[Constraint::Percentage(55), Constraint::Percentage(45)],
        );
        let top = split_h(
            rows[0],
            &[Constraint::Percentage(35), Constraint::Percentage(65)],
        );
        vec![top[0], top[1], rows[1]]
    } else {
        split_v(
            area,
            &[
                Constraint::Length(4),
                Constraint::Percentage(40),
                Constraint::Min(0),
            ],
        )
    };
    let index = w
        .data
        .entities
        .iter()
        .position(|e| Some(&e.id) == w.lead_id.as_ref())
        .unwrap_or(0);
    let visible = regions[0].height.saturating_sub(2).max(1) as usize;
    let list = w
        .data
        .entities
        .iter()
        .skip(index.saturating_sub(visible.saturating_sub(1)))
        .take(visible)
        .map(|e| {
            Line::from(format!(
                "{} {}",
                if Some(&e.id) == w.lead_id.as_ref() {
                    "›"
                } else {
                    " "
                },
                e.label
            ))
            .style(if Some(&e.id) == w.lead_id.as_ref() {
                theme::selected()
            } else {
                theme::text()
            })
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(if list.is_empty() {
            vec![Line::from("No leads yet")]
        } else {
            list
        })
        .block(panel(" Entities · j/k ")),
        regions[0],
    );
    let links = w.links();
    let mut lines = Vec::new();
    for (i, l) in links.iter().enumerate().skip(
        w.link
            .saturating_sub((regions[1].height.saturating_sub(3) / 3).saturating_sub(1) as usize),
    ) {
        let r = &l.relationship;
        let style = if l.candidate {
            Style::default().fg(theme::WARN)
        } else {
            theme::accent()
        };
        lines.push(
            Line::from(format!(
                "{} {} → {}",
                if i == w.link { "›" } else { " " },
                name(w, &r.from),
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
        regions[1],
    );
    let mut detail = Vec::new();
    if let Some(l) = links.get(w.link) {
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
        regions[2],
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
                " Review · a accept · r reject · d defer · t retain · reason required · o source "
            } else {
                " Evidence table · j/k select · o source · 5 review "
            })),
        area,
    );
}
fn timeline(frame: &mut Frame, w: &CaseWorkspace, area: Rect) {
    let mut lines = Vec::new();
    let start = w.row.saturating_sub(1);
    for (i, e) in w.data.timeline.iter().enumerate().skip(start).take(8) {
        lines.push(
            Line::from(format!(
                "{} {}\nevent {} · published {} · retrieved {}\n{}",
                if i == w.row { "›" } else { " " },
                e.lane,
                e.event_time.as_deref().unwrap_or("undated"),
                e.published_at.as_deref().unwrap_or("unknown"),
                if e.retrieved_at.is_empty() {
                    "unknown"
                } else {
                    &e.retrieved_at
                },
                e.statement
            ))
            .style(if i == w.row {
                theme::selected()
            } else {
                theme::text()
            }),
        );
    }
    for j in w.data.jobs.iter().rev().take(3) {
        lines.push(
            Line::from(format!(
                "Research lane · {} {:?}\nStarted {} · finished {}",
                j.provider,
                j.state,
                j.created_at,
                j.finished_at.as_deref().unwrap_or("pending")
            ))
            .style(theme::dim()),
        );
    }
    if lines.is_empty() {
        lines.push(Line::from(
            "No dated activity. Undated evidence stays undated.",
        ));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel(
                " Timeline lanes · j/k select observation · o same source ",
            )),
        area,
    );
}
fn jobs(frame: &mut Frame, w: &CaseWorkspace, area: Rect) {
    let mut lines = Vec::new();
    for (i, j) in w
        .data
        .jobs
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
