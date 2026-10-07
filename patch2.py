import sys

content = open("crates/argos-osint-core/src/jobs_view.rs").read()

old_jobrow = """    pub error_summary: String,
    pub correlation_id: String,
    pub children: i64,"""
new_jobrow = """    pub error_summary: String,
    pub correlation_id: String,
    pub stage_coverage_json: String,
    pub children: i64,"""

content = content.replace(old_jobrow, new_jobrow)

old_columns = """    j.error_category, j.error_summary, j.correlation_id,
    (SELECT COUNT(*) FROM argos_jobs c WHERE c.parent_id = j.id),"""
new_columns = """    j.error_category, j.error_summary, j.correlation_id, j.stage_coverage_json,
    (SELECT COUNT(*) FROM argos_jobs c WHERE c.parent_id = j.id),"""

content = content.replace(old_columns, new_columns)

old_parse = """        error_summary: r.get(28)?,
        correlation_id: r.get(29)?,
        children: r.get(30)?,
        events: r.get(31)?,
        attempt_rows: r.get(32)?,
        open_attempt_started: r.get(33)?,
        active_since: r.get(34)?,
        cancel_requested: r.get(35)?,
        cancellable: r.get(36)?,"""
new_parse = """        error_summary: r.get(28)?,
        correlation_id: r.get(29)?,
        stage_coverage_json: r.get(30)?,
        children: r.get(31)?,
        events: r.get(32)?,
        attempt_rows: r.get(33)?,
        open_attempt_started: r.get(34)?,
        active_since: r.get(35)?,
        cancel_requested: r.get(36)?,
        cancellable: r.get(37)?,"""

content = content.replace(old_parse, new_parse)

open("crates/argos-osint-core/src/jobs_view.rs", "w").write(content)
print("patched")
