//! Immutable, model-free Related article presentation shared by Intel and Brain.
use super::{app::Target, theme, ui::fit};
use argos_osint_core::related_articles::RelatedArticlePage;
use ratatui::text::{Line, Span};

#[derive(Clone, Debug, Default)]
pub struct ArticleRelatedView {
    pub page: RelatedArticlePage,
    pub error: Option<String>,
    pub loading: bool,
    pub sel: usize,
    pub scroll: u16,
}

/// Labelled bases ("same actor: X", "Shared mentions: …") print as-is;
/// bare mention names keep the shared-mentions label.
pub fn basis_text(basis: &[String]) -> String {
    if basis.iter().all(|b| b.contains(": ")) && !basis.is_empty() {
        basis.join("; ")
    } else {
        format!("Shared mentions: {}", basis.join(", "))
    }
}

pub fn rows(
    view: &ArticleRelatedView,
    width: usize,
    brain: bool,
) -> Vec<(Line<'static>, Option<Target>)> {
    let scope = if view.page.coverage == "this cycle" {
        "This cycle"
    } else {
        "All retained cycles"
    };
    let toggle = (
        Line::from(Span::styled(
            fit(&format!("Scope: {scope} · click to switch"), width),
            theme::accent(),
        )),
        Some(Target::Button(super::app::ButtonId::RelatedScope)),
    );
    if view.loading {
        return vec![
            toggle,
            (
                Line::from(Span::styled(
                    super::anim::center_text(
                        &super::anim::loading("Loading related articles"),
                        width,
                    ),
                    theme::dim(),
                )),
                None,
            ),
        ];
    }
    if let Some(error) = &view.error {
        return vec![
            toggle,
            (
                Line::from(Span::styled(
                    fit(&format!("Related unavailable: {error}"), width),
                    theme::dim(),
                )),
                None,
            ),
        ];
    }
    if view.page.rows.is_empty() {
        return vec![
            toggle,
            (
                Line::from(Span::styled(
                    fit(
                        &format!(
                            "No related articles · {}",
                            if view.page.coverage.is_empty() {
                                "coverage unavailable"
                            } else {
                                &view.page.coverage
                            }
                        ),
                        width,
                    ),
                    theme::dim(),
                )),
                None,
            ),
        ];
    }
    let mut lines = vec![
        toggle,
        (
            Line::from(Span::styled(
                fit("One row per article URL · cycles and basis below", width),
                theme::dim(),
            )),
            None,
        ),
    ];
    for (index, item) in view.page.rows.iter().enumerate() {
        if index > 0 {
            lines.push((Line::raw(""), None));
        }
        let target = if brain {
            Target::RelatedRow(index)
        } else {
            Target::IntelRelatedArticle(index)
        };
        let style = if index == view.sel {
            theme::selected()
        } else {
            theme::text()
        };
        for text in super::ui::wrap_text(&item.article.title, width) {
            lines.push((Line::from(Span::styled(text, style)), Some(target)));
        }
        lines.push((
            Line::from(Span::styled(
                fit(
                    &format!(
                        "{} · {} · {} · {} cycle{} · {}{}",
                        item.article.source_name,
                        item.article.category,
                        item.article.published_at.get(..10).unwrap_or("undated"),
                        item.originating_run_ids.len().max(1),
                        if item.originating_run_ids.len() > 1 {
                            "s"
                        } else {
                            ""
                        },
                        if item.established {
                            "same actor"
                        } else {
                            "tentative"
                        },
                        if item.place_only {
                            " · place-only"
                        } else {
                            ""
                        }
                    ),
                    width,
                ),
                theme::dim(),
            )),
            Some(target),
        ));
        lines.push((
            Line::from(Span::styled(
                fit(&basis_text(&item.shared_mentions), width),
                theme::muted(),
            )),
            Some(target),
        ));
        lines.push((
            Line::from(Span::styled(
                fit(
                    &format!("Cycles: {}", item.originating_run_ids.join(", ")),
                    width,
                ),
                theme::dim(),
            )),
            Some(target),
        ));
    }
    lines.push((
        Line::from(Span::styled(
            format!(
                "{} of {} · {}",
                view.page.rows.len(),
                view.page.total,
                view.page.coverage
            ),
            theme::dim(),
        )),
        None,
    ));
    lines
}
