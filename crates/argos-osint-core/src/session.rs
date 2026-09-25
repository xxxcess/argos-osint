//! Cases and the slash-command card.
//!
//! `resolve_case` matches a `/use` query by exact case-insensitive id or title,
//! then by a single unambiguous case-insensitive id or title prefix. An empty
//! or ambiguous query returns `None`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Case {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardOption {
    pub id: String,
    pub label: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub title: String,
    pub options: Vec<CardOption>,
    pub selected: usize,
}

impl Card {
    pub fn option_count(&self) -> usize {
        self.options.len()
    }

    pub fn selected(&self) -> Option<&CardOption> {
        self.options.get(self.selected)
    }

    pub fn move_sel(&mut self, delta: isize) {
        let n = self.options.len();
        if n == 0 {
            self.selected = 0;
            return;
        }
        let cur = self.selected as isize;
        let next = (cur + delta).rem_euclid(n as isize) as usize;
        self.selected = next;
    }
}

pub fn resolve_case(cases: &[Case], query: &str) -> Option<Case> {
    let q = query.trim();
    if q.is_empty() {
        return None;
    }
    let ql = q.to_lowercase();
    if let Some(hit) = cases
        .iter()
        .find(|c| c.id.eq_ignore_ascii_case(q) || c.title.eq_ignore_ascii_case(q))
    {
        return Some(hit.clone());
    }
    let prefs: Vec<Case> = cases
        .iter()
        .filter(|c| c.id.to_lowercase().starts_with(&ql) || c.title.to_lowercase().starts_with(&ql))
        .cloned()
        .collect();
    if prefs.len() == 1 {
        prefs.into_iter().next()
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SlashCommand {
    pub name: &'static str,
    pub summary: &'static str,
}

pub fn slash_commands() -> &'static [SlashCommand] {
    &[
        SlashCommand {
            name: "help",
            summary: "Show keyboard and command help",
        },
        SlashCommand {
            name: "dashboard",
            summary: "Return to the app launcher",
        },
        SlashCommand {
            name: "new",
            summary: "Open a case: /new <title>",
        },
        SlashCommand {
            name: "use",
            summary: "Focus a case by id or title",
        },
        SlashCommand {
            name: "search",
            summary: "Public web search into a markdown report",
        },
        SlashCommand {
            name: "report",
            summary: "Write the visible transcript into a markdown report",
        },
        SlashCommand {
            name: "hardware",
            summary: "Open the System hardware tab",
        },
        SlashCommand {
            name: "system",
            summary: "Open the System app",
        },
        SlashCommand {
            name: "log",
            summary: "Open the System log",
        },
        SlashCommand {
            name: "settings",
            summary: "Open System settings",
        },
        SlashCommand {
            name: "provider",
            summary: "Open text and voice provider login",
        },
        SlashCommand {
            name: "model",
            summary: "Pick a model, or /model <id>",
        },
        SlashCommand {
            name: "models",
            summary: "Open the model picker",
        },
        SlashCommand {
            name: "brain",
            summary: "Open recall, or /brain <fact> to remember",
        },
        SlashCommand {
            name: "gmail",
            summary: "Open the Gmail MCP setup",
        },
        SlashCommand {
            name: "voice",
            summary: "Send the next turns as voice",
        },
        SlashCommand {
            name: "text",
            summary: "Send the next turns as text",
        },
        SlashCommand {
            name: "open",
            summary: "Launch an app by name",
        },
        SlashCommand {
            name: "clear",
            summary: "Clear the case desk chat, or the open report chat",
        },
        SlashCommand {
            name: "quit",
            summary: "Leave Argos",
        },
    ]
}

pub fn filter_commands(query: &str) -> Vec<&'static SlashCommand> {
    let q = query
        .trim()
        .trim_start_matches('/')
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase();
    slash_commands()
        .iter()
        .filter(|c| q.is_empty() || c.name.starts_with(&q))
        .collect()
}

/// Slash menu card. `pub(crate)` at the TUI boundary; this library copy is
/// what tests and the binary both build from.
pub fn slash_menu(query: &str) -> Card {
    let options = filter_commands(query)
        .into_iter()
        .map(|c| CardOption {
            id: c.name.to_string(),
            label: format!("/{name}", name = c.name),
            detail: c.summary.to_string(),
        })
        .collect();
    Card {
        title: "Commands".into(),
        options,
        selected: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cases() -> Vec<Case> {
        vec![
            Case {
                id: "case-1".into(),
                title: "Harbor logs".into(),
            },
            Case {
                id: "case-2".into(),
                title: "Harbor lights".into(),
            },
            Case {
                id: "case-10".into(),
                title: "Northwind".into(),
            },
        ]
    }

    #[test]
    fn exact_id_or_title() {
        let all = cases();
        assert_eq!(resolve_case(&all, "CASE-10").unwrap().title, "Northwind");
        assert_eq!(resolve_case(&all, "northwind").unwrap().id, "case-10");
    }

    #[test]
    fn unique_prefix() {
        let all = cases();
        assert_eq!(resolve_case(&all, "north").unwrap().id, "case-10");
        assert_eq!(resolve_case(&all, "case-10").unwrap().id, "case-10");
    }

    #[test]
    fn empty_or_ambiguous_is_none() {
        let all = cases();
        assert!(resolve_case(&all, "  ").is_none());
        assert!(resolve_case(&all, "harbor").is_none());
        assert!(resolve_case(&all, "case").is_none());
        assert!(resolve_case(&all, "missing").is_none());
    }

    #[test]
    fn card_option_count_and_menu() {
        let menu = slash_menu("/us");
        assert_eq!(menu.option_count(), 1);
        assert_eq!(menu.options[0].id, "use");
        assert_eq!(slash_menu("").option_count(), slash_commands().len());
    }
}
