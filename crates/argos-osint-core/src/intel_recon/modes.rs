//! Section plans for the four Intel Recon report modes.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportMode {
    Verify,
    Explain,
    AssessOutlook,
    FullAssessment,
}

impl ReportMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verify => "verify",
            Self::Explain => "explain",
            Self::AssessOutlook => "assess_outlook",
            Self::FullAssessment => "full_assessment",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Verify => "Verify",
            Self::Explain => "Explain",
            Self::AssessOutlook => "Assess Outlook",
            Self::FullAssessment => "Full Assessment",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Verify => "Determine which assertions are supported, disputed or unresolved.",
            Self::Explain => "Establish actors, events, history, relationships and significance.",
            Self::AssessOutlook => "Test alternatives and assess conditional future developments.",
            Self::FullAssessment => "Run Verify, Explain and Assess Outlook using shared evidence.",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "verify" => Some(Self::Verify),
            "explain" => Some(Self::Explain),
            "assess_outlook" | "outlook" => Some(Self::AssessOutlook),
            "full_assessment" | "full" => Some(Self::FullAssessment),
            _ => None,
        }
    }

    pub fn all() -> [Self; 4] {
        [
            Self::Verify,
            Self::Explain,
            Self::AssessOutlook,
            Self::FullAssessment,
        ]
    }
}

/// Markdown section headings and analyst framing for a chat Recon synthesis answer.
pub fn chat_response_spec(mode: ReportMode) -> String {
    let mode = normalize_chat_mode(mode);
    let headings: Vec<String> = chat_section_titles(mode)
        .into_iter()
        .map(|title| format!("## {title}"))
        .collect();
    format!(
        "Recon mode: {} — {}.\n\
Write the answer as Markdown with these headings in order (omit a heading only when \
there is nothing evidence-backed to say under it):\n{}\n\
After the sections, add one line per directive (D1:, D2:, …) saying whether it was met, \
partly met, or not met, with citations.",
        mode.title(),
        mode.description(),
        headings.join("\n")
    )
}

fn normalize_chat_mode(mode: ReportMode) -> ReportMode {
    if mode == ReportMode::FullAssessment {
        ReportMode::Explain
    } else {
        mode
    }
}

fn chat_section_titles(mode: ReportMode) -> Vec<&'static str> {
    section_plan(mode)
        .into_iter()
        .map(|section| match (mode, section.key) {
            (ReportMode::Verify, "assertions") => "Claims and Evidence Assessments",
            _ => section.title,
        })
        .collect()
}

/// Compact mode guidance for directive derivation and tool picking (not synthesis prose).
pub fn investigation_mode_spec(mode: ReportMode) -> String {
    let mode = normalize_chat_mode(mode);
    let sections = chat_section_titles(mode).join("; ");
    let focus = match mode {
        ReportMode::Verify => {
            "Investigate assertions and consequential claims. Prefer tools that surface \
primary reporting, corroboration, contradictions, corrections, and source reliability. \
Do not expand into wide geopolitical overview unless the prompt requires it."
        }
        ReportMode::Explain => {
            "Map the larger situation: actors and roles, event timeline and locations, \
drivers, relationships, and implications. Prefer discovery and news/context tools that \
establish who is involved, what happened when, and how entities connect."
        }
        ReportMode::AssessOutlook => {
            "Establish the baseline, then competing scenarios with indicators and \
disconfirming evidence. Prefer recent news and monitoring tools that support or refute \
conditional outlooks; avoid speculative tools that cannot ground indicators."
        }
        ReportMode::FullAssessment => unreachable!("normalized away"),
    };
    format!(
        "Recon mode: {} — {}.\n\
Section priorities to support: {}.\n\
Investigation focus: {}",
        mode.title(),
        mode.description(),
        sections,
        focus
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionPlan {
    pub key: &'static str,
    pub title: &'static str,
}

pub fn section_plan(mode: ReportMode) -> Vec<SectionPlan> {
    match mode {
        ReportMode::Verify => vec![
            SectionPlan {
                key: "bluf",
                title: "Key Judgments / BLUF",
            },
            SectionPlan {
                key: "assertions",
                title: "Article Assertions and Evidence Assessments",
            },
            SectionPlan {
                key: "sources",
                title: "Source Reliability and Independent Corroboration",
            },
            SectionPlan {
                key: "inferences",
                title: "Inferences, Context and Link Validation",
            },
            SectionPlan {
                key: "contradictions",
                title: "Contradictions, Corrections and Unresolved Gaps",
            },
            SectionPlan {
                key: "coverage",
                title: "Sources and Coverage",
            },
        ],
        ReportMode::Explain => vec![
            SectionPlan {
                key: "bluf",
                title: "Key Judgments / BLUF",
            },
            SectionPlan {
                key: "actors",
                title: "Actors, Roles and Relevant Relationships",
            },
            SectionPlan {
                key: "timeline",
                title: "Event Timeline and Locations",
            },
            SectionPlan {
                key: "background",
                title: "Background, Drivers and Alternative Explanations",
            },
            SectionPlan {
                key: "implications",
                title: "Implications and Affected Parties",
            },
            SectionPlan {
                key: "gaps",
                title: "Gaps, Sources and Coverage",
            },
        ],
        ReportMode::AssessOutlook => vec![
            SectionPlan {
                key: "bluf",
                title: "Key Judgments and Forecast Horizon",
            },
            SectionPlan {
                key: "baseline",
                title: "Established Baseline and Critical Uncertainties",
            },
            SectionPlan {
                key: "scenarios",
                title: "Competing Explanations and Scenarios",
            },
            SectionPlan {
                key: "indicators",
                title: "Indicators, Triggers and Disconfirming Evidence",
            },
            SectionPlan {
                key: "outlook",
                title: "Conditional Outlook and Consequences",
            },
            SectionPlan {
                key: "assumptions",
                title: "Assumptions, Gaps, Sources and Coverage",
            },
        ],
        ReportMode::FullAssessment => vec![
            SectionPlan {
                key: "bluf",
                title: "Executive Intelligence Brief / BLUF",
            },
            SectionPlan {
                key: "verified",
                title: "Verified Claims and Evidence",
            },
            SectionPlan {
                key: "disputes",
                title: "Source Assessment, Disputes and Corrections",
            },
            SectionPlan {
                key: "actors",
                title: "Actors and Relationships",
            },
            SectionPlan {
                key: "timeline",
                title: "Event Timeline and Geographic Context",
            },
            SectionPlan {
                key: "drivers",
                title: "Drivers, Context and Competing Explanations",
            },
            SectionPlan {
                key: "implications",
                title: "Implications and Affected Parties",
            },
            SectionPlan {
                key: "scenarios",
                title: "Scenarios and Conditional Outlook",
            },
            SectionPlan {
                key: "indicators",
                title: "Indicators and Collection Priorities",
            },
            SectionPlan {
                key: "gaps",
                title: "Gaps, Sources and Complete Element Coverage",
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_has_bluf_first_and_sources_last() {
        for mode in ReportMode::all() {
            let plan = section_plan(mode);
            assert!(!plan.is_empty(), "{mode:?}");
            assert_eq!(plan[0].key, "bluf");
            let last = plan.last().unwrap().key;
            assert!(
                last == "coverage" || last == "gaps" || last == "assumptions",
                "{mode:?} last={last}"
            );
        }
    }

    #[test]
    fn full_assessment_has_ten_sections() {
        assert_eq!(section_plan(ReportMode::FullAssessment).len(), 10);
    }

    #[test]
    fn chat_response_spec_lists_mode_headings() {
        let spec = chat_response_spec(ReportMode::Explain);
        assert!(spec.contains("Recon mode: Explain"));
        assert!(spec.contains("## Key Judgments / BLUF"));
        assert!(spec.contains("## Actors, Roles and Relevant Relationships"));
        assert!(spec.contains("D1:"));
        let verify = chat_response_spec(ReportMode::Verify);
        assert!(verify.contains("## Claims and Evidence Assessments"));
        assert!(!verify.contains("Article Assertions"));
    }

    #[test]
    fn investigation_mode_spec_guides_directives_and_picker() {
        let explain = investigation_mode_spec(ReportMode::Explain);
        assert!(explain.contains("Recon mode: Explain"));
        assert!(explain.contains("Actors, Roles and Relevant Relationships"));
        assert!(explain.contains("who is involved"));
        let outlook = investigation_mode_spec(ReportMode::AssessOutlook);
        assert!(outlook.contains("competing scenarios"));
        assert!(outlook.contains("Indicators"));
    }
}
