use super::app::Target;
use super::tool_catalog;
use std::collections::HashSet;

#[test]
fn closed_categories_hide_tools_until_search() {
    let rows = tool_catalog::rows("", &HashSet::new());
    assert!(!rows.is_empty());
    assert!(rows
        .iter()
        .all(|(target, _)| matches!(target, Target::ToolCategory(_))));
    let tool = argos_osint_core::osint::registry()
        .first()
        .expect("tool registry");
    let matches = tool_catalog::rows(tool.id, &HashSet::new());
    assert!(matches
        .iter()
        .any(|(target, _)| matches!(target, Target::Tool(0))));
}

#[test]
fn search_is_case_insensitive_and_does_not_mutate_expansion() {
    let expanded = tool_catalog::initial_expansion();
    let original = expanded.clone();
    let tool = argos_osint_core::osint::registry()
        .first()
        .expect("tool registry");
    assert_eq!(
        tool_catalog::rows(tool.id, &expanded),
        tool_catalog::rows(&tool.id.to_uppercase(), &expanded)
    );
    assert_eq!(expanded, original);
    assert!(tool_catalog::rows("zzzz-no-fixture-tool", &expanded).is_empty());
}
