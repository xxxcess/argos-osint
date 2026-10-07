import sys

content = open("crates/argos-osint-core/src/jobs_view.rs").read()

new_fn = """
    pub fn set_job_stage_coverage(&self, job_id: &str, coverage_json: &str) -> Result<()> {
        crate::tasks::set_job_stage_coverage(&self.conn, job_id, coverage_json)
    }
"""

target = "    pub fn retry_failed_tasks(&self, id: &str) -> Result<usize> {\n        retry_failed_tasks(&self.conn, id)\n    }"

if new_fn not in content and target in content:
    content = content.replace(target, target + new_fn)
    open("crates/argos-osint-core/src/jobs_view.rs", "w").write(content)
    print("patched jobs_view.rs")
