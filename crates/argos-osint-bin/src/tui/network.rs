//! Open-report presentations. Every layout reads the same TnaSnapshot.
use super::super::app::TnaLayout;
use super::*;
use ratatui::widgets::canvas::{Canvas, Line as CanvasLine};

pub(super) fn draw_workspace(frame: &mut Frame, app: &mut App, area: Rect) {
    app.report_area = Rect::default();
    app.report_line_index.clear();
    app.case_tab_area = Rect::default();
    app.case_tab_hits.clear();
    app.tna_table_list_area = Rect::default();
    app.tna_table_detail_area = Rect::default();
    let rows = split_v(
        area,
        &[
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Min(0),
        ],
    );
    let title = app
        .open_report()
        .map(|r| r.title.as_str())
        .unwrap_or("report");
    frame.render_widget(
        Paragraph::new(format!(
            "NETWORK · {title} · scope {:?} · R read · i inspect",
            app.evidence_scope
        ))
        .style(theme::accent().add_modifier(Modifier::BOLD)),
        rows[0],
    );
    let shortcuts = ["g", "q", "p", "m", "r"];
    let labels: Vec<String> = TnaLayout::ALL
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if area.width < 70 {
                format!("{} {}", shortcuts[i], ["C", "Cl", "P", "M", "R"][i])
            } else {
                format!("{} {}", shortcuts[i], v.title())
            }
        })
        .collect();
    let titles: Vec<_> = labels.iter().map(|s| Line::from(s.clone())).collect();
    let tabs = Tabs::new(titles)
        .select(app.tna_layout.index())
        .style(theme::dim())
        .highlight_style(theme::selected())
        .divider(" │ ")
        .block(panel(" Layout "));
    frame.render_widget(tabs, rows[1]);
    app.tna_tab_hits.clear();
    let mut x = rows[1].x.saturating_add(2);
    for view in TnaLayout::ALL {
        let width = labels[view.index()].width() as u16 + 2;
        let width = width.min(rows[1].right().saturating_sub(x));
        app.tna_tab_hits.push(Rect::new(x, rows[1].y + 1, width, 1));
        x = x.saturating_add(width + 3);
    }
    app.canvas_area = rows[2];
    if app.workspace_reading {
        let mut text = Vec::new();
        if !app.originating_question.is_empty() {
            text.push(
                Line::from(format!(
                    "Question: {}",
                    app.originating_question
                        .chars()
                        .take(100)
                        .collect::<String>()
                ))
                .style(theme::dim()),
            );
        }
        text.extend(
            app.report_read_source
                .as_deref()
                .unwrap_or(&app.tna_source)
                .lines()
                .enumerate()
                .skip(app.report_read_line)
                .map(|(n, line)| Line::from(format!("{:>5}  {}", n + 1, line))),
        );
        frame.render_widget(
            Paragraph::new(text).wrap(Wrap { trim: false }).block(panel(
                " Report · j/k scroll · g Explore Network · /cite · /scope ",
            )),
            rows[2],
        );
        return;
    }
    if app.tna_snapshot().is_none() {
        let message = app
            .tna_source_error
            .as_deref()
            .unwrap_or(if app.tna_rebuilding {
                "Preparing the existing report graph…"
            } else {
                "No report graph available. See System log for rebuild errors."
            });
        frame.render_widget(
            Paragraph::new(message)
                .style(theme::dim())
                .wrap(Wrap { trim: false })
                .block(panel(" Report network ")),
            rows[2],
        );
        return;
    }
    match app.tna_layout {
        TnaLayout::Cockpit => cockpit(frame, app, rows[2]),
        TnaLayout::Clusters => clusters(frame, app, rows[2]),
        TnaLayout::Path => paths(frame, app, rows[2]),
        TnaLayout::Matrix => matrix(frame, app, rows[2]),
        TnaLayout::Ribbon => ribbon(frame, app, rows[2]),
    }
}

fn cockpit(frame: &mut Frame, app: &mut App, area: Rect) {
    let cols = if area.width >= 112 {
        split_h(
            area,
            &[
                Constraint::Length(30),
                Constraint::Min(42),
                Constraint::Length(38),
            ],
        )
    } else {
        split_h(
            area,
            &[Constraint::Percentage(35), Constraint::Percentage(65)],
        )
    };
    app.tna_table_list_area = cols[0];
    app.tna_table_detail_area = cols[1];
    draw_tna_table_list(frame, app, cols[0]);
    if cols.len() == 3 {
        ego(frame, app, cols[1]);
        ledger(frame, app, cols[2]);
    } else if cols[1].width >= 26 && cols[1].height >= 13 {
        ego(frame, app, cols[1]);
    } else {
        ledger(frame, app, cols[1]);
    }
}

fn ego(frame: &mut Frame, app: &App, area: Rect) {
    let selected = app
        .tna_focus_node()
        .map(|n| n.label.as_str())
        .unwrap_or("none");
    let block = panel(" Ego network · 5 hops ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = split_v(
        inner,
        &[
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(1),
        ],
    );
    frame.render_widget(
        Paragraph::new(selected.to_string())
            .style(theme::selected())
            .wrap(Wrap { trim: false }),
        rows[0],
    );
    draw_tna_ego_boxes(frame, app, rows[1]);
    let shown = app
        .tna_detail_box_items(tna_ego_box_fit(rows[1].width, rows[1].height))
        .len();
    frame.render_widget(
        Paragraph::new(format!(
            "{shown}/{} entities · Tab ego · j/k page",
            app.tna_display_nodes().len()
        ))
        .style(theme::dim()),
        rows[2],
    );
}

fn ledger(frame: &mut Frame, app: &App, area: Rect) {
    let mut lines = Vec::new();
    let node = app.tna_focus_node();
    let edges = app.tna_links();
    if edges.is_empty() {
        lines.push(Line::from("No connected edges in this snapshot.").style(theme::dim()));
    }
    for (i, edge) in edges
        .iter()
        .enumerate()
        .skip(app.tna_ledger_sel.min(edges.len().saturating_sub(1)))
    {
        let other = if node.is_some_and(|n| n.id == edge.from) {
            &edge.to
        } else {
            &edge.from
        };
        lines.push(
            Line::from(format!(
                "{} {} · weight {}",
                if i == app.tna_ledger_sel { "▶" } else { " " },
                app.tna_label(other),
                edge.weight
            ))
            .style(theme::accent()),
        );
        lines.extend(
            app.tna_edge_evidence(&edge.from, &edge.to)
                .lines()
                .map(|s| Line::from(s.to_string()).style(theme::dim())),
        );
        lines.push(Line::from(""));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel(" Link Ledger · [ / ] browse ")),
        area,
    );
}

fn clusters(frame: &mut Frame, app: &mut App, area: Rect) {
    let cols = split_h(
        area,
        &[
            Constraint::Min(30),
            Constraint::Length(if area.width >= 90 { 32 } else { 24 }),
        ],
    );
    app.tna_table_detail_area = cols[1];
    let regions_area = Rect::new(
        cols[0].x,
        cols[0].y,
        cols[0].width,
        cols[0].height.saturating_sub(1),
    );
    let rows = split_v(
        regions_area,
        &[
            Constraint::Percentage(50),
            Constraint::Length(1),
            Constraint::Percentage(50),
        ],
    );
    let top = split_h(
        rows[0],
        &[Constraint::Percentage(50), Constraint::Percentage(50)],
    );
    let bottom = split_h(
        rows[2],
        &[Constraint::Percentage(50), Constraint::Percentage(50)],
    );
    let regions = [top[0], top[1], bottom[0], bottom[1]];
    let snap = app.tna_snapshot().unwrap();
    let indicators: Vec<_> = snap
        .gaps
        .iter()
        .map(|gap| {
            format!(
                "┄ {} ↔ {}: no edge ┄",
                cluster_abbr(gap.cluster_a),
                cluster_abbr(gap.cluster_b)
            )
        })
        .collect();
    frame.render_widget(
        Paragraph::new(if indicators.is_empty() {
            "─".repeat(rows[1].width as usize)
        } else {
            indicators.join("  ")
        })
        .style(if indicators.is_empty() {
            theme::dim()
        } else {
            Style::default().fg(theme::WARN)
        }),
        rows[1],
    );
    let visible = app.tna_visible_nodes();
    let focus = app.tna_focus_node().map(|n| n.id.as_str());
    for (index, cluster) in TnaCluster::all().iter().enumerate() {
        let nodes: Vec<_> = visible
            .iter()
            .map(|i| &snap.nodes[*i])
            .filter(|n| n.cluster == *cluster)
            .collect();
        let block = panel(&format!(" {} · {} ", cluster.label(), nodes.len()));
        let inner = block.inner(regions[index]);
        frame.render_widget(block, regions[index]);
        if nodes.is_empty() {
            frame.render_widget(
                Paragraph::new("No entities in this report.")
                    .style(theme::dim())
                    .wrap(Wrap { trim: false }),
                inner,
            );
            continue;
        }
        // Keep the persisted coordinates; use separate labeled regions for type categories.
        let canvas = Canvas::default()
            .x_bounds([0.0, 1.0])
            .y_bounds([0.0, 1.0])
            .paint(|ctx| {
                for edge in &snap.edges {
                    let a = nodes.iter().find(|n| n.id == edge.from);
                    let b = nodes.iter().find(|n| n.id == edge.to);
                    if let (Some(a), Some(b)) = (a, b) {
                        ctx.draw(&CanvasLine {
                            x1: a.x,
                            y1: 1.0 - a.y,
                            x2: b.x,
                            y2: 1.0 - b.y,
                            color: theme::BORDER,
                        });
                    }
                }
            });
        frame.render_widget(canvas, inner);
        let mut ordered = nodes.clone();
        ordered.sort_by(|a, b| {
            (focus != Some(a.id.as_str()))
                .cmp(&(focus != Some(b.id.as_str())))
                .then(a.y.total_cmp(&b.y))
                .then(a.x.total_cmp(&b.x))
        });
        let mut used = std::collections::HashSet::new();
        let capacity = inner.height.saturating_sub(1) as usize;
        for node in ordered.iter().take(capacity) {
            let desired = (node.y.clamp(0.0, 1.0) * inner.height.saturating_sub(2) as f64) as u16;
            let row = (0..inner.height.saturating_sub(1))
                .filter(|r| !used.contains(r))
                .min_by_key(|r| r.abs_diff(desired));
            let Some(row) = row else {
                break;
            };
            used.insert(row);
            let text = format!(
                "{} {} · {}",
                tna_glyph_for_kind(node.kind),
                node.label,
                app.entity_coverage(&node.id)
            );
            let width = (text.width() as u16).min(inner.width);
            let x = ((node.x.clamp(0.0, 1.0) * inner.width as f64) as u16)
                .min(inner.width.saturating_sub(width));
            frame.render_widget(
                Paragraph::new(fit_label(&text, width as usize)).style(
                    if focus == Some(node.id.as_str()) {
                        theme::selected()
                    } else {
                        Style::default().fg(cluster_color(*cluster))
                    },
                ),
                Rect::new(inner.x + x, inner.y + row, width, 1),
            );
        }
        if nodes.len() > capacity && inner.height > 0 {
            frame.render_widget(
                Paragraph::new(format!("+{} · j/k browse", nodes.len() - capacity))
                    .style(theme::dim()),
                Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
            );
        }
    }
    let side = split_v(
        cols[1],
        &[Constraint::Percentage(55), Constraint::Percentage(45)],
    );
    let mut anchors =
        vec![Line::from("Degree, then source mention support; not confidence").style(theme::dim())];
    for anchor in snap.anchors.iter().take(10) {
        anchors.push(
            Line::from(format!(
                "{} {} · {}",
                if focus == Some(anchor.node_id.as_str()) {
                    "▶"
                } else {
                    " "
                },
                app.tna_label(&anchor.node_id),
                anchor.degree
            ))
            .style(theme::accent()),
        );
    }
    anchors.push(Line::from("j/k select · Enter Cockpit").style(theme::dim()));
    frame.render_widget(
        Paragraph::new(anchors)
            .wrap(Wrap { trim: false })
            .block(panel(" Connectivity · degree ")),
        side[0],
    );
    let mut gaps = vec![Line::from("Absent co-occurrence; type groups").style(theme::dim())];
    for gap in &snap.gaps {
        gaps.push(Line::from(format!("{}", gap.note)).style(Style::default().fg(theme::WARN)));
    }
    if snap.gaps.is_empty() {
        gaps.push(Line::from("No missing group links."));
    }
    frame.render_widget(
        Paragraph::new(gaps)
            .wrap(Wrap { trim: false })
            .block(panel(" Gap list ")),
        side[1],
    );
    // A readable focused label survives even when spatial labels overlap.
    if let Some(node) = app.tna_focus_node() {
        let line = Rect::new(
            cols[0].x,
            cols[0].bottom().saturating_sub(1),
            cols[0].width,
            1,
        );
        frame.render_widget(
            Paragraph::new(format!("Selected: {} · {}", node.label, node.kind.as_str()))
                .style(theme::selected()),
            line,
        );
    }
}

fn paths(frame: &mut Frame, app: &mut App, area: Rect) {
    let cols = split_h(
        area,
        &[
            Constraint::Length(if area.width >= 90 { 30 } else { 24 }),
            Constraint::Min(0),
        ],
    );
    app.tna_table_list_area = cols[0];
    app.tna_table_detail_area = cols[1];
    draw_tna_table_list(frame, app, cols[0]);
    let rows = split_v(
        cols[1],
        &[
            Constraint::Length(4),
            Constraint::Min(3),
            Constraint::Length((area.height / 3).max(4)),
        ],
    );
    let from = app
        .tna_from
        .as_ref()
        .map(|id| app.tna_label(id))
        .unwrap_or_else(|| "select entity, press f".into());
    let to = app
        .tna_to
        .as_ref()
        .map(|id| app.tna_label(id))
        .unwrap_or_else(|| "select entity, press t".into());
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!("FROM  {from}")).style(theme::accent()),
            Line::from(format!("TO    {to}")).style(theme::accent()),
        ])
        .block(panel(" Relationship pins ")),
        rows[0],
    );
    let paths = app.tna_paths();
    if paths.is_empty() {
        let msg = if app.tna_from.is_none() || app.tna_to.is_none() {
            "Pin both entities to explore up to five paths. Tab switches entity / path selection."
        } else {
            if app.tna_path_search_limited.get() {
                "No path recovered before the bounded search limit."
            } else {
                "The pins are disconnected or farther than four hops apart."
            }
        };
        frame.render_widget(
            Paragraph::new(msg)
                .wrap(Wrap { trim: false })
                .style(theme::dim())
                .block(panel(" Paths · up to 4 hops ")),
            rows[1],
        );
    } else {
        let mut y = rows[1].y;
        for (i, path) in paths
            .iter()
            .enumerate()
            .skip(app.tna_path_sel.min(paths.len() - 1))
        {
            let labels: Vec<_> = path.nodes.iter().map(|id| app.tna_label(id)).collect();
            let text = labels.join(" → ");
            let style = if i == app.tna_path_sel {
                theme::selected()
            } else {
                theme::text()
            };
            let lines = markdown_lines(&text, rows[1].width.saturating_sub(2) as usize, style);
            let wanted = (lines.len() as u16).saturating_add(2).max(4);
            let available = rows[1].bottom().saturating_sub(y);
            if available < 4 {
                break;
            }
            let height = wanted.min(available);
            frame.render_widget(
                Paragraph::new(lines).block(panel(&format!(
                    " Path {}/{} · {} hops · strength {} ",
                    i + 1,
                    paths.len(),
                    path.nodes.len() - 1,
                    path.strength
                ))),
                Rect::new(rows[1].x, y, rows[1].width, height),
            );
            y = y.saturating_add(height);
        }
    }

    if app.tna_path_search_limited.get() && !paths.is_empty() && rows[1].height > 0 {
        frame.render_widget(
            Paragraph::new("Search limit reached; showing recovered paths.")
                .style(Style::default().fg(theme::WARN)),
            Rect::new(rows[1].x, rows[1].bottom() - 1, rows[1].width, 1),
        );
    }
    let evidence = paths
        .get(app.tna_path_sel)
        .and_then(|path| {
            path.nodes
                .windows(2)
                .nth(app.tna_hop_sel.min(path.nodes.len().saturating_sub(2)))
        })
        .map(|pair| app.tna_edge_evidence(&pair[0], &pair[1]))
        .unwrap_or_else(|| {
            "Select a path to inspect each hop. Co-occurrence is not proof of a relationship."
                .into()
        });
    frame.render_widget(
        Paragraph::new(evidence)
            .wrap(Wrap { trim: false })
            .style(theme::dim())
            .block(panel(
                " Evidence · [ / ] every hop · /corroborate · /weakest ",
            )),
        rows[2],
    );
}

fn matrix(frame: &mut Frame, app: &mut App, area: Rect) {
    if app.tna_matrix_coverage {
        coverage_matrix(frame, app, area);
        return;
    }
    let rows = split_v(area, &[Constraint::Length(9), Constraint::Min(0)]);
    let counts = app.tna_cluster_counts();
    let snap = app.tna_snapshot().unwrap();
    let head =
        Row::new(["Group", "Infra", "Themes / Orgs", "Identity", "Reports"]).style(theme::accent());
    let macro_rows: Vec<_> = TnaCluster::all()
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut cells = vec![Cell::from(c.label())];
            for (j, other) in TnaCluster::all().iter().enumerate() {
                let present = snap.nodes.iter().any(|n| n.cluster == *c)
                    && snap.nodes.iter().any(|n| n.cluster == *other);
                cells.push(
                    Cell::from(if counts[i][j] == 0 && present && i != j {
                        "gap".into()
                    } else {
                        counts[i][j].to_string()
                    })
                    .style(if counts[i][j] == 0 {
                        theme::dim()
                    } else {
                        theme::accent()
                    }),
                );
            }
            Row::new(cells)
        })
        .collect();
    frame.render_widget(
        Table::new(
            macro_rows,
            [
                Constraint::Length(20),
                Constraint::Ratio(1, 4),
                Constraint::Ratio(1, 4),
                Constraint::Ratio(1, 4),
                Constraint::Ratio(1, 4),
            ],
        )
        .header(head)
        .block(panel(" Cluster edge counts · gap = no co-occurrence ")),
        rows[0],
    );
    let block = panel(" Adjacency · top 24 · c coverage · hjkl · Enter Cockpit ");
    let inner = block.inner(rows[1]);
    frame.render_widget(block, rows[1]);
    let nodes = app.tna_matrix_nodes();
    if nodes.is_empty() {
        frame.render_widget(
            Paragraph::new("No entities match Find.").style(theme::dim()),
            inner,
        );
        return;
    }
    app.tna_matrix_row = app.tna_matrix_row.min(nodes.len() - 1);
    app.tna_matrix_col = app.tna_matrix_col.min(nodes.len() - 1);
    let snap = app.tna_snapshot().unwrap();
    let cols = split_h(
        inner,
        &[
            Constraint::Length(22.min(inner.width / 2)),
            Constraint::Min(0),
        ],
    );
    let grid_rows = cols[1].height.saturating_sub(3) as usize;
    let grid_cols = (cols[1].width / 3) as usize;
    let row_start = app
        .tna_matrix_row
        .saturating_sub(grid_rows.saturating_sub(1));
    let col_start = app
        .tna_matrix_col
        .saturating_sub(grid_cols.saturating_sub(1));
    let mut labels = vec![Line::from("Entity").style(theme::accent())];
    for (index, node) in nodes.iter().enumerate().skip(row_start).take(grid_rows) {
        labels.push(
            Line::from(format!(
                "{:02} {}",
                index + 1,
                short_label(&snap.nodes[*node].label, 18)
            ))
            .style(if index == app.tna_matrix_row {
                theme::selected()
            } else {
                theme::text()
            }),
        );
    }
    frame.render_widget(Paragraph::new(labels), cols[0]);
    let header: Vec<_> = nodes
        .iter()
        .enumerate()
        .skip(col_start)
        .take(grid_cols)
        .map(|(i, _)| Span::styled(format!("{:02} ", i + 1), theme::accent()))
        .collect();
    let mut lines = vec![Line::from(header)];
    for (i, a) in nodes.iter().enumerate().skip(row_start).take(grid_rows) {
        let mut cells = Vec::new();
        for (j, b) in nodes.iter().enumerate().skip(col_start).take(grid_cols) {
            let weight = app.tna_edge_weight(&snap.nodes[*a].id, &snap.nodes[*b].id);
            let glyph = if i == j {
                " · "
            } else if weight == 0 {
                "   "
            } else if weight == 1 {
                " ░ "
            } else if weight < 4 {
                " ▒ "
            } else {
                " █ "
            };
            let style = if i == app.tna_matrix_row && j == app.tna_matrix_col {
                theme::selected().add_modifier(Modifier::REVERSED)
            } else if weight >= 4 {
                theme::accent()
            } else if weight > 0 {
                Style::default().fg(theme::BORDER)
            } else {
                theme::dim()
            };
            cells.push(Span::styled(glyph, style));
        }
        lines.push(Line::from(cells));
    }
    frame.render_widget(Paragraph::new(lines), cols[1]);
    let a = &snap.nodes[nodes[app.tna_matrix_row]];
    let b = &snap.nodes[nodes[app.tna_matrix_col]];
    let status = Rect::new(
        inner.x,
        inner.bottom().saturating_sub(2),
        inner.width,
        2.min(inner.height),
    );
    frame.render_widget(
        Paragraph::new(format!(
            "{} ↔ {} · weight {}",
            a.label,
            b.label,
            app.tna_edge_weight(&a.id, &b.id)
        ))
        .wrap(Wrap { trim: false })
        .style(theme::accent()),
        status,
    );
}

fn ribbon(frame: &mut Frame, app: &mut App, area: Rect) {
    let rows = split_v(area, &[Constraint::Length(7), Constraint::Min(0)]);
    let decisions = app.tna_ribbon_decisions();
    let positions = app.tna_ribbon_positions();
    let index = app.tna_ribbon_pos.min(positions.len().saturating_sub(1));
    let position = positions.get(index).copied().unwrap_or(0);
    let end = positions
        .get(index + 1)
        .copied()
        .unwrap_or(app.tna_source.len());
    let selected = decisions
        .iter()
        .find(|d| d.start >= position && d.start < end)
        .copied();
    let block = panel(&format!(
        " Evidence ribbon · h/l scrub · d rejected: {} ",
        if app.tna_show_rejected {
            "shown"
        } else {
            "hidden"
        }
    ));
    let inner = block.inner(rows[0]);
    frame.render_widget(block, rows[0]);
    let mut sections = Vec::new();
    for line in app
        .tna_source
        .lines()
        .filter(|l| l.trim_start().starts_with("## "))
    {
        let heading = line.trim().trim_start_matches('#').trim().to_string();
        if !sections.contains(&heading) {
            sections.push(heading);
        }
    }
    for d in &decisions {
        if !sections.contains(&d.section) {
            sections.push(d.section.clone());
        }
    }
    frame.render_widget(
        Paragraph::new(sections.join(" │ ")).style(theme::accent()),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    let width = inner.width as usize;
    let mut ticks = vec![Span::styled("─", theme::dim()); width];
    let length = app.tna_source.len().max(1);
    for d in &decisions {
        let x =
            (d.start.saturating_mul(width.saturating_sub(1)) / length).min(width.saturating_sub(1));
        if let Some(tick) = ticks.get_mut(x) {
            *tick = Span::styled(
                if d.canonical_id.is_some() {
                    "│"
                } else {
                    "┊"
                },
                if d.canonical_id.is_some() {
                    Style::default().fg(cluster_color(d.kind.cluster()))
                } else {
                    theme::dim()
                },
            );
        }
    }
    let x =
        (position.saturating_mul(width.saturating_sub(1)) / length).min(width.saturating_sub(1));
    if let Some(tick) = ticks.get_mut(x) {
        *tick = Span::styled("▼", theme::selected());
    }
    let details = selected
        .map(|d| {
            format!(
                "{} · {} · bytes {}–{} · {}",
                d.section,
                d.label.as_deref().unwrap_or(&d.original),
                d.start,
                d.end,
                d.reason
            )
        })
        .unwrap_or_else(|| format!("Report byte {position} · no visible candidate on this line"));
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(ticks),
            Line::from(details).style(theme::dim()),
        ])
        .wrap(Wrap { trim: false }),
        Rect::new(
            inner.x,
            inner.y + 1,
            inner.width,
            inner.height.saturating_sub(1),
        ),
    );
    let cols = split_h(
        rows[1],
        &[Constraint::Percentage(65), Constraint::Percentage(35)],
    );
    let source_block = panel(" Report text · playhead ");
    let source_inner = source_block.inner(cols[0]);
    frame.render_widget(source_block, cols[0]);
    let window = app.tna_ribbon_window(position);
    let mut source_lines = Vec::new();
    let mut offset = 0usize;
    let mut text_lines = Vec::new();
    for line in app.tna_source.split_inclusive('\n') {
        text_lines.push((offset, line));
        offset += line.len();
    }
    let line_index = text_lines
        .iter()
        .rposition(|(start, _)| *start <= position)
        .unwrap_or(0);
    let start = line_index.saturating_sub(3);
    for (offset, line) in text_lines
        .iter()
        .skip(start)
        .take(source_inner.height as usize)
    {
        let mut spans = Vec::new();
        let mut cursor = 0usize;
        for d in &window {
            let end = offset + line.len();
            if d.start >= *offset && d.end <= end {
                let a = d.start - offset;
                let b = d.end - offset;
                if a < cursor {
                    continue;
                }
                if let (Some(before), Some(entity)) = (line.get(cursor..a), line.get(a..b)) {
                    spans.push(Span::styled(before.to_string(), theme::text()));
                    spans.push(Span::styled(entity.to_string(), theme::selected()));
                    cursor = b;
                }
            }
        }
        spans.push(Span::styled(
            line.get(cursor..).unwrap_or("").trim_end().to_string(),
            theme::text(),
        ));
        source_lines.push(Line::from(spans));
    }
    if let Some(err) = &app.tna_source_error {
        source_lines = vec![Line::from(err.clone()).style(Style::default().fg(theme::RED))];
    }
    frame.render_widget(
        Paragraph::new(source_lines).wrap(Wrap { trim: false }),
        source_inner,
    );
    let mut lines = vec![Line::from("Existing window: 3 accepted mentions").style(theme::dim())];
    for d in window {
        lines.push(
            Line::from(format!(
                "{} {}",
                tna_glyph_for_kind(d.kind),
                d.label.as_deref().unwrap_or(&d.original)
            ))
            .style(Style::default().fg(cluster_color(d.kind.cluster()))),
        );
        lines.push(
            Line::from(format!("{} · bytes {}–{}", d.section, d.start, d.end)).style(theme::dim()),
        );
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel(" Nearby co-occurrence ")),
        cols[1],
    );
}

pub(super) fn draw_answer(frame: &mut Frame, app: &App, area: Rect) {
    let Some(answer) = &app.tna_answer else {
        return;
    };
    let modal = centered(
        area,
        area.width.saturating_mul(4) / 5,
        area.height.saturating_mul(3) / 4,
    );
    frame.render_widget(Clear, modal);
    let title = app
        .open_report()
        .map(|r| r.title.as_str())
        .unwrap_or("report");
    let block = panel(&format!(" TNA · {title} "));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    let rows = split_v(
        inner,
        &[
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(2),
        ],
    );
    frame.render_widget(
        Paragraph::new(format!("❯ {}", answer.question))
            .style(theme::user_message())
            .wrap(Wrap { trim: false }),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new("─".repeat(inner.width as usize)).style(theme::dim()),
        rows[1],
    );
    let mut lines = markdown_lines(&answer.answer, inner.width as usize, theme::text());
    if answer.pending {
        lines.push(Line::from(format!("{} thinking…", app.turn_spinner())).style(theme::accent()));
    }
    if let Some(err) = &answer.error {
        lines.push(Line::from(format!("Error: {err}")).style(Style::default().fg(theme::RED)));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((app.tna_answer_scroll, 0)),
        rows[2],
    );
    let memory = if answer.filed {
        "insight filed to report Brain"
    } else if answer.error.is_some() {
        "no insight filed"
    } else if answer.pending {
        "insight files after completion"
    } else {
        "insight filing to report Brain"
    };
    frame.render_widget(
        Paragraph::new(format!(
            "Enter follow-up · Esc dismiss · PgUp/Dn scroll\n{memory} · Q&A is temporary"
        ))
        .style(theme::dim()),
        rows[3],
    );
}

pub(super) fn draw_help(frame: &mut Frame, area: Rect) {
    let modal = centered(area, area.width * 3 / 4, area.height * 3 / 4);
    frame.render_widget(Clear, modal);
    let text="Open-report network\n\ng Cockpit · q Clusters · p Path · m Matrix · r Ribbon\nLeft/Right layouts · / Find · Enter finish Find · Esc clear Find\nj/k or arrows select · Tab entity / detail / Ask / Apps\nCockpit: [ / ] cycle linked entities\nClusters: Tab anchors · j/k select · Enter Cockpit\nPath: f FROM · t TO · Tab paths · j/k select path\nMatrix: hjkl cursor · Enter Cockpit\nRibbon: h/l scrub · d show rejected decisions\nEnter on answer follows up · PgUp/Dn scroll answer\nEsc: Find → answer → close report and return to Desk\n\nAnswers are temporary. Completed insights file to report Brain.\nResearch and memory editing are Desk-only.\nAny key dismisses this help.";
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .block(panel(" TNA help ")),
        modal,
    );
}

fn coverage_matrix(frame: &mut Frame, app: &mut App, area: Rect) {
    let block =
        panel(" Theme × report coverage · c adjacency · hjkl · Enter passage · limit 24 × 24 ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines=vec![Line::from("Metric: distinct supporting passages; 0 = no indexed match, ? = unavailable. Not confidence.").style(theme::dim())];
    let col_start = app
        .tna_matrix_col
        .saturating_sub((inner.width.saturating_sub(24) / 5).saturating_sub(1) as usize);
    let row_start = app
        .tna_matrix_row
        .saturating_sub(inner.height.saturating_sub(4) as usize);
    if let Some(row) = app.coverage_rows.first() {
        let mut head = vec![Span::styled(format!("{:<24}", "Theme"), theme::accent())];
        for (n, c) in row
            .cells
            .iter()
            .enumerate()
            .skip(col_start)
            .take((inner.width.saturating_sub(24) / 5) as usize)
        {
            head.push(Span::styled(
                format!("{:>4} ", n + 1),
                if n == app.tna_matrix_col {
                    theme::selected()
                } else {
                    theme::dim()
                },
            ));
            if n == app.tna_matrix_col {
                lines.push(
                    Line::from(format!("Selected report: {} ({})", c.title, c.report_id))
                        .style(theme::accent()),
                );
            }
        }
        lines.push(Line::from(head));
    }
    for (i, row) in app
        .coverage_rows
        .iter()
        .enumerate()
        .skip(row_start)
        .take(inner.height.saturating_sub(4) as usize)
    {
        let mut spans = vec![Span::styled(
            format!("{:<24}", short_label(&row.theme, 23)),
            if i == app.tna_matrix_row {
                theme::selected()
            } else {
                theme::text()
            },
        )];
        for (j, cell) in row
            .cells
            .iter()
            .enumerate()
            .skip(col_start)
            .take((inner.width.saturating_sub(24) / 5) as usize)
        {
            spans.push(Span::styled(
                format!(
                    "{:>4} ",
                    cell.passages
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "?".into())
                ),
                if i == app.tna_matrix_row && j == app.tna_matrix_col {
                    theme::selected()
                } else {
                    theme::text()
                },
            ));
        }
        lines.push(Line::from(spans));
    }
    if app.coverage_rows.is_empty() {
        lines.push(Line::from(
            "No extracted themes in this scope, or coverage is still loading.",
        ));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
