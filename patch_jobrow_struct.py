import sys

content = open("crates/argos-osint-core/src/jobs_view.rs").read()

old_jobrow = """    pub error_summary: String,
    pub correlation_id: String,
    /// Child jobs directly under this one.
    pub children: i64,"""
new_jobrow = """    pub error_summary: String,
    pub correlation_id: String,
    pub stage_coverage_json: String,
    /// Child jobs directly under this one.
    pub children: i64,"""

if old_jobrow in content:
    content = content.replace(old_jobrow, new_jobrow)
    open("crates/argos-osint-core/src/jobs_view.rs", "w").write(content)
    print("Patched JobRow struct")
else:
    print("Could not find JobRow block")
