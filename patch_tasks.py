import sys

content = open("crates/argos-osint-core/src/tasks.rs").read()

new_fn = """

pub fn set_job_stage_coverage(conn: &Connection, job_id: &str, coverage_json: &str) -> Result<()> {
    conn.execute(
        "UPDATE argos_jobs SET stage_coverage_json=?2, updated_at=?3 WHERE id=?1",
        rusqlite::params![job_id, coverage_json, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}
"""

if "pub fn set_job_stage_coverage" not in content:
    content += new_fn
    open("crates/argos-osint-core/src/tasks.rs", "w").write(content)
    print("patched tasks.rs")
