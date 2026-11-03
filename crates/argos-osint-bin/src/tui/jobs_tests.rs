use super::app::ModuleId;
use super::testkit;
use argos_osint_core::jobs_view::{JobRow, JobStatusFilter};

fn fixture() -> super::App {
    let mut app = testkit::app(120, 40);
    app.module = Some(ModuleId::Jobs);
    app.jobs.loaded = true;
    app.jobs.rows = vec![
        JobRow {
            id: "fixture-active".into(),
            title: "Active fixture".into(),
            app: "intel".into(),
            state: "running".into(),
            ..Default::default()
        },
        JobRow {
            id: "fixture-complete".into(),
            title: "Completed fixture".into(),
            app: "intel".into(),
            state: "completed".into(),
            ..Default::default()
        },
    ];
    app.jobs.apply_filter();
    app
}

#[test]
fn jobs_empty_state_is_visible() {
    let mut app = fixture();
    app.jobs.rows.clear();
    app.jobs.apply_filter();
    let text = testkit::screen_text(&testkit::render(&app, 120, 40));
    assert!(text.contains("No background work recorded yet."), "{text}");
}

#[test]
fn jobs_table_renders_both_fixture_titles() {
    let app = fixture();
    let text = testkit::screen_text(&testkit::render(&app, 120, 40));
    assert!(text.contains("Active fixture"), "{text}");
    assert!(text.contains("Completed fixture"), "{text}");
}

#[test]
fn active_filter_removes_completed_row_and_synchronizes_detail() {
    let mut app = fixture();
    app.jobs.sel = 1;
    app.jobs.sel_id = Some("fixture-complete".into());
    app.jobs.status = JobStatusFilter::Active;
    app.jobs.apply_filter();
    assert_eq!(app.jobs.rows.len(), 1);
    assert_eq!(app.jobs.sel_id.as_deref(), Some("fixture-active"));
    assert_eq!(
        app.jobs.detail.as_ref().expect("active detail").job.id,
        "fixture-active"
    );
    let text = testkit::screen_text(&testkit::render(&app, 120, 40));
    assert!(!text.contains("Completed fixture"));
}

#[test]
fn pruning_selected_completed_job_clamps_selection() {
    let mut app = fixture();
    app.jobs.sel = 1;
    app.jobs.sel_id = Some("fixture-complete".into());
    app.jobs.prune_status(JobStatusFilter::Completed);
    assert_eq!(app.jobs.rows.len(), 1);
    assert_eq!(app.jobs.sel, 0);
    assert_eq!(app.jobs.sel_id.as_deref(), Some("fixture-active"));
}

#[test]
fn empty_filter_closes_stale_detail() {
    let mut app = fixture();
    app.jobs.detail_open = true;
    app.jobs.app = "missing-app".into();
    app.jobs.apply_filter();
    assert!(app.jobs.rows.is_empty());
    assert!(app.jobs.detail.is_none());
    assert!(app.jobs.sel_id.is_none());
    assert!(!app.jobs.detail_open);
}
