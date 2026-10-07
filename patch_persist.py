with open("crates/argos-osint-core/src/intel_recon/persist.rs", "r") as f:
    text = f.read()

# Add attempt methods
attempt_methods = """
    pub fn insert_report_attempt(
        &self,
        job_id: &str,
        generation: i64,
        task_id: &str,
        tool_id: &str,
    ) -> Result<String> {
        let id = new_row_id("atmpt");
        self.conn.execute(
            "INSERT INTO intel_report_attempts (id, job_id, generation, task_id, tool_id, state, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6)",
            rusqlite::params![id, job_id, generation, task_id, tool_id, now()],
        )?;
        // also increment tool_calls_done on the job
        self.conn.execute(
            "UPDATE intel_report_jobs SET tool_calls_done = tool_calls_done + 1, current_tool = ?2 WHERE id = ?1",
            rusqlite::params![job_id, tool_id]
        )?;
        Ok(id)
    }

    pub fn finish_report_attempt(&self, job_id: &str, id: &str, state: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE intel_report_attempts SET state=?2, finished_at=?3 WHERE id=?1",
            rusqlite::params![id, state, now()],
        )?;
        self.conn.execute(
            "UPDATE intel_report_jobs SET current_tool = '' WHERE id = ?1",
            rusqlite::params![job_id]
        )?;
        Ok(())
    }
"""

text = text.replace("pub fn update_report_job(", attempt_methods + "\n    pub fn update_report_job(")
with open("crates/argos-osint-core/src/intel_recon/persist.rs", "w") as f:
    f.write(text)
