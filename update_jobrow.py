import sys

content = open("crates/argos-osint-core/src/jobs_view.rs").read()
old = """    pub attempt_cap: i64,
    pub children: i64,
    pub events: i64,
    pub error_summary: String,"""

new = """    pub attempt_cap: i64,
    pub children: i64,
    pub events: i64,
    pub error_summary: String,
    pub stage_coverage: serde_json::Value,"""

if old in content:
    open("crates/argos-osint-core/src/jobs_view.rs", "w").write(content.replace(old, new))
else:
    print("Old not found")

