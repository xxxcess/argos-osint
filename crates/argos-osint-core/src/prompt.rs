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
    /// The turn may use only the report text supplied with the question.
    pub evidence_only: bool,
    /// The included evidence is fact memories from completed reports, not the files.
    pub from_memory: bool,
    /// Tool results are already in the conversation. This call only writes.
    pub writer_only: bool,
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

{memory}{scope}"#,
        view = parts.view_name,
        context = parts.view_context.trim(),
        hardware = parts.hardware_line,
        modality = parts.modality,
        memory = memory,
        scope = if parts.from_memory {
            "SCOPE: Answer only from the fact memories included with this question. \
             Cite the report title given with each fact so the user knows which completed report to open. \
             Do not read report files, do not search, and do not start a new case. \
             If the facts do not cover the question, say so."
        } else if parts.evidence_only {
            "SCOPE: Answer only from the report evidence included with this question. \
             Paraphrase or quote that report. If it does not contain the answer, say that the report does not cover the question. \
             Do not search, do not use brain memories, do not invent sources, and do not start a new case."
        } else if parts.writer_only {
            "A tool caller already ran. Its results are plain text in the user message under TOOL RESULTS, alongside any material gathered before you were asked. Use that, the report list, and the open report chat. Answer the user. Do not call tools. Prefer a short answer, then the report. If a tool or API call failed, do not quote the error. Say that the step failed and that the detail is in the System log."
        } else {
            "When a public search was already run for this turn, use those hits. Call web_search, news_search, domain_lookup, social_search, or identity_lookup for another public source, and fetch_page for a URL. If the user asked you to remember a fact about themselves, call remember. Prefer a short answer, then the report. If a tool, search, or API call fails, do not quote the error. Say that the step failed and that the detail is in the System log."
        },
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
        let memories = vec![Memory::fact("1", "My name is Ada", "t")];
        let hits = recall(&memories, "who am I", 3);
        let prompt = system_prompt(&PromptParts {
            view_name: "Brain",
            view_context: "The brain list is open.",
            hardware_line: "macos arm64",
            memories: &hits,
            modality: "text",
            evidence_only: false,
            from_memory: false,
            writer_only: false,
        });
        assert!(prompt.contains("My name is Ada"));
        assert!(prompt.contains("ACTIVE VIEW: Brain"));
        assert!(!prompt.contains("app_password"));
        assert!(!prompt.contains("SCOPE:"));
    }

    #[test]
    fn evidence_scope_forbids_a_new_search() {
        let prompt = system_prompt(&PromptParts {
            view_name: "Report · Harbor",
            view_context: "The user selected one report.",
            hardware_line: "macos arm64",
            memories: &[],
            modality: "text",
            evidence_only: true,
            from_memory: false,
            writer_only: false,
        });
        assert!(prompt.contains("Answer only from the report evidence"));
        assert!(prompt.contains("Do not search"));
    }

    #[test]
    fn writer_reads_tool_results_as_text() {
        let prompt = system_prompt(&PromptParts {
            view_name: "Case Desk",
            view_context: "Three reports are listed.",
            hardware_line: "macos arm64",
            memories: &[],
            modality: "text",
            evidence_only: false,
            from_memory: false,
            writer_only: true,
        });
        assert!(prompt.contains("TOOL RESULTS"));
        assert!(prompt.contains("Do not call tools"));
        assert!(prompt.contains("Three reports are listed."));
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
