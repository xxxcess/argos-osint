//! Decision thresholds, uncertainty handling, and policy enforcement.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::contracts::NormalizedAnswer;

/// Application outcome of evaluating a decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnforcementOutcome {
    Pass,
    Reject { reason: String },
    Uncertain { reason: String },
    Unavailable { reason: String },
}

impl EnforcementOutcome {
    /// True ONLY when the decision explicitly passed all checks and thresholds.
    /// Uncertain and Unavailable NEVER imply approval.
    pub fn is_approved(&self) -> bool {
        matches!(self, Self::Pass)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Reject { .. } => "reject",
            Self::Uncertain { .. } => "uncertain",
            Self::Unavailable { .. } => "unavailable",
        }
    }
}

/// Versioned threshold configuration per question or gate.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ThresholdProfile {
    pub min_probability: Option<f64>,
    pub min_margin: Option<f64>,
    pub min_confidence: Option<f64>,
    pub abstention_labels: Vec<String>,
}

impl Default for ThresholdProfile {
    fn default() -> Self {
        Self {
            min_probability: Some(0.50),
            min_margin: Some(0.10),
            min_confidence: None,
            abstention_labels: vec![
                "insufficient".into(),
                "ambiguous".into(),
                "no_match".into(),
                "unrelated".into(),
            ],
        }
    }
}

/// Policy engine applying threshold rules to normalized answers.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DecisionPolicy {
    pub default_profile: ThresholdProfile,
    pub question_profiles: BTreeMap<String, ThresholdProfile>,
}

impl DecisionPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_question_profile(
        mut self,
        question_id: impl Into<String>,
        profile: ThresholdProfile,
    ) -> Self {
        self.question_profiles.insert(question_id.into(), profile);
        self
    }

    /// Evaluate an answer against threshold and abstention rules.
    pub fn evaluate(&self, question_id: &str, answer: &NormalizedAnswer) -> EnforcementOutcome {
        let profile = self
            .question_profiles
            .get(question_id)
            .unwrap_or(&self.default_profile);

        let label = answer.label.trim();
        if label.is_empty() {
            return EnforcementOutcome::Unavailable {
                reason: "missing answer label".into(),
            };
        }

        // 1. Check if the answer selected an explicit abstention outcome
        if profile.abstention_labels.iter().any(|abs| abs == label) {
            return EnforcementOutcome::Uncertain {
                reason: format!("model selected abstention label '{}'", label),
            };
        }

        // 2. Check winning option probability if present (e.g. from Jev)
        if let (Some(min_p), Some(prob)) = (profile.min_probability, answer.probability) {
            if prob < min_p {
                return EnforcementOutcome::Uncertain {
                    reason: format!(
                        "winning probability {:.3} is below threshold {:.3}",
                        prob, min_p
                    ),
                };
            }
        }

        // 3. Check distribution margin between top-1 and top-2
        if let Some(min_margin) = profile.min_margin {
            if answer.probabilities.len() > 1 {
                let mut sorted_probs: Vec<f64> = answer.probabilities.values().copied().collect();
                sorted_probs.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
                let top1 = sorted_probs.first().copied().unwrap_or(0.0);
                let top2 = sorted_probs.get(1).copied().unwrap_or(0.0);
                let margin = top1 - top2;
                if margin < min_margin {
                    return EnforcementOutcome::Uncertain {
                        reason: format!(
                            "margin between top outcomes ({:.3}) is below required {:.3}",
                            margin, min_margin
                        ),
                    };
                }
            }
        }

        // 4. Check confidence floor if specified
        if let (Some(min_conf), Some(conf)) = (profile.min_confidence, answer.confidence) {
            if conf < min_conf {
                return EnforcementOutcome::Uncertain {
                    reason: format!(
                        "reported confidence {:.3} is below required {:.3}",
                        conf, min_conf
                    ),
                };
            }
        }

        EnforcementOutcome::Pass
    }
}
