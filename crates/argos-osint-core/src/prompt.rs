//! System prompt for the user-centric loop. Recall, the active view, and a
//! one-line hardware profile are injected on every turn. Gmail passwords and
//! API keys are not.

use crate::brain::{self, ScoredMemory};

#[derive(Clone, Debug)]
pub struct PromptParts<'a> {
    pub view_name: &'a str,
    pub view_context: &'a str,
    pub hardware_line: &'a str,
    pub memories: &'a [ScoredMemory],
    pub modality: &'a str,
}

pub fn system_prompt(parts: &PromptParts<'_>) -> String {
    let memory = brain::format_injection(parts.memories);
    format!(
        r#"You are Argos, a terminal research analyst sitting with the user at one keyboard.
The prompt the user just sent belongs to the view that is open. Answer that view.
Write in plain markdown. Cite public URLs when you use them. Do not invent sources.

You research publicly available information and file markdown reports.
You do not help with unauthorized access, credential theft, malware, covert surveillance, or breaking into accounts.
Gmail tools, when they are offered, read only the mailbox the user connected with an app password.

ACTIVE VIEW: {view}
{context}

HOST: {hardware}

MODALITY: {modality}
The user may be speaking or typing. Treat the latest user message as what they just said.

{memory}When a public search was already run for this turn, use those hits. If you need another public page, call fetch_page. If the user asked you to remember a fact about themselves, call remember. Prefer a short answer, then the report."#,
        view = parts.view_name,
        context = parts.view_context.trim(),
        hardware = parts.hardware_line,
        modality = parts.modality,
        memory = memory,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Investigate,
    Remember,
    Hardware,
    Gmail,
    Chat,
}

pub fn classify(text: &str) -> Intent {
    let l = text.trim().to_lowercase();
    if l.starts_with("remember ") || l.starts_with("remember:") || l.starts_with("/brain ") {
        Intent::Remember
    } else if l.contains("vram")
        || l.contains("this machine")
        || l.contains("this host")
        || l.contains("hardware profile")
        || (l.contains("cpu") && l.contains("ram"))
    {
        Intent::Hardware
    } else if l.contains("inbox") || l.contains("gmail") || l.contains("unread mail") {
        Intent::Gmail
    } else if l.contains("search")
        || l.contains("investigate")
        || l.contains("osint")
        || l.contains("report on")
        || l.contains("who is")
        || l.contains("find ")
        || l.contains("look up")
    {
        Intent::Investigate
    } else {
        Intent::Chat
    }
}

pub fn remember_text(text: &str) -> String {
    let t = text.trim();
    for prefix in ["remember:", "remember ", "/brain "] {
        if let Some(rest) = t.get(prefix.len()..) {
            if t.to_lowercase().starts_with(prefix) {
                return rest.trim().to_string();
            }
        }
    }
    t.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::{recall, Memory};

    #[test]
    fn injection_includes_recalled_name() {
        let memories = vec![Memory {
            id: "1".into(),
            text: "My name is Ada".into(),
            created_at: "t".into(),
        }];
        let hits = recall(&memories, "who am I", 3);
        let prompt = system_prompt(&PromptParts {
            view_name: "Brain",
            view_context: "The brain list is open.",
            hardware_line: "macos arm64",
            memories: &hits,
            modality: "text",
        });
        assert!(prompt.contains("My name is Ada"));
        assert!(prompt.contains("ACTIVE VIEW: Brain"));
        assert!(!prompt.contains("app_password"));
    }

    #[test]
    fn classifies_user_requests() {
        assert_eq!(classify("remember that I prefer cites"), Intent::Remember);
        assert_eq!(
            classify("search the public record for the harbor"),
            Intent::Investigate
        );
        assert_eq!(classify("what is my gmail inbox"), Intent::Gmail);
        assert_eq!(
            classify("how much vram does this machine have"),
            Intent::Hardware
        );
        assert_eq!(classify("hello"), Intent::Chat);
    }
}
