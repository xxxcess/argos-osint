use super::app::ModuleId;
use super::logs::LevelFilter;
use super::testkit;
use argos_osint_core::events::Severity;

#[test]
fn severity_cycle_updates_query_minimum_and_wraps() {
    let mut app = testkit::app(100, 36);
    for expected in [
        Some(Severity::Info),
        Some(Severity::Warn),
        Some(Severity::Error),
        None,
    ] {
        app.logs.level = app.logs.level.next();
        assert_eq!(app.logs.filter().min_severity, expected);
    }
    assert_eq!(app.logs.level, LevelFilter::All);
}

#[test]
fn filter_invalidation_removes_stale_display_and_resets_loading() {
    let mut app = testkit::app(100, 36);
    app.module = Some(ModuleId::Logs);
    app.logs.loaded = true;
    app.logs.error = Some("Old fixture failure".into());
    app.logs.invalidate_filter();
    assert!(!app.logs.loaded);
    assert!(app.logs.error.is_none());
    assert!(app.logs.selected().is_none());
    let text = testkit::screen_text(&testkit::render(&app, 100, 36));
    assert!(!text.contains("Old fixture failure"));
}
