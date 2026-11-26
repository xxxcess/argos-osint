use fleetworks_contracts::chat::ChatSourceKind;
use fleetworks_contracts::readiness::ReadinessCheckId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Question {
    Greeting,
    Acknowledgement,
    Help,
    FollowUp,
    ChooseSubject,
    Recall,
    ExplainSelected,
    Readiness(ReadinessCheckId),
    ReadinessExplanation(ReadinessCheckId),
    StatusChecks(StatusChecks),
    StatusChecksFollowUp(StatusChecks, Option<usize>),
    Metric(Metric),
    HostResources,
    JobsSummary,
    RunningJobList,
    ContextStatus(ChatSourceKind),
    ContextLogs(ChatSourceKind),
    Configuration,
    Summary,
    Unsupported,
}

impl Question {
    /// Keep unambiguous single-fact requests on the direct path.
    fn is_exact_fact(self, question: &str) -> bool {
        if let Self::Metric(metric) = self {
            let current = strip_any_suffix(question, CURRENT_TIME).unwrap_or(question);
            if short_job_count(workstation_jobs_query(current)) == Some(metric) {
                return true;
            }
        }
        let canonical = match self {
            Self::Metric(Metric::Cpu) => "what is the current cpu utilization",
            Self::Metric(Metric::Memory) => "what is the current memory usage",
            Self::Metric(Metric::Disk) => "what is the current disk usage",
            Self::Metric(Metric::QueuedJobs) => "how many jobs are queued",
            Self::Metric(Metric::RunningJobs) => "how many jobs are running",
            Self::Metric(Metric::RetryingJobs) => "how many jobs are retrying",
            Self::Metric(Metric::FailedJobs) => "how many jobs failed in the past 24 hours",
            Self::Readiness(ReadinessCheckId::ActorCatalog) => "does the catalog need seeding",
            _ => return false,
        };
        question == canonical
    }

    pub(super) fn wants_measurement(self, metric: Metric) -> bool {
        match (self, metric) {
            (Self::Metric(requested), metric) => requested == metric,
            (Self::HostResources, Metric::Cpu | Metric::Memory | Metric::Disk)
            | (
                Self::JobsSummary,
                Metric::QueuedJobs | Metric::RunningJobs | Metric::RetryingJobs,
            ) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StatusChecks {
    All,
    Passed,
    Unresolved,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Metric {
    Cpu,
    Memory,
    Disk,
    QueuedJobs,
    RunningJobs,
    RetryingJobs,
    FailedJobs,
}

const GREETINGS: &[&str] = &[
    "hi there",
    "hello there",
    "hey there",
    "good morning",
    "good afternoon",
    "good evening",
    "hi",
    "hello",
    "hey",
];
const CURRENT_TIME: &[&str] = &[" right now", " currently", " at the moment", " now"];

#[derive(Clone, Copy)]
pub(super) struct Classified {
    pub(super) intent: Question,
    pub(super) exact_fact: bool,
}

/// Match the whole question so additional subjects or qualifiers cannot silently change its scope.
pub(super) fn classify(question: &str) -> Classified {
    let normalized = question
        .to_ascii_lowercase()
        .replace(['_', '-'], " ")
        .replace('’', "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let normalized = trim_sentence_end(&normalized);
    let mut intent = classify_simple(normalized);
    if intent == Question::Unsupported {
        if let Some((first, second)) = normalized.split_once(" and ") {
            let first = classify_simple(trim_sentence_end(first.trim_end_matches([',', ' '])));
            let second = classify_simple(second);
            if first != second && is_operational(first) && is_operational(second) {
                intent = Question::ChooseSubject;
            }
        }
    }
    Classified {
        intent,
        exact_fact: intent.is_exact_fact(normalized),
    }
}

fn is_operational(intent: Question) -> bool {
    matches!(
        intent,
        Question::ExplainSelected
            | Question::Readiness(_)
            | Question::ReadinessExplanation(_)
            | Question::StatusChecks(_)
            | Question::Metric(_)
            | Question::HostResources
            | Question::JobsSummary
            | Question::RunningJobList
            | Question::ContextStatus(_)
            | Question::ContextLogs(_)
            | Question::Configuration
            | Question::Summary
    )
}

fn classify_simple(question: &str) -> Question {
    if GREETINGS.contains(&question) {
        return Question::Greeting;
    }
    let question = GREETINGS
        .iter()
        .find_map(|salutation| {
            let rest = question.strip_prefix(*salutation)?;
            strip_any_prefix(rest, &[", ", "! ", ". ", " "])
        })
        .or_else(|| {
            strip_any_prefix(
                question,
                &["thanks, ", "thanks! ", "thank you, ", "thank you! "],
            )
        })
        .unwrap_or(question);
    let question = strip_any_prefix(question, &["please, ", "please "]).unwrap_or(question);
    if let Some(intent) = conversational_intent(question) {
        return intent;
    }
    let question = strip_any_suffix(
        question,
        &[
            ", please",
            " please",
            ", thanks",
            " thanks",
            ", thank you",
            " thank you",
        ],
    )
    .map_or(question, trim_sentence_end);
    if let Some(intent) = conversational_intent(question) {
        return intent;
    }
    let question = strip_any_suffix(question, CURRENT_TIME).unwrap_or(question);
    let request =
        strip_any_prefix(question, &["could you please ", "can you please "]).unwrap_or(question);
    let query = strip_article(
        strip_any_prefix(
            request,
            &[
                "what is ",
                "what's ",
                "what are ",
                "can you tell me ",
                "can you show me ",
                "can you check ",
                "could you tell me ",
                "could you show me ",
                "could you check ",
                "give me a ",
                "show me ",
                "tell me ",
                "show ",
                "check ",
            ],
        )
        .unwrap_or(request),
    );

    let current_query = strip_any_prefix(query, &["current ", "live "]).unwrap_or(query);
    if host_resources(current_query) {
        return Question::HostResources;
    }
    if jobs_summary(current_query) {
        return Question::JobsSummary;
    }
    if matches!(
        workstation_jobs_query(current_query),
        "what jobs are running"
            | "which jobs are running"
            | "what jobs are currently running"
            | "which jobs are currently running"
            | "running jobs"
            | "list running jobs"
            | "list current running jobs"
            | "list currently running jobs"
            | "jobs that are running"
    ) {
        return Question::RunningJobList;
    }
    if let Some(metric) = metric(current_query) {
        return Question::Metric(metric);
    }
    if let Some(checks) = status_checks(current_query) {
        return Question::StatusChecks(checks);
    }
    if let Some(intent) = status_checks_follow_up(current_query) {
        return intent;
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
    if matches!(
        query,
        "what happened with this job"
            | "what happened with this indexing job"
            | "what happened to this job"
            | "why did this job fail"
            | "why did this indexing job fail"
            | "explain this job"
    ) {
        return Question::ExplainSelected;
    }
    let context = match query {
        "status of this job"
        | "state of this job"
        | "current job status"
        | "how's this job doing"
        | "is this job still running" => Some(ChatSourceKind::Job),
        "status of this cycle"
        | "state of this cycle"
        | "current cycle status"
        | "how's this cycle doing"
        | "has this cycle finished" => Some(ChatSourceKind::Cycle),
        _ => None,
    };
    if let Some(context) = context {
        return Question::ContextStatus(context);
    }
    let log_source = match current_query {
        "logs for this job"
        | "job log summary"
        | "summarize the logs for this job"
        | "any errors or warnings for this job" => Some(ChatSourceKind::Job),
        "logs for this cycle"
        | "cycle log summary"
        | "summarize the logs for this cycle"
        | "any errors or warnings for this cycle" => Some(ChatSourceKind::Cycle),
        _ => None,
    };
    if let Some(source) = log_source {
        return Question::ContextLogs(source);
    }
    if matches!(
        current_query,
        "workstation configuration"
            | "this workstation's configuration"
            | "backend mode"
            | "assistant configuration"
            | "which backend is this workstation using"
            | "which backend am i connected to"
            | "active assistant model"
            | "which assistant model is active"
            | "which model is the assistant using"
            | "is backend inference enabled"
            | "is backend inference on"
    ) {
        return Question::Configuration;
    }
    if summary(question, query) {
        return Question::Summary;
    }
    Question::Unsupported
}

fn conversational_intent(question: &str) -> Option<Question> {
    let intent = match question {
        "how are you" | "how are you doing" | "how's it going" => Question::Greeting,
        "thanks" | "thank you" | "thanks a lot" | "thank you so much" | "many thanks" | "ok"
        | "okay" | "got it" | "understood" | "makes sense" | "great" => Question::Acknowledgement,
        "help"
        | "help me"
        | "can you help"
        | "could you help"
        | "i need help"
        | "help with setup"
        | "workstation help"
        | "how do i get started"
        | "how do i use this"
        | "how do i set up this workstation"
        | "how do i set up my workstation"
        | "what can you do"
        | "what can i ask"
        | "what can i ask you"
        | "what can i ask about"
        | "what should i ask"
        | "what can you help me with"
        | "what can you help with" => Question::Help,
        "and now"
        | "what about now"
        | "how about now"
        | "has that changed"
        | "has it changed"
        | "any change"
        | "any changes"
        | "is that still true"
        | "is it still the same"
        | "check again"
        | "can you check again"
        | "could you check again" => Question::FollowUp,
        "what did i just ask"
        | "what did i ask you"
        | "what did i just say"
        | "what was my last question"
        | "what did you just say"
        | "what was your last answer"
        | "recap the last exchange"
        | "what were we talking about"
        | "what were we just talking about"
        | "recap our conversation" => Question::Recall,
        "what happened"
        | "what does this mean"
        | "what does this status mean"
        | "what is this"
        | "summarize this"
        | "can you summarize this"
        | "explain this"
        | "can you explain this"
        | "what is happening here"
        | "what's happening here"
        | "what is going on here"
        | "what's going on here"
        | "tell me about this" => Question::ExplainSelected,
        _ => return None,
    };
    Some(intent)
}

fn trim_sentence_end(value: &str) -> &str {
    value.trim_end_matches(['?', '.', '!']).trim_end()
}

fn status_checks(query: &str) -> Option<StatusChecks> {
    let query = strip_any_prefix(
        query,
        &[
            "list ",
            "summarize ",
            "summarise ",
            "explain ",
            "which ",
            "why are ",
            "how many ",
            "are there any ",
        ],
    )
    .unwrap_or(query);
    let query = strip_any_prefix(query, &["the ", "tthe "]).unwrap_or(query);
    let query = strip_any_prefix(query, &["current ", "live "]).unwrap_or(query);
    let query = strip_any_suffix(query, &[" on this workstation", " for this workstation"])
        .unwrap_or(query);
    let query = strip_any_suffix(query, &[" about", " are there"]).unwrap_or(query);
    for subject in [
        "workstation status checks",
        "workstation status check",
        "backend status checks",
        "backend status check",
        "status checks",
        "status check",
        "checks",
        "check",
    ] {
        if query == subject && subject.contains("status") {
            return Some(StatusChecks::All);
        }
        let group = query
            .strip_suffix(subject)
            .and_then(|group| group.strip_suffix(' '))
            .or_else(|| query.strip_prefix(subject)?.strip_prefix(' '));
        if let Some(group) = group.and_then(status_check_group) {
            return Some(group);
        }
    }
    None
}

fn status_check_group(group: &str) -> Option<StatusChecks> {
    let group = strip_any_prefix(group, &["are ", "is ", "have ", "has "]).unwrap_or(group);
    let group = group.strip_prefix("still ").unwrap_or(group);
    Some(match group {
        "green" | "passed" | "passing" | "ready" => StatusChecks::Passed,
        "red" | "failed" | "failing" => StatusChecks::Failed,
        "unresolved" | "need attention" | "needs attention" => StatusChecks::Unresolved,
        _ => return None,
    })
}

fn status_checks_follow_up(query: &str) -> Option<Question> {
    let query = strip_any_prefix(
        query,
        &[
            "can you ",
            "could you ",
            "would you ",
            "what about ",
            "list ",
            "name ",
            "which ",
        ],
    )
    .unwrap_or(query);
    let query = strip_any_prefix(query, &["list ", "name ", "the "]).unwrap_or(query);
    if matches!(query, "ones" | "those" | "them" | "of those") {
        return Some(Question::StatusChecksFollowUp(StatusChecks::All, None));
    }
    let (count, group) = query
        .split_once(' ')
        .and_then(|(first, rest)| Some((first.parse::<usize>().ok()?, rest)))
        .map_or((None, query), |(count, rest)| (Some(count), rest));
    let group = strip_any_suffix(group, &[" one", " ones"])
        .or_else(|| strip_any_prefix(group, &["ones ", "of those ", "those ", "that "]))?;
    Some(Question::StatusChecksFollowUp(
        status_check_group(group)?,
        count,
    ))
}

fn metric(query: &str) -> Option<Metric> {
    if let Some(metric) = host_usage(query) {
        return Some(metric);
    }

    let jobs_query = workstation_jobs_query(query);
    let presence_query =
        strip_any_prefix(jobs_query, &["are there ", "are ", "do we have "]).unwrap_or(jobs_query);
    if let Some(metric) = short_job_count(presence_query) {
        return Some(metric);
    }
    let failed_query = strip_any_suffix(
        jobs_query,
        &[" in the last 24 hours", " in the past 24 hours"],
    )
    .unwrap_or(jobs_query);
    if matches!(
        failed_query,
        "how many jobs failed"
            | "how many failed jobs"
            | "failed job count"
            | "number of failed jobs"
    ) {
        return Some(Metric::FailedJobs);
    }
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
        ("running job count" | "are jobs running", _)
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

fn workstation_jobs_query(query: &str) -> &str {
    strip_any_suffix(query, &[" on this workstation", " for this workstation"]).unwrap_or(query)
}

fn short_job_count(query: &str) -> Option<Metric> {
    let query = strip_any_suffix(query, CURRENT_TIME).unwrap_or(query);
    Some(match query {
        "any queued jobs" | "any jobs queued" | "any jobs in the queue" => Metric::QueuedJobs,
        "any running jobs"
        | "any jobs running"
        | "any jobs currently running"
        | "any jobs still running"
        | "any jobs in progress" => Metric::RunningJobs,
        "any retrying jobs" | "any jobs retrying" | "any jobs waiting to retry" => {
            Metric::RetryingJobs
        }
        "any failed jobs in the last 24 hours" | "any failed jobs in the past 24 hours" => {
            Metric::FailedJobs
        }
        _ => return None,
    })
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
        | "resource snapshot"
        | "host resource snapshot"
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
            " usage percentage",
            " in use",
            " used",
        ],
    )
    .and_then(host_resource)
    {
        return Some(metric);
    }

    let usage = strip_any_prefix(
        host_query,
        &[
            "how much of ",
            "how much ",
            "what percent of ",
            "what percentage of ",
        ],
    )?;
    if let Some(usage) = strip_any_suffix(
        usage,
        &[
            " is in use",
            " is being used",
            " is currently in use",
            " is currently being used",
            " is used",
            " are we using",
            " we are using",
            " we're using",
            " is the api host using",
            " is the backend host using",
        ],
    ) {
        return host_resource(strip_any_prefix(usage, &["the ", "our "]).unwrap_or(usage));
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
        "cpu" | "processor" | "system cpu" => Some(Metric::Cpu),
        "memory" | "ram" | "system memory" => Some(Metric::Memory),
        "disk" | "disk space" => Some(Metric::Disk),
        _ => None,
    }
}

fn jobs_summary(query: &str) -> bool {
    matches!(
        workstation_jobs_query(query),
        "job counts"
            | "workstation job counts"
            | "job activity"
            | "workstation job activity"
            | "job summary"
            | "jobs summary"
            | "summary of jobs"
            | "overview of jobs"
            | "job queue overview"
            | "how busy is the job queue"
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
                    " ready for use",
                    " ready to use",
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
    if matches!(
        question,
        "how's everything"
            | "how's everything doing"
            | "how's everything looking"
            | "how is everything"
            | "how is everything doing"
            | "how is everything looking"
            | "is everything okay"
            | "is everything ok"
            | "what needs attention"
            | "does anything need attention"
            | "is anything wrong"
            | "any problems with this workstation"
            | "any problems with the workstation"
            | "any blockers on this workstation"
            | "are there any problems with this workstation"
            | "is anything wrong with this workstation"
    ) {
        return true;
    }
    let query = strip_any_prefix(question, &["summarize ", "summarise "]).unwrap_or(query);
    let query = strip_article(query);
    let query = strip_any_prefix(query, &["overall ", "current "]).unwrap_or(query);
    if let Some(subject) = strip_any_prefix(
        query,
        &[
            "can you give me the big picture on ",
            "could you give me the big picture on ",
            "give me the big picture on ",
            "big picture on ",
        ],
    ) {
        return matches!(
            subject,
            "this workstation" | "the workstation" | "the backend"
        );
    }
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
            &[
                " status update",
                " status",
                " readiness",
                " health",
                " health summary",
            ],
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
    use super::Metric;
    use super::Question;
    use super::StatusChecks;
    use fleetworks_contracts::chat::ChatSourceKind;
    use fleetworks_contracts::readiness::ReadinessCheckId;

    fn classify(prompt: &str) -> Question {
        super::classify(prompt).intent
    }

    #[test]
    fn recognizes_standalone_greetings() {
        for question in ["hi", "  Hello there! ", "Good morning.", "Hi, how are you?"] {
            assert_eq!(classify(question), Question::Greeting, "{question}");
        }
    }

    #[test]
    fn recognizes_conversational_intents_without_inferring_a_subject() {
        for (question, intent) in [
            ("Thanks a lot!", Question::Acknowledgement),
            ("Okay, thanks.", Question::Acknowledgement),
            ("Got it!", Question::Acknowledgement),
            ("Can you help?", Question::Help),
            ("What can I ask you?", Question::Help),
            ("How do I set up this workstation?", Question::Help),
            ("And now?", Question::FollowUp),
            ("Thanks, has that changed?", Question::FollowUp),
            ("Could you check again?", Question::FollowUp),
            ("What does this mean?", Question::ExplainSelected),
            ("Explain this, please.", Question::ExplainSelected),
            ("What is happening here?", Question::ExplainSelected),
            ("Tell me about this.", Question::ExplainSelected),
        ] {
            assert_eq!(classify(question), intent, "{question}");
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
            ("How much of the CPU is being used?", Metric::Cpu),
            ("What is the CPU usage percentage now?", Metric::Cpu),
            ("What is the current memory usage?", Metric::Memory),
            ("Hi! What is the memory usage?", Metric::Memory),
            (
                "Hi there, could you please check RAM usage?",
                Metric::Memory,
            ),
            ("Hello what's our memory usage, please?", Metric::Memory),
            ("What is the RAM usage? Thanks!", Metric::Memory),
            (
                "How much system memory is currently in use?",
                Metric::Memory,
            ),
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
            ("How many jobs failed?", Metric::FailedJobs),
            (
                "How many jobs failed in the past 24 hours?",
                Metric::FailedJobs,
            ),
            ("Are jobs running?", Metric::RunningJobs),
            ("Are there any jobs running?", Metric::RunningJobs),
            ("Are there any jobs in the queue?", Metric::QueuedJobs),
            ("Are any jobs waiting to retry?", Metric::RetryingJobs),
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
    fn answers_scoped_job_shorthand_without_model_inference() {
        for (question, metric) in [
            ("any jobs running?", Metric::RunningJobs),
            (
                "Any   running jobs on this workstation right now ?",
                Metric::RunningJobs,
            ),
            ("Any jobs currently running?", Metric::RunningJobs),
            ("Any jobs still running?", Metric::RunningJobs),
            ("Any jobs in the queue?", Metric::QueuedJobs),
            ("Any jobs waiting to retry?", Metric::RetryingJobs),
            ("Any failed jobs in the last 24 hours?", Metric::FailedJobs),
        ] {
            let route = super::classify(question);
            assert_eq!(route.intent, Question::Metric(metric), "{question}");
            assert!(route.exact_fact, "{question} is a scoped single fact");
        }

        for question in [
            "Are any jobs running?",
            "Do we have any jobs in progress right now?",
        ] {
            let route = super::classify(question);
            assert_eq!(
                route.intent,
                Question::Metric(Metric::RunningJobs),
                "{question}"
            );
            assert!(!route.exact_fact, "{question} can use model-guided wording");
        }
    }

    #[test]
    fn binds_natural_workstation_queries_to_known_evidence() {
        for (question, expected) in [
            (
                "How much RAM is the API host using?",
                Question::Metric(Metric::Memory),
            ),
            (
                "Could you check the host resource snapshot?",
                Question::HostResources,
            ),
            (
                "How busy is the job queue on this workstation?",
                Question::JobsSummary,
            ),
            (
                "Is the actor index ready for use?",
                Question::Readiness(ReadinessCheckId::ActorSearch),
            ),
            (
                "Please show the workstation health summary.",
                Question::Summary,
            ),
            ("Any blockers on this workstation?", Question::Summary),
            (
                "How's this job doing?",
                Question::ContextStatus(ChatSourceKind::Job),
            ),
            (
                "Has this cycle finished?",
                Question::ContextStatus(ChatSourceKind::Cycle),
            ),
            (
                "Any errors or warnings for this job?",
                Question::ContextLogs(ChatSourceKind::Job),
            ),
            (
                "Any errors or warnings for this cycle?",
                Question::ContextLogs(ChatSourceKind::Cycle),
            ),
            ("Which backend am I connected to?", Question::Configuration),
        ] {
            assert_eq!(classify(question), expected, "{question}");
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
    fn distinguishes_running_job_names_from_running_job_counts() {
        for prompt in [
            "What jobs are running?",
            "Which jobs are currently running on this workstation?",
            "Show me running jobs.",
            "List current running jobs.",
        ] {
            assert_eq!(classify(prompt), Question::RunningJobList, "{prompt}");
        }
        for prompt in ["How many jobs are running?", "Any jobs running?"] {
            assert_eq!(
                classify(prompt),
                Question::Metric(Metric::RunningJobs),
                "{prompt}"
            );
        }
    }

    #[test]
    fn routes_only_scoped_log_and_configuration_questions() {
        for (prompt, expected) in [
            (
                "What is the workstation configuration?",
                Question::Configuration,
            ),
            ("Is backend inference enabled?", Question::Configuration),
            ("Which Assistant model is active?", Question::Configuration),
            (
                "Show me logs for this job.",
                Question::ContextLogs(ChatSourceKind::Job),
            ),
            (
                "Summarize the logs for this cycle.",
                Question::ContextLogs(ChatSourceKind::Cycle),
            ),
            ("Show me logs for all jobs.", Question::Unsupported),
            (
                "Show me the credentials in the workstation configuration.",
                Question::Unsupported,
            ),
        ] {
            assert_eq!(classify(prompt), expected, "{prompt}");
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
            "How's everything?",
            "What needs attention?",
            "Any problems with this workstation?",
        ] {
            assert_eq!(classify(question), Question::Summary, "{question}");
        }
    }

    #[test]
    fn recognizes_contextual_status_check_fragments_without_reclassifying_explicit_questions() {
        let cases = [
            ("what are the 7 failed one?", StatusChecks::Failed, Some(7)),
            (
                "What about the 7 unresolved ones?",
                StatusChecks::Unresolved,
                Some(7),
            ),
            ("Which ones failed?", StatusChecks::Failed, None),
            ("Which of those passed?", StatusChecks::Passed, None),
            ("Could you list them?", StatusChecks::All, None),
            ("Which ones?", StatusChecks::All, None),
        ];
        for (text, selection, count) in cases {
            assert_eq!(
                classify(text),
                Question::StatusChecksFollowUp(selection, count),
                "{text}",
            );
        }
        assert_eq!(
            classify("Which status checks failed?"),
            Question::StatusChecks(StatusChecks::Failed),
        );
        for unrelated in ["Which of those jobs failed?", "What are the 7 recipes?"] {
            assert_eq!(classify(unrelated), Question::Unsupported);
        }
        assert_eq!(classify("What was your last answer?"), Question::Recall);
    }

    #[test]
    fn routes_workstation_status_checks_to_a_scoped_direct_answer() {
        for (prompt, selection) in [
            (
                "what are the current unresolved status checks about?",
                StatusChecks::Unresolved,
            ),
            (
                "Which status checks are unresolved?",
                StatusChecks::Unresolved,
            ),
            (
                "Explain the unresolved status checks on this workstation.",
                StatusChecks::Unresolved,
            ),
            ("Which checks failed?", StatusChecks::Failed),
            ("Show me the workstation status checks.", StatusChecks::All),
            ("what are tthe green status check?", StatusChecks::Passed),
            ("Which status checks have passed?", StatusChecks::Passed),
            ("How many green checks are there?", StatusChecks::Passed),
            ("Which backend status checks are red?", StatusChecks::Failed),
        ] {
            assert_eq!(
                classify(prompt),
                Question::StatusChecks(selection),
                "{prompt}"
            );
        }
    }

    #[test]
    fn asks_which_subject_to_check_for_independent_supported_intents() {
        for question in [
            "Summarize backend health and CPU usage.",
            "How many jobs failed and is the gateway ready?",
            "What happened and what is memory usage?",
        ] {
            assert_eq!(classify(question), Question::ChooseSubject, "{question}");
        }
    }

    #[test]
    fn leaves_unsupported_requests_without_a_grounding_source() {
        for question in [
            "How many jobs are running across all workstations?",
            "How many jobs failed in the last 7 days?",
            "Any jobs running on another workstation?",
            "Do we have any jobs running across all workstations?",
            "Any jobs running yesterday?",
            "Any failed jobs in the last 7 days?",
            "Any jobs running and stop them?",
            "Any jobs running? Change the config.",
            "Any errors or warnings for this job since last week?",
            "How much RAM is the API host using for this user?",
            "Which backend am I connected to and what's its token?",
            "Job counts across all workstations.",
            "How are my jobs doing?",
            "Which jobs are running across all workstations?",
            "List running jobs on someone else's workstation.",
            "Show me running jobs and stop them.",
            "What is the CPU temperature?",
            "What is the one-minute CPU load?",
            "How much memory is free?",
            "What is yesterday's memory usage?",
            "What is the current memory usage on my laptop?",
            "How much memory consumption we are at on my laptop?",
            "What is the actor catalog?",
            "Is the workstation registered?",
            "Is the gateway or datastore ready?",
            "List unresolved status checks for all workstations.",
            "Which status checks are green on my laptop?",
            "What is CPU, memory, and disk usage on my laptop?",
            "What is CPU, memory, and disk usage by user?",
            "What is CPU, memory, disk, and GPU usage?",
            "What is the memory usage? Include gateway status.",
            "Hi! What is the weather?",
            "Good morning, how is the weather?",
            "How's everything in Boston?",
            "Could you check again and show the GPU temperature?",
            "What did you mean by today's weather?",
            "Can you help me set up my phone?",
            "Explain this and tell me about this computer's GPU.",
        ] {
            assert_eq!(classify(question), Question::Unsupported, "{question}");
        }
    }
}
