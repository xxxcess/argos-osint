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

pub fn rows(
    view: &ArticleRelatedView,
    width: usize,
    brain: bool,
) -> Vec<(Line<'static>, Option<Target>)> {
    if view.loading {
        return vec![(
            Line::from(Span::styled(
                super::anim::loading("Loading related articles"),
                theme::dim(),
            )),
            None,
        )];
    }
    if let Some(error) = &view.error {
        return vec![(
            Line::from(Span::styled(
                fit(&format!("Related unavailable: {error}"), width),
                theme::dim(),
            )),
            None,
        )];
    }
    if view.page.rows.is_empty() {
        return vec![(
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
        )];
    }
    let mut lines = vec![(
        Line::from(Span::styled(
            fit("Shared headline/description entities · tentative", width),
            theme::dim(),
        )),
        None,
    )];
    for (index, item) in view.page.rows.iter().enumerate() {
        let target = if brain {
            Target::BrainRelatedArticle(index)
        } else {
            Target::IntelRelatedArticle(index)
        };
        let style = if index == view.sel {
            theme::selected()
        } else {
            theme::text()
        };
        lines.push((
            Line::from(Span::styled(fit(&item.article.title, width), style)),
            Some(target),
        ));
        lines.push((
            Line::from(Span::styled(
                fit(
                    &format!(
                        "{} · {} · tentative{}",
                        item.article.source_name,
                        item.article.category,
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
                fit(
                    &format!("Shared mentions: {}", item.shared_mentions.join(", ")),
                    width,
                ),
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
