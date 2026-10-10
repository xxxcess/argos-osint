//! Catalog identity and category expansion are independent of search.
use super::app::Target;
use argos_osint_core::osint;
use std::collections::{BTreeMap, HashSet};

pub fn initial_expansion() -> HashSet<&'static str> {
    osint::registry()
        .first()
        .map(|tool| HashSet::from([tool.category]))
        .unwrap_or_default()
}

pub fn rows(query: &str, expanded: &HashSet<&'static str>) -> Vec<(Target, String)> {
    let query = query.trim().to_ascii_lowercase();
    let mut groups: BTreeMap<&'static str, Vec<(usize, &osint::ToolDefinition)>> = BTreeMap::new();
    for (index, tool) in osint::registry().iter().enumerate() {
        if query.is_empty()
            || [tool.name, tool.id, tool.category, tool.description]
                .iter()
                .any(|text| text.to_ascii_lowercase().contains(&query))
        {
            groups.entry(tool.category).or_default().push((index, tool));
        }
    }
    let mut rows = Vec::new();
    for (category, tools) in groups {
        let open = !query.is_empty() || expanded.contains(category);
        rows.push((
            Target::ToolCategory(category),
            format!(
                "{} {category} ({})",
                if open { "▾" } else { "▸" },
                tools.len()
            ),
        ));
        if open {
            rows.extend(
                tools
                    .into_iter()
                    .map(|(index, tool)| (Target::Tool(index), format!("  {}", tool.name))),
            );
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_reveals_groups_without_mutating_expansion() {
        let closed = HashSet::new();
        assert!(rows("", &closed)
            .iter()
            .all(|(target, _)| matches!(target, Target::ToolCategory(_))));
        let tool = &osint::registry()[0];
        assert!(rows(tool.id, &closed)
            .iter()
            .any(|(target, _)| *target == Target::Tool(0)));
        assert!(closed.is_empty());
    }
}
