use fleetworks_contracts::chat::ChatSourceKind;
use fleetworks_contracts::readiness::ReadinessCheckId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Question {
    Greeting,
    Readiness(ReadinessCheckId),
    ReadinessExplanation(ReadinessCheckId),
    Metric(Metric),
    HostResources,
    JobsSummary,
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
    if matches!(
        question,
        "hi" | "hello"
            | "hey"
            | "hi there"
            | "hello there"
            | "hey there"
            | "good morning"
            | "good afternoon"
            | "good evening"
    ) {
        return Question::Greeting;
    }
    let question = strip_any_prefix(
        question,
        &["hi, ", "hi! ", "hello, ", "hello! ", "hey, ", "hey! "],
    )
    .unwrap_or(question);
    let question = strip_any_prefix(question, &["please, ", "please "]).unwrap_or(question);
    let question = strip_any_suffix(question, &[" right now", " currently", " at the moment"])
        .unwrap_or(question);
    let query = strip_article(
        strip_any_prefix(
            question,
            &[
                "what is ",
                "what's ",
                "what are ",
                "can you tell me ",
                "can you show me ",
                "can you check ",
                "give me a ",
                "show me ",
                "tell me ",
                "show ",
                "check ",
            ],
        )
        .unwrap_or(question),
    );

    let current_query = strip_any_prefix(query, &["current ", "live "]).unwrap_or(query);
    if host_resources(current_query) {
        return Question::HostResources;
    }
    if jobs_summary(current_query) {
        return Question::JobsSummary;
    }
    if let Some(metric) = metric(current_query) {
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

fn metric(query: &str) -> Option<Metric> {
    if let Some(metric) = host_usage(query) {
        return Some(metric);
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
            | "jobs are in the queue"
            | "jobs in the queue"
            | "jobs are waiting in the queue",
            true,
        ) => Some(Metric::QueuedJobs),
        ("running job count" | "are jobs running" | "are any jobs running", _)
        | (
            "running jobs"
            | "jobs are running"
            | "jobs are currently running"
            | "jobs are in progress",
            true,
        ) => Some(Metric::RunningJobs),
        ("retrying job count", _)
        | (
            "retrying jobs"
            | "jobs are retrying"
            | "jobs are currently retrying"
            | "jobs scheduled to retry"
            | "jobs are scheduled to retry"
            | "jobs are waiting to retry",
            true,
        ) => Some(Metric::RetryingJobs),
        _ => None,
    }
}

fn host_resource_query(query: &str) -> &str {
    let host_query = strip_any_prefix(
        query,
        &[
            "api host's ",
            "api host ",
            "backend host's ",
            "backend host ",
            "our ",
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
    strip_any_prefix(host_query, &["current ", "live "]).unwrap_or(host_query)
}

fn host_resources(query: &str) -> bool {
    if matches!(
        query,
        "how are our resources looking" | "how are we doing on resources"
    ) {
        return true;
    }
    let query =
        strip_any_prefix(query, &["how is ", "how's ", "how are we doing on "]).unwrap_or(query);
    let query = host_resource_query(strip_article(query));
    let usage = strip_any_suffix(query, &[" usage", " utilization", " utilisation"]);
    match usage.unwrap_or(query) {
        "resources"
        | "host resources"
        | "system resources"
        | "cpu, memory, and disk"
        | "cpu, memory and disk"
        | "cpu memory and disk"
        | "cpu, ram, and disk"
        | "cpu, ram and disk" => true,
        "resource"
        | "host resource"
        | "system resource"
        | "cpu usage, memory usage, and disk"
        | "cpu usage, memory usage and disk" => usage.is_some(),
        _ => false,
    }
}

fn host_usage(query: &str) -> Option<Metric> {
    let host_query = host_resource_query(query);
    if let Some(metric) = strip_any_suffix(
        host_query,
        &[
            " usage",
            " use",
            " utilization",
            " utilisation",
            " consumption",
        ],
    )
    .and_then(host_resource)
    {
        return Some(metric);
    }

    let usage = strip_any_prefix(
        host_query,
        &["how much ", "what percent of ", "what percentage of "],
    )?;
    if let Some(usage) = strip_any_suffix(
        usage,
        &[
            " is in use",
            " is being used",
            " is used",
            " are we using",
            " we are using",
            " we're using",
        ],
    ) {
        return host_resource(strip_article(usage));
    }
    let usage = strip_any_suffix(usage, &[" are we at", " we are at", " we're at"])?;
    let resource = strip_any_suffix(
        usage,
        &[" usage", " consumption", " utilization", " utilisation"],
    )
    .unwrap_or(usage);
    host_resource(resource)
}

fn host_resource(resource: &str) -> Option<Metric> {
    match resource {
        "cpu" | "processor" => Some(Metric::Cpu),
        "memory" | "ram" => Some(Metric::Memory),
        "disk" | "disk space" => Some(Metric::Disk),
        _ => None,
    }
}

fn jobs_summary(query: &str) -> bool {
    let query = strip_any_suffix(query, &[" on this workstation", " for this workstation"])
        .unwrap_or(query);
    matches!(
        query,
        "job counts"
            | "workstation job counts"
            | "job activity"
            | "workstation job activity"
            | "job summary"
            | "jobs summary"
            | "summary of jobs"
            | "overview of jobs"
            | "summarize jobs"
            | "summarise jobs"
            | "how are the jobs doing"
            | "happening with jobs"
            | "queued, running, and retrying job counts"
            | "queued, running and retrying job counts"
            | "how many jobs are queued, running, and retrying"
            | "how many jobs are queued, running and retrying"
    )
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
        "can actor search receive queries"
        | "can the actor search index receive queries"
        | "can't actor search receive queries"
        | "can't the actor search index receive queries" => {
            return Some(ReadinessCheckId::ActorSearch)
        }
        _ => {}
    }

    let query = query.strip_prefix("current ").unwrap_or(query);
    let subject = health_subject(query)
        .or_else(|| how_is_subject(question))
        .or_else(|| {
            let subject = strip_any_prefix(question, &["is ", "isn't ", "has "])?;
            strip_any_suffix(
                subject,
                &[
                    " not ready",
                    " not applicable",
                    " ready",
                    " healthy",
                    " working",
                    " operational",
                    " responding",
                    " reachable",
                    " unavailable",
                    " available",
                    " unknown",
                    " degraded",
                    " online",
                    " down",
                    " up",
                    " up and running",
                    " running",
                    " been seeded",
                    " seeded",
                ],
            )
        })?;
    match strip_article(subject) {
        "gateway" | "api gateway" | "backend gateway" => Some(ReadinessCheckId::Gateway),
        "database" | "datastore" | "data store" | "database service" => {
            Some(ReadinessCheckId::Datastore)
        }
        "jobs" | "jobs service" | "job service" => Some(ReadinessCheckId::Jobs),
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
    health_subject(query)
        .or_else(|| how_is_subject(question))
        .or_else(|| {
            strip_any_prefix(question, &["is ", "isn't "]).and_then(|subject| {
                strip_any_suffix(subject, &[" healthy", " ready", " operational"])
            })
        })
        .is_some_and(|subject| {
            let subject =
                strip_any_prefix(subject, &["the ", "this ", "my ", "our "]).unwrap_or(subject);
            matches!(subject, "backend" | "workstation")
        })
}

fn health_subject(query: &str) -> Option<&str> {
    strip_any_prefix(
        query,
        &["status of ", "state of ", "readiness of ", "health of "],
    )
    .or_else(|| {
        strip_any_suffix(
            query,
            &[" status update", " status", " readiness", " health"],
        )
    })
}

fn how_is_subject(question: &str) -> Option<&str> {
    let subject = strip_any_prefix(question, &["how is ", "how's "])?;
    Some(strip_any_suffix(subject, &[" doing", " looking", " holding up"]).unwrap_or(subject))
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
    fn recognizes_standalone_greetings() {
        for question in ["hi", "  Hello there! ", "Good morning."] {
            assert_eq!(classify(question), Question::Greeting, "{question}");
        }
    }

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
            ("Is the Jobs service ready?", ReadinessCheckId::Jobs),
            ("How's the Jobs service doing?", ReadinessCheckId::Jobs),
            ("How is the database?", ReadinessCheckId::Datastore),
            ("Is the database responding?", ReadinessCheckId::Datastore),
            (
                "Can actor search receive queries?",
                ReadinessCheckId::ActorSearch,
            ),
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
            ("How much CPU are we using?", Metric::Cpu),
            ("What is the current memory usage?", Metric::Memory),
            ("Hi! What is the memory usage?", Metric::Memory),
            ("how much memory consumption we are at", Metric::Memory),
            ("What's our RAM consumption right now?", Metric::Memory),
            ("How much memory is in use?", Metric::Memory),
            ("What percentage of memory is in use?", Metric::Memory),
            ("What is the disk usage on the backend host?", Metric::Disk),
            (
                "How much disk space is being used at the moment?",
                Metric::Disk,
            ),
            (
                "How many jobs are queued on this workstation?",
                Metric::QueuedJobs,
            ),
            ("How many jobs are running?", Metric::RunningJobs),
            ("Are jobs running?", Metric::RunningJobs),
            ("How many jobs are in the queue?", Metric::QueuedJobs),
            ("How many jobs are in progress?", Metric::RunningJobs),
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
    fn recognizes_grouped_resource_and_job_observations() {
        for (question, intent) in [
            ("What is our resource usage?", Question::HostResources),
            (
                "What are the current host resources?",
                Question::HostResources,
            ),
            (
                "What is the CPU, memory, and disk usage?",
                Question::HostResources,
            ),
            (
                "Show CPU usage, memory usage and disk usage on the backend host",
                Question::HostResources,
            ),
            ("How are the jobs doing?", Question::JobsSummary),
            ("What is the current job activity?", Question::JobsSummary),
            (
                "How many jobs are queued, running, and retrying on this workstation?",
                Question::JobsSummary,
            ),
        ] {
            assert_eq!(classify(question), intent, "{question}");
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
            "How's my workstation doing?",
            "What is the health of this workstation?",
            "Give me a workstation status update.",
            "Is the workstation ready?",
        ] {
            assert_eq!(classify(question), Question::Summary, "{question}");
        }
    }

    #[test]
    fn leaves_unsupported_or_mixed_requests_without_a_grounding_source() {
        for question in [
            "How many jobs failed?",
            "How many jobs are running across all workstations?",
            "Job counts across all workstations.",
            "How are my jobs doing?",
            "What jobs are running?",
            "Show me running jobs.",
            "What is the CPU temperature?",
            "What is the one-minute CPU load?",
            "How much memory is free?",
            "What is yesterday's memory usage?",
            "What is the current memory usage on my laptop?",
            "How much memory consumption we are at on my laptop?",
            "What is the actor catalog?",
            "Is the workstation registered?",
            "Is the gateway or datastore ready?",
            "Summarize backend health and CPU usage.",
            "What is CPU, memory, and disk usage on my laptop?",
            "What is CPU, memory, and disk usage by user?",
            "What is CPU, memory, disk, and GPU usage?",
            "What is the memory usage? Include gateway status.",
            "Hi! What is the weather?",
            "Good morning, how is the weather?",
        ] {
            assert_eq!(classify(question), Question::Unsupported, "{question}");
        }
    }
}
