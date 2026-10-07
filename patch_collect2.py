import re

with open("crates/argos-osint-core/src/intel_recon/worker.rs", "r") as f:
    text = f.read()

text = text.replace(
    '"collect" => run_collect(&runtime, &job).await,',
    '"collect" => run_collect(&runtime, &job, &task.id).await,'
)

text = text.replace(
    'async fn run_collect(\n    runtime: &JobRuntime,\n    job: &super::persist::IntelReportJobRow,\n) -> Result<String> {',
    'async fn run_collect(\n    runtime: &JobRuntime,\n    job: &super::persist::IntelReportJobRow,\n    task_id: &str,\n) -> Result<String> {'
)

old_executor_call = """
    let executor = crate::osint::Executor::new()?;
    let query = runtime.article_title.chars().take(80).collect::<String>();
    let ua = runtime.settings.osint_user_agent.clone();
    let result = executor
        .run_configured(
            "firecrawl_search",
            json!({"query": query, "limit": 5}),
            if ua.trim().is_empty() {
                None
            } else {
                Some(ua.as_str())
            },
            &runtime.keys,
        )
        .await;

    let store = Store::open(&runtime.db_path)?;
    match result {
"""

new_executor_call = """
    let executor = crate::osint::Executor::new()?;
    let query = runtime.article_title.chars().take(80).collect::<String>();
    let ua = runtime.settings.osint_user_agent.clone();

    let attempt_id = {
        let store = Store::open(&runtime.db_path)?;
        store.insert_report_attempt(&job.id, job.generation, task_id, "firecrawl_search")?
    };

    let result = executor
        .run_configured(
            "firecrawl_search",
            json!({"query": query, "limit": 5}),
            if ua.trim().is_empty() {
                None
            } else {
                Some(ua.as_str())
            },
            &runtime.keys,
        )
        .await;

    let store = Store::open(&runtime.db_path)?;
    let state = if result.is_ok() { "success" } else { "failed" };
    let _ = store.finish_report_attempt(&job.id, &attempt_id, state);

    match result {
"""

text = text.replace(old_executor_call.strip(), new_executor_call.strip())

with open("crates/argos-osint-core/src/intel_recon/worker.rs", "w") as f:
    f.write(text)

