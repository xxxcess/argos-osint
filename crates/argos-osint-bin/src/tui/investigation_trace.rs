//! Shared investigation trace-to-block adapter and disclosure state.

use argos_osint_core::investigation::trace::InvestigationEvent;

use super::app::App;
use super::recon_parts::InvestigationPart;
use super::ui::ChatBlock;

/// Adapts a durable InvestigationEvent into a presentation ChatBlock.
pub fn event_to_chat_block(event: &InvestigationEvent, message_index: Option<usize>) -> ChatBlock {
    let ev_type = event.event_type.as_str();

    match ev_type {
        "thinking" | "reasoning" | "ThinkingEmitted" => {
            let role = if event.role.is_empty() {
                "planner"
            } else {
                event.role.as_str()
            };
            let model = if event.model.is_empty() {
                "model"
            } else {
                event.model.as_str()
            };
            let elapsed = if !event.occurrence_time.is_empty() {
                &event.occurrence_time
            } else {
                "live"
            };

            let title = format!("Thinking… · {role} · {model} · {elapsed}");
            let text = event
                .payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or(&event.summary)
                .to_string();

            ChatBlock {
                part: InvestigationPart::Thinking,
                key: format!("thinking:{}", event.id),
                title,
                body: text,
                collapsible: true,
                message_index,
                has_memory: false,
            }
        }
        "decision" | "DecisionEvaluated" | "gate" | "GateEvaluated" => {
            let status = if event.status.is_empty() {
                "pass"
            } else {
                event.status.as_str()
            };
            let model = if event.model.is_empty() {
                "Jev"
            } else {
                event.model.as_str()
            };
            let title = if status == "running" || status == "deciding" {
                format!("Deciding… · {model} · {}", event.summary)
            } else {
                format!("Decision · {} [{status}] · {model}", event.summary)
            };
            let body = serde_json::to_string_pretty(&event.payload).unwrap_or_default();

            ChatBlock {
                part: InvestigationPart::GateValidation,
                key: format!("decision:{}", event.id),
                title,
                body,
                collapsible: true,
                message_index,
                has_memory: false,
            }
        }
        "passage" | "PassageCurated" => {
            let title = format!("Evidence passage · {}", event.summary);
            let quote = event
                .payload
                .get("quote")
                .or_else(|| event.payload.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or(&event.summary)
                .to_string();

            ChatBlock {
                part: InvestigationPart::EvidencePassage,
                key: format!("evidence:{}", event.id),
                title,
                body: quote,
                collapsible: true,
                message_index,
                has_memory: false,
            }
        }
        "claim" | "ClaimAssessed" => {
            let title = format!("Claim assessment · {}", event.summary);
            let body = serde_json::to_string_pretty(&event.payload).unwrap_or_default();

            ChatBlock {
                part: InvestigationPart::DirectiveAssessment,
                key: format!("claim:{}", event.id),
                title,
                body,
                collapsible: true,
                message_index,
                has_memory: false,
            }
        }
        "role" | "RoleDecision" => {
            let title = format!("Role decision · {} [{}]", event.summary, event.role);
            let body = serde_json::to_string_pretty(&event.payload).unwrap_or_default();

            ChatBlock {
                part: InvestigationPart::RoleDecision,
                key: format!("role:{}", event.id),
                title,
                body,
                collapsible: true,
                message_index,
                has_memory: false,
            }
        }
        "handoff" | "Handoff" => {
            let title = format!("Handoff · {}", event.summary);
            let body = serde_json::to_string_pretty(&event.payload).unwrap_or_default();

            ChatBlock {
                part: InvestigationPart::Handoff,
                key: format!("handoff:{}", event.id),
                title,
                body,
                collapsible: true,
                message_index,
                has_memory: false,
            }
        }
        _ => {
            let title = format!("Activity · {}", event.summary);
            let body = serde_json::to_string_pretty(&event.payload).unwrap_or_default();

            ChatBlock {
                part: InvestigationPart::ToolActivity,
                key: format!("activity:{}", event.id),
                title,
                body,
                collapsible: true,
                message_index,
                has_memory: false,
            }
        }
    }
}

/// Checks whether a block's thinking row is expanded, considering both individual block expansion and global /thinking toggle.
#[allow(dead_code)]
pub fn is_thinking_expanded(app: &App, block_key: &str) -> bool {
    app.show_thinking || app.expanded.contains(block_key)
}

/// Formats the disclosure indicator for a thinking row.
#[allow(dead_code)]
pub fn format_thinking_header(title: &str, expanded: bool) -> String {
    let indicator = if expanded { "▼" } else { "▶" };
    format!("{indicator} {title}")
}
