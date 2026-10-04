//! Admiralty-style source evaluation: Source Reliability (A–F) and Information
//! Credibility (1–6). Wikipedia WP:RSP feeds the letter axis; peer corroboration
//! feeds the digit. Together they scale claim confidence.

use serde::{Deserialize, Serialize};

/// Publisher track record (Who/What).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SourceReliability {
    A,
    B,
    C,
    D,
    E,
    F,
}

impl SourceReliability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::D => "D",
            Self::E => "E",
            Self::F => "F",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::A => "COMPLETELY RELIABLE",
            Self::B => "USUALLY RELIABLE",
            Self::C => "FAIRLY RELIABLE",
            Self::D => "NOT USUALLY RELIABLE",
            Self::E => "UNRELIABLE",
            Self::F => "CANNOT BE JUDGED",
        }
    }

    /// 0–1 meter weight for UI (F is unjudgeable, shown as empty).
    pub fn meter(self) -> f64 {
        match self {
            Self::A => 1.0,
            Self::B => 0.85,
            Self::C => 0.55,
            Self::D => 0.30,
            Self::E => 0.12,
            Self::F => 0.0,
        }
    }

    pub fn factor(self) -> f64 {
        match self {
            Self::A => 1.20,
            Self::B => 1.12,
            Self::C => 1.00,
            Self::D => 0.72,
            Self::E => 0.40,
            // Unvetted: do not invent a penalty.
            Self::F => 1.00,
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_uppercase().as_str() {
            "A" => Some(Self::A),
            "B" => Some(Self::B),
            "C" => Some(Self::C),
            "D" => Some(Self::D),
            "E" => Some(Self::E),
            "F" => Some(Self::F),
            _ => None,
        }
    }
}

/// Truth value of one piece of information (1–6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InformationCredibility {
    Confirmed = 1,
    ProbablyTrue = 2,
    PossiblyTrue = 3,
    DoubtfullyTrue = 4,
    Improbable = 5,
    CannotBeJudged = 6,
}

impl InformationCredibility {
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Confirmed => "CONFIRMED",
            Self::ProbablyTrue => "PROBABLY TRUE",
            Self::PossiblyTrue => "POSSIBLY TRUE",
            Self::DoubtfullyTrue => "DOUBTFULLY TRUE",
            Self::Improbable => "IMPROBABLE",
            Self::CannotBeJudged => "CANNOT BE JUDGED",
        }
    }

    pub fn meter(self) -> f64 {
        match self {
            Self::Confirmed => 1.0,
            Self::ProbablyTrue => 0.82,
            Self::PossiblyTrue => 0.58,
            Self::DoubtfullyTrue => 0.35,
            Self::Improbable => 0.15,
            Self::CannotBeJudged => 0.0,
        }
    }

    pub fn factor(self) -> f64 {
        match self {
            Self::Confirmed => 1.15,
            Self::ProbablyTrue => 1.05,
            Self::PossiblyTrue => 1.00,
            Self::DoubtfullyTrue => 0.75,
            Self::Improbable => 0.45,
            // Unjudgeable: neutral.
            Self::CannotBeJudged => 1.00,
        }
    }

    pub fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Confirmed),
            2 => Some(Self::ProbablyTrue),
            3 => Some(Self::PossiblyTrue),
            4 => Some(Self::DoubtfullyTrue),
            5 => Some(Self::Improbable),
            6 => Some(Self::CannotBeJudged),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmiraltyCode {
    pub reliability: SourceReliability,
    pub credibility: InformationCredibility,
}

impl AdmiraltyCode {
    pub fn new(reliability: SourceReliability, credibility: InformationCredibility) -> Self {
        Self {
            reliability,
            credibility,
        }
    }

    pub fn display(self) -> String {
        format!("{}{}", self.reliability.as_str(), self.credibility.as_u8())
    }
}

/// Scale a base confidence by Admiralty reliability × information credibility.
pub fn scale_confidence(base: f64, code: AdmiraltyCode) -> f64 {
    (base.clamp(0.0, 1.0) * code.reliability.factor() * code.credibility.factor()).clamp(0.0, 1.0)
}

/// Inputs used to assign Information Credibility (1–6).
#[derive(Clone, Copy, Debug)]
pub struct CredibilityInputs<'a> {
    pub classification: &'a str,
    pub confidence: f64,
    pub title_peers: u32,
    pub body_peers: u32,
    pub description_empty: bool,
    pub reliability: SourceReliability,
    pub has_conflict: bool,
}

/// Derive Information Credibility from corroboration signals.
pub fn information_credibility(input: CredibilityInputs<'_>) -> InformationCredibility {
    if input.reliability == SourceReliability::E || input.has_conflict {
        return InformationCredibility::Improbable;
    }
    if input.title_peers > 0 && input.classification == "fact" {
        return InformationCredibility::Confirmed;
    }
    if input.classification == "fact" || input.body_peers > 0 {
        return InformationCredibility::ProbablyTrue;
    }
    if input.reliability == SourceReliability::F
        && input.title_peers == 0
        && input.body_peers == 0
        && input.description_empty
    {
        return InformationCredibility::CannotBeJudged;
    }
    if input.confidence < 0.40 {
        return InformationCredibility::DoubtfullyTrue;
    }
    if input.classification == "inference" {
        return InformationCredibility::PossiblyTrue;
    }
    InformationCredibility::PossiblyTrue
}

/// Best (lowest digit) credibility among a set; 6 when empty.
pub fn best_credibility(values: &[InformationCredibility]) -> InformationCredibility {
    values
        .iter()
        .copied()
        .min_by_key(|item| item.as_u8())
        .unwrap_or(InformationCredibility::CannotBeJudged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_up_for_usually_reliable_confirmed() {
        let code = AdmiraltyCode::new(SourceReliability::B, InformationCredibility::Confirmed);
        let scaled = scale_confidence(0.80, code);
        assert!(scaled > 0.80);
        assert!(scaled <= 1.0);
        assert_eq!(code.display(), "B1");
    }

    #[test]
    fn scales_down_for_unreliable_improbable() {
        let code = AdmiraltyCode::new(SourceReliability::E, InformationCredibility::Improbable);
        let scaled = scale_confidence(0.90, code);
        assert!(scaled < 0.90);
    }

    #[test]
    fn unlisted_reliability_is_neutral() {
        let code = AdmiraltyCode::new(
            SourceReliability::F,
            InformationCredibility::CannotBeJudged,
        );
        assert!((scale_confidence(0.70, code) - 0.70).abs() < f64::EPSILON);
    }

    #[test]
    fn title_peers_yield_confirmed() {
        let rating = information_credibility(CredibilityInputs {
            classification: "fact",
            confidence: 0.7,
            title_peers: 1,
            body_peers: 0,
            description_empty: false,
            reliability: SourceReliability::B,
            has_conflict: false,
        });
        assert_eq!(rating, InformationCredibility::Confirmed);
    }

    #[test]
    fn unreliable_source_is_improbable() {
        let rating = information_credibility(CredibilityInputs {
            classification: "fact",
            confidence: 0.9,
            title_peers: 2,
            body_peers: 0,
            description_empty: false,
            reliability: SourceReliability::E,
            has_conflict: false,
        });
        assert_eq!(rating, InformationCredibility::Improbable);
    }
}
