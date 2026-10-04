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
}
