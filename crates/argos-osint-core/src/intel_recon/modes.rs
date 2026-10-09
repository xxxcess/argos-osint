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

    pub fn style_guide(self) -> &'static str {
        match self {
            Self::Verify => {
                "This mode should read like a verification brief. Keep background only when it changes a claim’s assessment."
            }
            Self::Explain => {
                "The main repetition risk is describing the same event in the timeline, background, and implications. Each section should add a different analytical contribution."
            }
            Self::AssessOutlook => {
                "Keep scenarios distinct from the outlook: scenarios describe alternatives; the outlook weighs their consequences under stated conditions. Avoid unsupported numerical probabilities."
            }
            Self::FullAssessment => {
                "Full Assessment should integrate the other modes around shared evidence. Its ten sections should not become three reports pasted together."
            }
        }
    }
}

/// Markdown section headings and analyst framing for a chat Recon synthesis answer.
pub fn chat_response_spec(mode: ReportMode) -> String {
    let mode = normalize_chat_mode(mode);
    let mut sections = Vec::new();
    for section in section_plan(mode) {
        let heading = match (mode, section.key) {
            (ReportMode::Verify, "assertions") => "Claims and Evidence Assessments",
            _ => section.title,
        };
        sections.push(format!("## {heading}\n- {}", section.guideline));
    }
    format!(
        "Recon mode: {} — {}.\n\
Mode writing style and guidelines:\n{}\n\
The writing targets below are recommendations—soft limits that should expand when necessary to preserve material evidence or uncertainty.\n\
Write the answer as Markdown with these headings in order (omit a heading only when there is nothing evidence-backed to say under it):\n\n{}\n\n\
After the sections, add one line per directive (D1:, D2:, …) saying whether it was met, partly met, or not met, with citations.",
        mode.title(),
        mode.description(),
        mode.style_guide(),
        sections.join("\n\n")
    )
}

fn normalize_chat_mode(mode: ReportMode) -> ReportMode {
    mode
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
        ReportMode::FullAssessment => {
            "Shared collection to verify core assertions and explain the larger situation, \
reconciling baseline facts before evaluating competing scenarios and outlook indicators. \
Integrate primary reporting, actor relationships, and forward-looking triggers without duplicate collection."
        }
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
    pub guideline: &'static str,
}

pub fn section_guideline(mode: ReportMode, section_key: &str) -> Option<&'static str> {
    section_plan(mode)
        .into_iter()
        .find(|s| s.key == section_key)
        .map(|s| s.guideline)
}

pub fn section_plan(mode: ReportMode) -> Vec<SectionPlan> {
    match mode {
        ReportMode::Verify => vec![
            SectionPlan {
                key: "bluf",
                title: "Key Judgments / BLUF",
                guideline: "2–3 bullets stating the overall finding, strongest supporting evidence, and principal uncertainty.",
            },
            SectionPlan {
                key: "assertions",
                title: "Article Assertions and Evidence Assessments",
                guideline: "One bullet per consequential claim: claim → supported/disputed/unresolved → brief reason → citation.",
            },
            SectionPlan {
                key: "sources",
                title: "Source Reliability and Independent Corroboration",
                guideline: "2–4 bullets explaining source quality, independence, and meaningful corroboration. Avoid repeating the claims.",
            },
            SectionPlan {
                key: "inferences",
                title: "Inferences, Context and Link Validation",
                guideline: "2–4 bullets identifying inferred relationships, their supporting evidence, and limits. Clearly distinguish inference from observation.",
            },
            SectionPlan {
                key: "contradictions",
                title: "Contradictions, Corrections and Unresolved Gaps",
                guideline: "One bullet per material conflict: disagreement → effect on judgment → unresolved point.",
            },
            SectionPlan {
                key: "coverage",
                title: "Sources and Coverage",
                guideline: "Compact source and coverage bullets. Account for assessed and unresolved claims without retelling the findings.",
            },
        ],
        ReportMode::Explain => vec![
            SectionPlan {
                key: "bluf",
                title: "Key Judgments / BLUF",
                guideline: "2–3 bullets answering what happened, why it matters, and the main explanatory uncertainty.",
            },
            SectionPlan {
                key: "actors",
                title: "Actors, Roles and Relevant Relationships",
                guideline: "One bullet per relevant actor: name → role → consequential relationship. Exclude incidental people and publishers.",
            },
            SectionPlan {
                key: "timeline",
                title: "Event Timeline and Locations",
                guideline: "Numbered chronological entries: date → event → location → significance, with citations.",
            },
            SectionPlan {
                key: "background",
                title: "Background, Drivers and Alternative Explanations",
                guideline: "3–5 bullets covering necessary context, main drivers, and credible alternatives. Separate established causes from proposed explanations.",
            },
            SectionPlan {
                key: "implications",
                title: "Implications and Affected Parties",
                guideline: "2–4 bullets: affected party → consequence → supporting basis.",
            },
            SectionPlan {
                key: "gaps",
                title: "Gaps, Sources and Coverage",
                guideline: "Short bullets identifying missing information, its effect on the explanation, and source coverage.",
            },
        ],
        ReportMode::AssessOutlook => vec![
            SectionPlan {
                key: "bluf",
                title: "Key Judgments and Forecast Horizon",
                guideline: "2–3 bullets stating the horizon, central outlook, and main uncertainty.",
            },
            SectionPlan {
                key: "baseline",
                title: "Established Baseline and Critical Uncertainties",
                guideline: "3–5 bullets explicitly labeled Established or Uncertain.",
            },
            SectionPlan {
                key: "scenarios",
                title: "Competing Explanations and Scenarios",
                guideline: "Usually 2–3 numbered scenarios: scenario → conditions → supporting/challenging evidence → potential outcome. Do not invent alternatives to meet a count.",
            },
            SectionPlan {
                key: "indicators",
                title: "Indicators, Triggers and Disconfirming Evidence",
                guideline: "One bullet per observable signal: indicator → scenario it supports or weakens → why.",
            },
            SectionPlan {
                key: "outlook",
                title: "Conditional Outlook and Consequences",
                guideline: "2–4 “If … then …” bullets connecting conditions to outcomes within the stated horizon.",
            },
            SectionPlan {
                key: "assumptions",
                title: "Assumptions, Gaps, Sources and Coverage",
                guideline: "Compact labeled bullets for assumptions, forecast-sensitive gaps, and evidence coverage.",
            },
        ],
        ReportMode::FullAssessment => vec![
            SectionPlan {
                key: "bluf",
                title: "Executive Intelligence Brief / BLUF",
                guideline: "3–4 bullets covering the central finding, explanation, conditional outlook, and largest uncertainty.",
            },
            SectionPlan {
                key: "verified",
                title: "Verified Claims and Evidence",
                guideline: "One concise cited bullet per verified claim.",
            },
            SectionPlan {
                key: "disputes",
                title: "Source Assessment, Disputes and Corrections",
                guideline: "Bullets containing source limitations, contested claims, and corrections.",
            },
            SectionPlan {
                key: "actors",
                title: "Actors and Relationships",
                guideline: "One bullet per material actor or relationship.",
            },
            SectionPlan {
                key: "timeline",
                title: "Event Timeline and Geographic Context",
                guideline: "Numbered chronological entries; include geographic context only when consequential.",
            },
            SectionPlan {
                key: "drivers",
                title: "Drivers, Context and Competing Explanations",
                guideline: "3–5 explanatory bullets.",
            },
            SectionPlan {
                key: "implications",
                title: "Implications and Affected Parties",
                guideline: "2–4 consequence-focused bullets.",
            },
            SectionPlan {
                key: "scenarios",
                title: "Scenarios and Conditional Outlook",
                guideline: "Usually 2–3 numbered scenarios with conditions, outcomes, and uncertainty.",
            },
            SectionPlan {
                key: "indicators",
                title: "Indicators and Collection Priorities",
                guideline: "Prioritized numbered items identifying observable signals or evidence needed to resolve consequential gaps.",
            },
            SectionPlan {
                key: "gaps",
                title: "Gaps, Sources and Complete Element Coverage",
                guideline: "Compact coverage accounting. Preserve every required element’s status, even when this section needs more entries.",
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

    #[test]
    fn every_mode_section_has_guideline() {
        for mode in ReportMode::all() {
            assert!(!mode.style_guide().is_empty(), "{mode:?}");
            for section in section_plan(mode) {
                assert!(!section.guideline.is_empty(), "{mode:?} {}", section.key);
                assert_eq!(
                    section_guideline(mode, section.key),
                    Some(section.guideline)
                );
            }
        }
    }

    #[test]
    fn chat_response_spec_embeds_section_guidelines_and_style_guide() {
        for mode in ReportMode::all() {
            let spec = chat_response_spec(mode);
            assert!(spec.contains(mode.style_guide()), "{mode:?}");
            assert!(spec.contains("soft limits"));
            for section in section_plan(mode) {
                assert!(spec.contains(section.guideline), "{mode:?} {}", section.key);
            }
        }
    }
}
