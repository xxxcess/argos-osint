//! One-turn local parsing and the bounded values retained with verified replies.

use fleetworks_contracts::chat::{ChatSourceKind, ChatSourceReference};
use fleetworks_contracts::readiness::ReadinessCheckId;
use serde::{Deserialize, Serialize};

use super::{Span, Status};

pub(super) const MAX_INPUT: usize = fleetworks_contracts::chat::MAX_INPUT_BYTES;
const MAX_FEATURES: usize = 12;

/// Time and optional language supplied by the calling workstation context.
#[derive(Clone, Debug)]
pub struct Context {
    pub reference_ms: i64,
    /// Minutes east of UTC, or the workstation calendar when absent.
    pub timezone_offset_minutes: Option<i32>,
    pub language_hint: Option<String>,
}

impl Context {
    /// Use UTC when the client supplies no timezone.
    #[must_use]
    pub fn utc(reference_ms: i64) -> Self {
        Self {
            reference_ms,
            timezone_offset_minutes: Some(0),
            language_hint: None,
        }
    }

    /// Resolve calendar dates in the Embedded workstation's local timezone.
    #[must_use]
    pub fn workstation(reference_ms: i64) -> Self {
        Self {
            timezone_offset_minutes: None,
            ..Self::utc(reference_ms)
        }
    }

    pub(super) fn supports_language(&self, question: &str) -> bool {
        if let Some(language) = self.language_hint.as_deref() {
            return language.len() <= 35
                && language
                    .split('-')
                    .next()
                    .is_some_and(|primary| primary.eq_ignore_ascii_case("en"));
        }
        // Without a language hint, decline script-only inputs that the English
        // tokenizer cannot interpret. Mixed-script labels still get position hints.
        question.bytes().any(|byte| byte.is_ascii_alphabetic())
            || !question.chars().any(char::is_alphabetic)
    }
}

/// A parser result references the original UTF-8 input. No raw text is retained here.
#[derive(Clone, Debug)]
pub struct Features {
    pub status: Status,
    pub hints: Vec<Hint>,
    pub subjects: Vec<Span>,
    pub question_words: Vec<Span>,
    pub references: Vec<Span>,
    pub corrections: Vec<Span>,
    pub negations: Vec<Span>,
    pub exclusions: Vec<Span>,
    pub temporal: Vec<TemporalCandidate>,
}

impl Features {
    /// A parser outcome without features. Callers can fall back to established routing.
    #[must_use]
    pub fn terminal(status: Status) -> Self {
        Self {
            status,
            hints: Vec::new(),
            subjects: Vec::new(),
            question_words: Vec::new(),
            references: Vec::new(),
            corrections: Vec::new(),
            negations: Vec::new(),
            exclusions: Vec::new(),
            temporal: Vec::new(),
        }
    }
    /// Verify that all feature positions refer to original UTF-8 input within bounds.
    #[must_use]
    pub fn validate(&self, question: &str) -> bool {
        fn valid(spans: &[Span], question: &str) -> bool {
            spans.len() <= MAX_FEATURES && spans.iter().all(|span| span.slice(question).is_some())
        }

        question.len() <= MAX_INPUT
            && self.hints.len() <= MAX_FEATURES
            && self
                .hints
                .iter()
                .all(|hint| hint.span.slice(question).is_some())
            && self.temporal.len() <= MAX_FEATURES
            && self
                .temporal
                .iter()
                .all(|candidate| candidate.span.slice(question).is_some())
            && [
                &self.subjects,
                &self.question_words,
                &self.references,
                &self.corrections,
                &self.negations,
                &self.exclusions,
            ]
            .into_iter()
            .all(|spans| valid(spans, question))
    }
}

/// A bounded entity or part-of-speech hint. Hints never confer resource identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Hint {
    pub span: Span,
    pub kind: HintKind,
}

/// Classes of parser hints useful to conversation routing.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum HintKind {
    Entity,
    Noun,
    Verb,
}

/// A recognized time expression can be current, a concrete interval, or ambiguous.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TemporalCandidate {
    pub span: Span,
    pub scope: TemporalScope,
}

/// Explicit ambiguity keeps an unresolvable historical request out of current routing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TemporalScope {
    Current,
    Historical(Interval),
    Ambiguous,
}

/// Half-open history interval with the reference instant and timezone used to resolve it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Interval {
    pub start_ms: i64,
    pub end_ms: i64,
    pub reference_ms: i64,
    pub timezone: String,
}

/// A locally resolved, allowlisted question type.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    ExplainSelected,
    Readiness(ReadinessCheckId),
    ReadinessExplanation(ReadinessCheckId),
    StatusChecks(CheckSelection),
    Metric(Metric),
    HostResources,
    JobsSummary,
    RunningJobList,
    ContextStatus(ChatSourceKind),
    ContextLogs(ChatSourceKind),
    Configuration,
    Summary,
}

/// Readiness check categories that can be reused in a follow-up.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckSelection {
    All,
    Passed,
    NotPassed,
    Unresolved,
    Failed,
}

/// Metrics already supported by workstation routing.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    Cpu,
    Memory,
    Disk,
    QueuedJobs,
    RunningJobs,
    RetryingJobs,
    FailedJobs,
}

/// What the user wants to know about their selected operational subject.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Goal {
    Status,
    Explanation,
    Listing,
    Measurement,
    Summary,
}

/// Explicit limits on suggestions and actions.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Constraint {
    NoSuggestions,
    NoActions,
}

/// Reusable context that contains no arbitrary wording.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Focus {
    pub intent: Intent,
    pub source: Option<ChatSourceReference>,
    pub goal: Goal,
    pub temporal: Option<Interval>,
    pub constraints: Vec<Constraint>,
}

impl Focus {
    /// Summarize typed context using bounded labels and deliberately omit resource IDs.
    #[must_use]
    pub fn compact(&self) -> String {
        let subject = match self.intent {
            Intent::Readiness(check) | Intent::ReadinessExplanation(check) => check.as_str(),
            Intent::StatusChecks(_) => "readiness checks",
            Intent::Metric(Metric::Cpu) => "CPU usage",
            Intent::Metric(Metric::Memory) => "memory usage",
            Intent::Metric(Metric::Disk) => "disk usage",
            Intent::Metric(Metric::QueuedJobs) => "queued jobs",
            Intent::Metric(Metric::RunningJobs) | Intent::RunningJobList => "running jobs",
            Intent::Metric(Metric::RetryingJobs) => "retrying jobs",
            Intent::Metric(Metric::FailedJobs) => "failed jobs",
            Intent::HostResources => "host resources",
            Intent::JobsSummary => "job activity",
            Intent::ContextStatus(ChatSourceKind::Cycle)
            | Intent::ContextLogs(ChatSourceKind::Cycle) => "selected cycle",
            Intent::ContextStatus(_) | Intent::ContextLogs(_) => "selected job",
            Intent::ExplainSelected => match self.source.as_ref().map(|source| source.kind) {
                Some(ChatSourceKind::Job) => "selected job",
                Some(ChatSourceKind::Cycle) => "selected cycle",
                Some(ChatSourceKind::Readiness) => "selected readiness check",
                _ => "current selection",
            },
            Intent::Configuration => "workstation configuration",
            Intent::Summary => "workstation readiness",
        };
        let goal = match self.goal {
            Goal::Status => "status",
            Goal::Explanation => "explain",
            Goal::Listing => "list",
            Goal::Measurement => "measure",
            Goal::Summary => "summarize",
        };
        let mut result = format!("{goal} {subject}");
        if let Some(interval) = &self.temporal {
            use chrono::DateTime;
            use chrono::SecondsFormat;
            match (
                DateTime::from_timestamp_millis(interval.start_ms),
                DateTime::from_timestamp_millis(interval.end_ms),
            ) {
                (Some(start), Some(end)) => {
                    let start = start.to_rfc3339_opts(SecondsFormat::Millis, true);
                    let end = end.to_rfc3339_opts(SecondsFormat::Millis, true);
                    result.push_str(&format!(", recorded from {start} before {end}"));
                }
                _ => result.push_str(", recorded interval"),
            }
        } else {
            result.push_str(", current");
        }
        if !self.allows_suggestions() {
            result.push_str(", omit suggestions");
        }
        result
    }

    /// A negated action suppresses all advice when no narrower action is known.
    #[must_use]
    pub fn allows_suggestions(&self) -> bool {
        self.constraints.is_empty()
    }
}

/// A fixed clarification category for the approved reply menu.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClarificationReason {
    Subject,
    Time,
}

/// Retain bounded, uncited choices independently of answered findings.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PendingClarification {
    pub reason: ClarificationReason,
    pub choices: Vec<Focus>,
    /// The requested interval survives a clarification of the subject alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporal: Option<Interval>,
}

#[cfg(feature = "nlp")]
mod decode {
    use chrono::{DateTime, Days, FixedOffset, Local, NaiveDate, Offset, TimeDelta, TimeZone, Utc};

    use crate::nlp::spans;

    use super::{
        Context, Features, Hint, HintKind, Interval, Span, Status, TemporalCandidate,
        TemporalScope, MAX_FEATURES,
    };
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct JsSpan {
        start: usize,
        end: usize,
    }

    impl JsSpan {
        fn convert(self, text: &str) -> Option<Span> {
            let span = Span {
                start_byte: spans::utf16_to_byte(text, self.start)?,
                end_byte: spans::utf16_to_byte(text, self.end)?,
            };
            span.slice(text)?;
            Some(span)
        }
    }

    #[derive(Deserialize)]
    struct JsHint {
        span: JsSpan,
        kind: HintKind,
    }

    #[derive(Deserialize)]
    pub(super) struct JsFeatures {
        hints: Vec<JsHint>,
        subjects: Vec<JsSpan>,
        question_words: Vec<JsSpan>,
        references: Vec<JsSpan>,
        corrections: Vec<JsSpan>,
        negations: Vec<JsSpan>,
        exclusions: Vec<JsSpan>,
        temporal: Vec<JsSpan>,
    }

    fn convert_spans(spans: Vec<JsSpan>, text: &str) -> Option<Vec<Span>> {
        (spans.len() <= MAX_FEATURES)
            .then(|| spans.into_iter().map(|span| span.convert(text)).collect())?
    }

    impl JsFeatures {
        pub(super) fn into_features(self, text: &str, context: &Context) -> Option<Features> {
            if self.hints.len() > MAX_FEATURES {
                return None;
            }
            let hints = self
                .hints
                .into_iter()
                .map(|hint| {
                    Some(Hint {
                        span: hint.span.convert(text)?,
                        kind: hint.kind,
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            let temporal = convert_spans(self.temporal, text)?
                .into_iter()
                .map(|span| {
                    let phrase = span.slice(text).unwrap_or_default();
                    TemporalCandidate {
                        span,
                        scope: temporal_scope(phrase, context),
                    }
                })
                .collect();
            Some(Features {
                status: Status::Complete,
                hints,
                subjects: convert_spans(self.subjects, text)?,
                question_words: convert_spans(self.question_words, text)?,
                references: convert_spans(self.references, text)?,
                corrections: convert_spans(self.corrections, text)?,
                negations: convert_spans(self.negations, text)?,
                exclusions: convert_spans(self.exclusions, text)?,
                temporal,
            })
        }
    }

    pub(super) fn temporal_scope(phrase: &str, context: &Context) -> TemporalScope {
        let phrase = phrase
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase();
        let phrase = phrase.as_str();
        if matches!(phrase, "currently" | "now" | "right now") {
            return TemporalScope::Current;
        }
        interval(phrase, context)
            .map(TemporalScope::Historical)
            .unwrap_or(TemporalScope::Ambiguous)
    }

    fn interval(phrase: &str, context: &Context) -> Option<Interval> {
        let reference: DateTime<Utc> = DateTime::from_timestamp_millis(context.reference_ms)?;
        match context.timezone_offset_minutes {
            Some(minutes) => {
                let offset = FixedOffset::east_opt(minutes.checked_mul(60)?)?;
                calendar_interval(phrase, reference.with_timezone(&offset))
            }
            None => calendar_interval(phrase, reference.with_timezone(&Local)),
        }
    }

    fn calendar_interval<Tz: TimeZone>(phrase: &str, reference: DateTime<Tz>) -> Option<Interval> {
        let timezone = reference.timezone();
        let reference_ms = reference.timestamp_millis();
        let today = reference.date_naive();
        let yesterday = today.checked_sub_days(Days::new(1))?;
        let midnight = |date: NaiveDate| {
            timezone
                .from_local_datetime(&date.and_hms_opt(0, 0, 0)?)
                .earliest()
                .map(|time| time.timestamp_millis())
        };
        let date = |text| NaiveDate::parse_from_str(text, "%Y-%m-%d").ok();
        let now = reference_ms.checked_add(1)?;
        let (start_ms, end_ms) = match phrase {
            "today" | "since today" => (midnight(today)?, now),
            "yesterday" => (midnight(yesterday)?, midnight(today)?),
            "since yesterday" => (midnight(yesterday)?, now),
            _ => {
                if let Some(since) = phrase.strip_prefix("since ") {
                    (midnight(date(since)?)?, now)
                } else if let Some(range) = phrase.strip_prefix("between ") {
                    let (first, last) = range.split_once(" and ")?;
                    (
                        midnight(date(first)?)?,
                        midnight(date(last)?.checked_add_days(Days::new(1))?)?.min(now),
                    )
                } else if let Some(duration) = phrase
                    .strip_prefix("in the ")
                    .unwrap_or(phrase)
                    .strip_prefix("last ")
                    .or_else(|| {
                        phrase
                            .strip_prefix("in the ")
                            .unwrap_or(phrase)
                            .strip_prefix("past ")
                    })
                {
                    let (count, unit) = duration.trim().split_once(' ')?;
                    let count: i64 = count.parse().ok()?;
                    if count <= 0 {
                        return None;
                    }
                    let delta = match unit.trim().trim_end_matches('s') {
                        "minute" => TimeDelta::try_minutes(count)?,
                        "hour" => TimeDelta::try_hours(count)?,
                        "day" => TimeDelta::try_days(count)?,
                        _ => return None,
                    };
                    (reference.checked_sub_signed(delta)?.timestamp_millis(), now)
                } else {
                    let day = date(phrase.strip_prefix("on ").unwrap_or(phrase))?;
                    (
                        midnight(day)?,
                        midnight(day.checked_add_days(Days::new(1))?)?.min(now),
                    )
                }
            }
        };
        if start_ms >= end_ms {
            return None;
        }
        let offset = reference.offset().fix();
        Some(Interval {
            start_ms,
            end_ms,
            reference_ms,
            timezone: if offset.local_minus_utc() == 0 {
                "UTC".into()
            } else {
                offset.to_string()
            },
        })
    }

    #[cfg(test)]
    #[test]
    fn workstation_calendar_uses_local_midnight() {
        let reference = DateTime::parse_from_rfc3339("2026-11-26T12:00:00Z").unwrap();
        let context = Context::workstation(reference.timestamp_millis());
        let local_reference = reference.with_timezone(&Local);
        let yesterday = local_reference.date_naive().pred_opt().unwrap();
        let midnight = yesterday.and_hms_opt(0, 0, 0).unwrap();
        let expected = Local.from_local_datetime(&midnight).earliest().unwrap();

        let range = interval("since yesterday", &context).unwrap();
        assert_eq!(range.start_ms, expected.timestamp_millis());
        assert_eq!(range.end_ms, context.reference_ms + 1);
    }
}

#[cfg(feature = "nlp")]
pub(super) fn decode(text: &str, json: &str, context: &Context) -> Option<Features> {
    if json.len() > 16 * 1024 {
        return None;
    }
    let decoded: decode::JsFeatures = serde_json::from_str(json).ok()?;
    let features = decoded.into_features(text, context)?;
    features.validate(text).then_some(features)
}
