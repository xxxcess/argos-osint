//! Failure diagnostics for flaky scheduler tests: a plain-text dump of the
//! job, task, lease and attempt tables, so a CI failure shows the state that
//! caused it instead of only the assertion that noticed it.

const TABLES: &[&str] = &[
    "argos_jobs",
    "argos_tasks",
    "argos_attempts",
    "insight_owner_state",
];
