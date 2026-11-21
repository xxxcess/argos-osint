use fleetworks_contracts::chat::ChatSourceKind;
use fleetworks_contracts::readiness::ReadinessCheckId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Question {
    Readiness(ReadinessCheckId),
    ReadinessExplanation(ReadinessCheckId),
    Metric(Metric),
    ContextStatus(ChatSourceKind),
    Summary,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Metric {
    Cpu,
    Memory,
    Disk,
    QueuedJobs,
    RunningJobs,
    RetryingJobs,
}

/// Match the whole question so additional subjects or qualifiers cannot silently change its scope.
pub(super) fn classify(question: &str) -> Question {
    let normalized = question
        .to_ascii_lowercase()
        .replace(['_', '-'], " ")
        .replace('’', "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let question = normalized.trim_end_matches(['?', '.', '!']).trim_end();
    let question = strip_any_prefix(question, &["please, ", "please "]).unwrap_or(question);
    let question = strip_any_suffix(question, &[" right now", " currently"]).unwrap_or(question);
    let query = strip_article(
        strip_any_prefix(
            question,
            &["what is ", "what's ", "show me ", "tell me ", "show "],
        )
        .unwrap_or(question),
    );

    if let Some(metric) = metric(question, query) {
        return Question::Metric(metric);
    }
    if let Some(check) = readiness(question, query) {
        return Question::Readiness(check);
    }
    if let Some(check) = question
        .strip_prefix("why ")
        .and_then(|question| readiness(question, question))
    {
        return Question::ReadinessExplanation(check);
    }
    let context = match query {
        "status of this job" | "state of this job" | "current job status" => {
            Some(ChatSourceKind::Job)
        }
        "status of this cycle" | "state of this cycle" | "current cycle status" => {
            Some(ChatSourceKind::Cycle)
        }
        _ => None,
    };
    if let Some(context) = context {
        return Question::ContextStatus(context);
    }
    if summary(question, query) {
        return Question::Summary;
    }
    Question::Unsupported
}

fn metric(question: &str, query: &str) -> Option<Metric> {
    let query = strip_any_prefix(query, &["current ", "live "]).unwrap_or(query);
    let host_query = strip_any_prefix(
        query,
        &[
            "api host's ",
            "api host ",
            "backend host's ",
            "backend host ",
        ],
    )
    .unwrap_or(query);
    let host_query = strip_any_suffix(
        host_query,
        &[
            " on the api host",
            " of the api host",
            " on the backend host",
        ],
    )
    .unwrap_or(host_query);
    match (host_query, question) {
        ("cpu usage" | "cpu utilization", _) => return Some(Metric::Cpu),
        ("memory usage" | "memory utilization" | "ram usage" | "ram utilization", _)
        | (_, "how much memory is in use" | "how much memory is used") => {
            return Some(Metric::Memory);
        }
        ("disk usage" | "disk utilization" | "disk space usage", _)
        | (_, "how much disk space is in use" | "how much disk space is used") => {
            return Some(Metric::Disk);
        }
        _ => {}
    }

    let jobs_query = strip_any_suffix(query, &[" on this workstation", " for this workstation"])
        .unwrap_or(query);
    let count_subject = strip_any_prefix(jobs_query, &["how many ", "number of ", "count of "]);
    let jobs_query = count_subject.unwrap_or(jobs_query);
    let jobs_query = jobs_query.strip_suffix(" are there").unwrap_or(jobs_query);
    let jobs_query = jobs_query
        .strip_prefix("workstation ")
        .unwrap_or(jobs_query);
    match (jobs_query, count_subject.is_some()) {
        ("queued job count" | "job queue depth", _)
        | (
            "queued jobs"
            | "jobs are queued"
            | "jobs are currently queued"
            | "jobs are in the queue",
            true,
        ) => Some(Metric::QueuedJobs),
        ("running job count", _)
        | ("running jobs" | "jobs are running" | "jobs are currently running", true) => {
            Some(Metric::RunningJobs)
        }
        ("retrying job count", _)
        | (
            "retrying jobs"
            | "jobs are retrying"
            | "jobs are currently retrying"
            | "jobs scheduled to retry"
            | "jobs are scheduled to retry",
            true,
        ) => Some(Metric::RetryingJobs),
        _ => None,
    }
}

fn readiness(question: &str, query: &str) -> Option<ReadinessCheckId> {
    match question {
        "have actors been seeded"
        | "have the actors been seeded"
        | "does the actor catalog have data"
        | "does the catalog need seeding"
        | "is the actor catalog empty"
        | "is the actor catalog unseeded"
        | "is the catalog empty" => return Some(ReadinessCheckId::ActorCatalog),
        "is the classifier trained"
        | "has the classifier been trained"
        | "is the legacy natural classifier trained"
        | "has the legacy natural classifier been trained" => {
            return Some(ReadinessCheckId::ClassifierTraining);
        }
        _ => {}
    }

    let query = query.strip_prefix("current ").unwrap_or(query);
    let subject = strip_any_prefix(
        query,
        &["status of ", "state of ", "readiness of ", "health of "],
    )
    .or_else(|| strip_any_suffix(query, &[" status", " readiness", " health"]))
    .or_else(|| strip_any_prefix(question, &["how is ", "how's "]))
    .or_else(|| {
        let subject = strip_any_prefix(question, &["is ", "isn't ", "has "])?;
        strip_any_suffix(
            subject,
            &[
                " not ready",
                " not applicable",
                " ready",
                " healthy",
                " reachable",
                " unavailable",
                " available",
                " unknown",
                " degraded",
                " online",
                " down",
                " up",
                " running",
                " been seeded",
                " seeded",
            ],
        )
    })?;
    match strip_article(subject) {
        "gateway" | "api gateway" | "backend gateway" => Some(ReadinessCheckId::Gateway),
        "database" | "datastore" | "data store" => Some(ReadinessCheckId::Datastore),
        "jobs" | "jobs service" => Some(ReadinessCheckId::Jobs),
        "brain" | "brain service" => Some(ReadinessCheckId::Brain),
        "control" | "control service" => Some(ReadinessCheckId::Control),
        "nlp" | "nlp service" | "natural language processing" => Some(ReadinessCheckId::Nlp),
        "classifier" | "classifier service" => Some(ReadinessCheckId::Classifier),
        "actor catalog" | "catalog" => Some(ReadinessCheckId::ActorCatalog),
        "actor search" | "actor index" | "actor search index" => {
            Some(ReadinessCheckId::ActorSearch)
        }
        "classifier inference"
        | "atlas inference"
        | "atlas rust bert inference"
        | "rust bert inference" => Some(ReadinessCheckId::ClassifierInference),
        "classifier training"
        | "natural training"
        | "legacy natural training"
        | "natural classifier training" => Some(ReadinessCheckId::ClassifierTraining),
        _ => None,
    }
}

fn summary(question: &str, query: &str) -> bool {
    let query = strip_any_prefix(question, &["summarize ", "summarise "]).unwrap_or(query);
    let query = strip_article(query);
    let query = strip_any_prefix(query, &["overall ", "current "]).unwrap_or(query);
    matches!(
        query,
        "backend health"
            | "backend readiness"
            | "backend status"
            | "workstation health"
            | "workstation readiness"
            | "workstation status"
            | "status of backend"
            | "state of the backend"
            | "status of the backend"
            | "health of the backend"
    ) || matches!(
        question,
        "how is the backend doing" | "how is the backend" | "is the backend healthy"
    )
}

fn strip_article(value: &str) -> &str {
    value.strip_prefix("the ").unwrap_or(value)
}

fn strip_any_prefix<'a>(value: &'a str, patterns: &[&str]) -> Option<&'a str> {
    patterns
        .iter()
        .find_map(|pattern| value.strip_prefix(pattern))
}

fn strip_any_suffix<'a>(value: &'a str, patterns: &[&str]) -> Option<&'a str> {
    patterns
        .iter()
        .find_map(|pattern| value.strip_suffix(pattern))
}

#[cfg(test)]
mod tests {
    use super::classify;
    use super::Metric;
    use super::Question;
    use fleetworks_contracts::readiness::ReadinessCheckId;

    #[test]
    fn binds_known_readiness_questions_including_negative_forms_to_one_check() {
        for (question, check) in [
            (
                "What is the state of the gateway?",
                ReadinessCheckId::Gateway,
            ),
            (
                "Please, is the backend gateway ready?",
                ReadinessCheckId::Gateway,
            ),
            (
                "Isn’t the actor catalog ready?",
                ReadinessCheckId::ActorCatalog,
            ),
            (
                "Is the actor catalog not ready?",
                ReadinessCheckId::ActorCatalog,
            ),
            (
                "Has the actor catalog been seeded?",
                ReadinessCheckId::ActorCatalog,
            ),
            ("Is the catalog empty?", ReadinessCheckId::ActorCatalog),
            ("ACTOR_SEARCH health", ReadinessCheckId::ActorSearch),
            (
                "What is the current status of the Jobs service?",
                ReadinessCheckId::Jobs,
            ),
            ("How is the database?", ReadinessCheckId::Datastore),
            (
                "Is classifier inference available?",
                ReadinessCheckId::ClassifierInference,
            ),
        ] {
            assert_eq!(classify(question), Question::Readiness(check), "{question}");
        }
    }

    #[test]
    fn binds_supported_numeric_questions_to_the_requested_measurement() {
        for (question, metric) in [
            ("What's the API host CPU usage?", Metric::Cpu),
            ("What is the current memory usage?", Metric::Memory),
            ("What is the disk usage on the backend host?", Metric::Disk),
            (
                "How many jobs are queued on this workstation?",
                Metric::QueuedJobs,
            ),
            ("How many jobs are running?", Metric::RunningJobs),
            (
                "Number of workstation jobs scheduled to retry?",
                Metric::RetryingJobs,
            ),
            (
                "How many jobs are retrying right now?",
                Metric::RetryingJobs,
            ),
        ] {
            assert_eq!(classify(question), Question::Metric(metric), "{question}");
        }
    }

    #[test]
    fn keeps_classifier_checks_and_explanations_distinct() {
        for (question, intent) in [
            (
                "Is the classifier service ready?",
                Question::Readiness(ReadinessCheckId::Classifier),
            ),
            (
                "Is Atlas Rust-BERT inference ready?",
                Question::Readiness(ReadinessCheckId::ClassifierInference),
            ),
            (
                "What is the state of Legacy Natural training?",
                Question::Readiness(ReadinessCheckId::ClassifierTraining),
            ),
            (
                "Why is the actor catalog not ready?",
                Question::ReadinessExplanation(ReadinessCheckId::ActorCatalog),
            ),
            (
                "Why is the datastore unavailable?",
                Question::ReadinessExplanation(ReadinessCheckId::Datastore),
            ),
            (
                "Why is Atlas Rust-BERT inference unknown?",
                Question::ReadinessExplanation(ReadinessCheckId::ClassifierInference),
            ),
        ] {
            assert_eq!(classify(question), intent, "{question}");
        }
    }

    #[test]
    fn admits_explicit_general_summaries_to_model_inference() {
        for question in [
            "What is the current workstation readiness?",
            "What is the overall backend status?",
            "Summarize backend health.",
        ] {
            assert_eq!(classify(question), Question::Summary, "{question}");
        }
    }

    #[test]
    fn leaves_unsupported_or_mixed_requests_without_a_grounding_source() {
        for question in [
            "How many jobs failed?",
            "How many jobs are running across all workstations?",
            "Are jobs running?",
            "Show me running jobs.",
            "What is the CPU temperature?",
            "What is the one-minute CPU load?",
            "How much memory is free?",
            "What is yesterday's memory usage?",
            "What is the current memory usage on my laptop?",
            "What is the actor catalog?",
            "Is the workstation registered?",
            "Is the gateway or datastore ready?",
            "Summarize backend health and CPU usage.",
            "What is the memory usage? Include gateway status.",
        ] {
            assert_eq!(classify(question), Question::Unsupported, "{question}");
        }
    }
}
