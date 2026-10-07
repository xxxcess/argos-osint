with open("crates/argos-osint-core/src/intel_recon/worker.rs", "r") as f:
    text = f.read()

import re

# find:
#    let result = executor
#        .run_configured(
#            "firecrawl_search",

# and insert `insert_report_attempt` before it
# wait, `store` is opened at the top of `run_collect` in a block `{ let store = Store::open... }`
# we should open it again to insert the attempt, or reuse.

replacement = """
    let attempt_id = {
        let store = Store::open(&runtime.db_path)?;
        store.insert_report_attempt(&job.id, job.generation, "", "firecrawl_search")?
    };

    let result = executor
        .run_configured(
            "firecrawl_search",
            serde_json::json!({"query": query, "limit": 5}),
            if ua.trim().is_empty() {
                None
            } else {
                Some(ua.as_str())
            },
            &runtime.keys,
        )
        .await;

    {
        let store = Store::open(&runtime.db_path)?;
        let state = if result.is_ok() { "success" } else { "failed" };
        let _ = store.finish_report_attempt(&job.id, &attempt_id, state);
    }
"""

# Wait, `task` id is needed? "task_id: &str" is an argument to `insert_report_attempt`.
# `run_collect` currently only takes `(runtime: &JobRuntime, job: &super::persist::IntelReportJobRow)`.
# Let's see if we can pass task_id to `run_collect`
