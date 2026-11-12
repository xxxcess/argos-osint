//! Shared Jobs query controls. Production and Runtime fixtures use this view
//! and the same controller; the adapter owns registered read effects.

use rune::El;
use rune::TextEdit;
use rune::Tx;
use rune_kit::Button;
use rune_kit::ButtonVariant;
use rune_kit::Kit;
use rune_kit::TextInput;

use crate::jobs_query::JobsEvent;
use crate::jobs_query::JobsLoad;
use crate::jobs_query::JobsQueryState;
use crate::jobs_query::JobsStatusFilter;
use crate::jobs_query::JobsTime;
use crate::jobs_query::HISTORY_LIMIT;
use crate::views::heading;
use crate::views::line;
use crate::views::quiet;

#[derive(Clone, Copy)]
enum Field {
    App,
    Operation,
    Kind,
    Correlation,
    Text,
}

impl Field {
    fn label(self) -> &'static str {
        match self {
            Self::App => "App (exact, optional)",
            Self::Operation => "Operation (exact, optional)",
            Self::Kind => "Job type (exact, optional)",
            Self::Correlation => "Correlation ID (exact, optional)",
            Self::Text => "Search job ID, operation or available private summaries",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Operation => "operation",
            Self::Kind => "kind",
            Self::Correlation => "correlation",
            Self::Text => "text",
        }
    }
    fn edit(self, jobs: &mut JobsQueryState) -> &mut TextEdit {
        match self {
            Self::App => &mut jobs.draft.app,
            Self::Operation => &mut jobs.draft.operation,
            Self::Kind => &mut jobs.draft.kind,
            Self::Correlation => &mut jobs.draft.correlation_id,
            Self::Text => &mut jobs.draft.text,
        }
    }
    fn value(self, jobs: &JobsQueryState) -> &TextEdit {
        match self {
            Self::App => &jobs.draft.app,
            Self::Operation => &jobs.draft.operation,
            Self::Kind => &jobs.draft.kind,
            Self::Correlation => &jobs.draft.correlation_id,
            Self::Text => &jobs.draft.text,
        }
    }
}

fn input<S: 'static>(
    jobs: &JobsQueryState,
    kit: &Kit,
    field: Field,
    lens: for<'a> fn(&'a mut S) -> &'a mut JobsQueryState,
) -> El {
    rune::col()
        .grow(1.0)
        .min_w(0.0)
        .gap(5.0)
        .child(quiet(field.label(), kit).key(format!("jobs-label-{}", field.key())))
        .child(
            TextInput::new(field.value(jobs))
                .label(field.label())
                .width(None)
                .on_edit(move |state: &mut S| field.edit(lens(state)))
                .on_change(move |state: &mut S, _| lens(state).draft_edited())
                .build(kit)
                .key(format!("jobs-filter-{}", field.key())),
        )
}

pub fn controls<S: 'static>(
    jobs: &JobsQueryState,
    kit: &Kit,
    lens: for<'a> fn(&'a mut S) -> &'a mut JobsQueryState,
    dispatch: fn(&mut S, JobsEvent, &mut Tx),
) -> El {
    let mut body = rune::col()
        .gap(8.0)
        .key("jobs-query-controls")
        .child(heading("Filter workstation Jobs", kit));
    let Some(origin) = jobs.origin_label() else {
        body = body.child(quiet(
            "Connect and register a workspace with activity permission to view Jobs.",
            kit,
        ));
        if let Some(message) = &jobs.message {
            if jobs.status == JobsLoad::Idle {
                return body;
            }
            body = body.child(line(message.clone(), kit).role("alert"));
        }
        return body;
    };
    body = body
        .child(quiet(format!("Current request origin: {origin}"), kit))
        .child(line("Status", kit).key("jobs-label-status"));
    for choices in JobsStatusFilter::ALL.chunks(3) {
        let mut row = rune::row().gap(6.0).w_full();
        for &status in choices {
            row = row.child(
                Button::new(status.label())
                    .variant(if jobs.draft.status == status {
                        ButtonVariant::Submit
                    } else {
                        ButtonVariant::Outline
                    })
                    .label_with(move |font, ink| {
                        rune::text(status.label())
                            .font(font)
                            .color(ink)
                            .wrap()
                            .min_w(0.0)
                    })
                    .on_press(move |state: &mut S| {
                        let jobs = lens(state);
                        jobs.draft.status = status;
                        jobs.draft_edited();
                    })
                    .build(kit)
                    .grow(1.0)
                    .min_w(0.0)
                    .key(format!("jobs-status-{}", status.key())),
            );
        }
        body = body.child(row);
    }
    body = body.child(line("Created within", kit).key("jobs-label-time"));
    let mut times = rune::row().gap(6.0).w_full();
    for time in JobsTime::ALL {
        times = times.child(
            Button::new(time.label())
                .variant(if jobs.draft.time == time {
                    ButtonVariant::Submit
                } else {
                    ButtonVariant::Outline
                })
                .label_with(move |font, ink| {
                    rune::text(time.label())
                        .font(font)
                        .color(ink)
                        .wrap()
                        .min_w(0.0)
                })
                .on_press(move |state: &mut S| {
                    let jobs = lens(state);
                    jobs.draft.time = time;
                    jobs.draft_edited();
                })
                .build(kit)
                .grow(1.0)
                .min_w(0.0)
                .key(format!("jobs-time-{}", time.key())),
        );
    }
    body = body.child(times);
    for fields in [
        [Field::App, Field::Operation],
        [Field::Kind, Field::Correlation],
    ] {
        body = body.child(
            rune::row()
                .gap(8.0)
                .w_full()
                .child(input(jobs, kit, fields[0], lens))
                .child(input(jobs, kit, fields[1], lens)),
        );
    }
    body = body
        .child(input(jobs, kit, Field::Text, lens).grow(0.0).w_full())
        .child(
            Button::new("Apply filters / refresh Jobs")
                .on_press_tx(move |state: &mut S, tx| dispatch(state, JobsEvent::Apply, tx))
                .build(kit)
                .key("jobs-query-apply"),
        )
        .child(
            Button::new("Clear filters")
                .variant(ButtonVariant::Ghost)
                .on_press_tx(move |state: &mut S, tx| dispatch(state, JobsEvent::Clear, tx))
                .build(kit)
                .key("jobs-query-clear"),
        );
    if jobs.dirty {
        body = body.child(
            quiet("Filters edited. Apply them before changing pages.", kit).key("jobs-query-dirty"),
        );
    }
    if jobs.status == JobsLoad::Loading {
        body = body.child(quiet("Loading this scoped Jobs page…", kit).role("status"));
    }
    if let Some(message) = &jobs.message {
        body = body.child(
            line(message.clone(), kit)
                .role("alert")
                .key("jobs-query-message"),
        );
    }
    body
}

pub fn pager<S: 'static>(
    jobs: &JobsQueryState,
    kit: &Kit,
    dispatch: fn(&mut S, JobsEvent, &mut Tx),
) -> El {
    let mut body = rune::col().gap(8.0).key("jobs-query-pager");
    if let Some(page) = &jobs.page {
        body = body
            .child(line(
                format!("Page {} · newest created first", jobs.page_number()),
                kit,
            ))
            .child(
                line(
                    match page.matched_total {
                        Some(total) => format!("Observed matching jobs: {total}"),
                        None => "Observed matching jobs: unavailable".into(),
                    },
                    kit,
                )
                .key("jobs-matched-total"),
            )
            .child(quiet(
                format!(
                    "Observed {}. Statuses and totals can change; refresh for new jobs.",
                    page.observed_at
                ),
                kit,
            ));
        if !page.apps_complete {
            body = body.child(
                quiet("App choices are incomplete for this observation.", kit)
                    .key("jobs-apps-incomplete"),
            );
        }
        if let Some(counts) = &page.counts {
            body = body.child(quiet(format!("Unfiltered root-job dashboard: {} running · {} queued · {} retrying · {} failed · {} completed in 24 hours. This is separate from matching jobs and child stages.", counts.active, counts.queued, counts.retrying, counts.failed, counts.completed_recent), kit).key("jobs-dashboard-counts"));
        }
        if page.has_more && jobs.page_number() == HISTORY_LIMIT {
            body = body.child(
                quiet(
                    "Page history limit reached. Narrow filters and Apply to restart.",
                    kit,
                )
                .key("jobs-history-limit"),
            );
        }
    }
    body.child(
        rune::row()
            .gap(8.0)
            .child(
                Button::new("Previous")
                    .disabled(!jobs.can_previous())
                    .on_press_tx(move |state: &mut S, tx| dispatch(state, JobsEvent::Previous, tx))
                    .build(kit)
                    .key("jobs-page-previous"),
            )
            .child(
                Button::new("Next")
                    .disabled(!jobs.can_next())
                    .on_press_tx(move |state: &mut S, tx| dispatch(state, JobsEvent::Next, tx))
                    .build(kit)
                    .key("jobs-page-next"),
            ),
    )
}
