//! SQLite case desk. Tool-call inspection is test-only.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::brain::Memory;
use crate::report::ReportMeta;
use crate::session::Case;
use crate::tna::TnaSnapshot;

static IDS: AtomicU64 = AtomicU64::new(1);

pub fn new_id(prefix: &str) -> String {
    let n = IDS.fetch_add(1, Ordering::Relaxed);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{prefix}-{millis:x}-{n}")
}

#[derive(Clone, Debug)]
pub struct ChatLine {
    pub role: String,
    pub body: String,
    pub created_at: String,
}

pub struct Store {
    pub(crate) conn: Connection,
}

/// Reviewable counts for a case-only destructive operation. Reports are retained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseDataPlan {
    pub case_id: String,
    pub title: String,
    pub delete_case: bool,
    pub messages: usize,
    pub records: usize,
    pub decisions: usize,
    pub jobs: usize,
    pub reports: usize,
}

impl CaseDataPlan {
    pub fn describe(&self) -> String {
        format!("{} case: {} ({})\n\nRemove {} chat messages, {} case evidence/history records, {} review decisions, and {} research jobs/cache entries. Clear the case network and tool-call history.\n{} saved reports and their versions, citations, and report-owned evidence remain available, detached from this case. Other cases, shared evidence, and credentials are preserved.\n\n{}\nThis cannot be undone. /confirm-case {} to apply · /cancel-case-data to cancel",
            if self.delete_case { "Delete" } else { "Clear" }, self.title, self.case_id,
            self.messages, self.records, self.decisions, self.jobs, self.reports,
            if self.delete_case { "The case itself will be removed." } else { "The case will remain empty and can be reused." }, self.case_id)
    }
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        let store = Self { conn };
        store.ensure_schema()?;
        Ok(store)
    }

    pub fn memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self { conn };
        store.ensure_schema()?;
        Ok(store)
    }

    fn ensure_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                kind TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                body TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                text TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS reports (
                id TEXT PRIMARY KEY,
                case_id TEXT,
                title TEXT NOT NULL,
                path TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS tool_calls (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                name TEXT NOT NULL,
                status TEXT NOT NULL,
                detail TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS tna_graphs (
                key TEXT PRIMARY KEY,
                snapshot_json TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS case_gaps (
                id TEXT PRIMARY KEY,
                case_id TEXT NOT NULL,
                body TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS case_gaps_case ON case_gaps(case_id);
            CREATE TABLE IF NOT EXISTS case_data_resets (
                case_id TEXT PRIMARY KEY,
                cleared_at TEXT NOT NULL,
                deleted INTEGER NOT NULL
            );
            "#,
        )?;
        self.ensure_memory_columns()?;
        self.ensure_evidence_schema()?;
        Ok(())
    }

    fn ensure_memory_columns(&self) -> Result<()> {
        let mut stmt = self.conn.prepare("PRAGMA table_info(memories)")?;
        let cols = stmt.query_map([], |row| row.get::<_, String>(1))?;
        let cols: Vec<String> = cols.collect::<Result<Vec<_>, _>>()?;
        if !cols.iter().any(|col| col == "category") {
            self.conn.execute(
                "ALTER TABLE memories ADD COLUMN category TEXT NOT NULL DEFAULT 'fact'",
                [],
            )?;
        }
        if !cols.iter().any(|col| col == "pinned") {
            self.conn.execute(
                "ALTER TABLE memories ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !cols.iter().any(|col| col == "report_id") {
            self.conn
                .execute("ALTER TABLE memories ADD COLUMN report_id TEXT", [])?;
        }
        self.conn
            .execute("UPDATE memories SET category = 'fact'", [])?;
        Ok(())
    }

    pub fn ensure_session(&self, id: &str, title: &str, kind: &str) -> Result<()> {
        if kind == "case" {
            self.check_case_write(id, None)?;
        }
        let now = stamp();
        self.conn.execute(
            "INSERT INTO sessions (id, title, kind, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET title = excluded.title, updated_at = excluded.updated_at",
            params![id, title, kind, now],
        )?;
        Ok(())
    }

    pub fn list_cases(&self) -> Result<Vec<Case>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title FROM sessions WHERE kind = 'case' ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Case {
                id: row.get(0)?,
                title: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn delete_case(&self, id: &str) -> Result<()> {
        let plan = self.plan_case_data(id, true)?;
        self.apply_case_data_plan(&plan)
    }

    pub fn plan_case_data(&self, id: &str, delete_case: bool) -> Result<CaseDataPlan> {
        let title = self
            .conn
            .query_row(
                "SELECT title FROM sessions WHERE id=?1 AND kind='case'",
                [id],
                |r| r.get(0),
            )
            .context("Case not found")?;
        let count = |sql: &str| -> Result<usize> {
            Ok(self.conn.query_row(sql, [id], |r| r.get::<_, i64>(0))? as usize)
        };
        let active = count("SELECT COUNT(*) FROM research_jobs WHERE json_extract(body,'$.input.case_id')=?1 AND state IN ('\"queued\"','\"running\"')")?;
        anyhow::ensure!(active == 0, "Cancel or finish this case's {active} active research jobs before clearing or deleting it");
        Ok(CaseDataPlan {
            case_id: id.into(), title, delete_case,
            messages: count("SELECT COUNT(*) FROM messages WHERE session_id=?1")?,
            records: count("SELECT COUNT(*) FROM evidence_records WHERE case_id=?1 AND (report_id IS NULL OR kind IN ('case_ingestion','investigation_scope','entity_correction','identity_resolution'))")? + count("SELECT COUNT(*) FROM case_gaps WHERE case_id=?1")?,
            decisions: count("SELECT COUNT(*) FROM finding_decisions WHERE observation_id IN (SELECT json_extract(body,'$.id') FROM evidence_records WHERE case_id=?1 AND kind='observation' AND report_id IS NULL)")?,
            jobs: count("SELECT COUNT(*) FROM research_jobs WHERE json_extract(body,'$.input.case_id')=?1")?,
            reports: count("SELECT COUNT(*) FROM reports WHERE case_id=?1")?,
        })
    }

    /// Atomic case cleanup. Recheck the reviewed counts and active jobs under the write lock.
    pub fn apply_case_data_plan(&self, plan: &CaseDataPlan) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        anyhow::ensure!(
            self.plan_case_data(&plan.case_id, plan.delete_case)? == *plan,
            "Case data changed; review a fresh clear/delete plan"
        );
        let id = &plan.case_id;
        tx.execute("INSERT INTO case_data_resets VALUES(?1,?2,?3) ON CONFLICT(case_id) DO UPDATE SET cleared_at=excluded.cleared_at,deleted=excluded.deleted", params![id, chrono::Utc::now().to_rfc3339(), plan.delete_case])?;
        tx.execute("DELETE FROM finding_decisions WHERE observation_id IN (SELECT json_extract(body,'$.id') FROM evidence_records WHERE case_id=?1 AND kind='observation' AND report_id IS NULL)", [id])?;
        tx.execute("DELETE FROM evidence_records WHERE case_id=?1 AND (report_id IS NULL OR kind IN ('case_ingestion','investigation_scope','entity_correction','identity_resolution'))", [id])?;
        // Report-backed artifacts and raw history remain reachable by historical citations.
        tx.execute("UPDATE evidence_records SET case_id=NULL,body=CASE WHEN json_type(body,'$.case_id') IS NOT NULL THEN json_set(body,'$.case_id',NULL) ELSE body END WHERE case_id=?1", [id])?;
        tx.execute("UPDATE reports SET case_id=NULL WHERE case_id=?1", [id])?;
        tx.execute(
            "DELETE FROM research_jobs WHERE json_extract(body,'$.input.case_id')=?1",
            [id],
        )?;
        tx.execute("DELETE FROM case_gaps WHERE case_id=?1", [id])?;
        tx.execute("DELETE FROM messages WHERE session_id=?1", [id])?;
        tx.execute("DELETE FROM tool_calls WHERE session_id=?1", [id])?;
        tx.execute(
            "DELETE FROM tna_graphs WHERE key=?1 OR key='desk'",
            [format!("case:{id}")],
        )?;
        if plan.delete_case {
            tx.execute("DELETE FROM sessions WHERE id=?1 AND kind='case'", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn check_case_write(&self, id: &str, job_created_at: Option<&str>) -> Result<()> {
        use rusqlite::OptionalExtension;
        let reset: Option<(String, bool)> = self
            .conn
            .query_row(
                "SELECT cleared_at,deleted FROM case_data_resets WHERE case_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((cleared_at, deleted)) = reset {
            anyhow::ensure!(!deleted, "Case was deleted; stale work discarded");
            if let Some(created_at) = job_created_at {
                anyhow::ensure!(
                    chrono::DateTime::parse_from_rfc3339(created_at)?
                        > chrono::DateTime::parse_from_rfc3339(&cleared_at)?,
                    "Case data was cleared; stale research results discarded"
                );
            }
        }
        Ok(())
    }

    pub fn create_case(&self, title: &str) -> Result<Case> {
        let case = Case {
            id: new_id("case"),
            title: title.trim().to_string(),
        };
        self.ensure_session(&case.id, &case.title, "case")?;
        Ok(case)
    }

    pub fn touch(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![stamp(), id],
        )?;
        Ok(())
    }

    /// Replace the newest message of `role` in the session. Inserts one when none exists.
    pub fn update_last_message(&self, session_id: &str, role: &str, body: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE messages SET body = ?1 WHERE id = (
                SELECT id FROM messages WHERE session_id = ?2 AND role = ?3 ORDER BY id DESC LIMIT 1
            )",
            params![body, session_id, role],
        )?;
        if changed == 0 {
            self.append_message(session_id, role, body)?;
        }
        Ok(())
    }

    pub fn append_message(&self, session_id: &str, role: &str, body: &str) -> Result<()> {
        self.check_case_write(session_id, None)?;
        self.conn.execute(
            "INSERT INTO messages (session_id, role, body, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![session_id, role, body, stamp()],
        )?;
        self.touch(session_id)?;
        Ok(())
    }

    pub fn load_messages(&self, session_id: &str) -> Result<Vec<ChatLine>> {
        let mut stmt = self.conn.prepare(
            "SELECT role, body, created_at FROM messages WHERE session_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(ChatLine {
                role: row.get(0)?,
                body: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn clear_messages(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM messages WHERE session_id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    pub fn list_memories(&self) -> Result<Vec<Memory>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, text, created_at, category, pinned, report_id FROM memories ORDER BY pinned DESC, created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let pinned: i64 = row.get(4)?;
            Ok(Memory {
                id: row.get(0)?,
                text: row.get(1)?,
                created_at: row.get(2)?,
                category: "fact".into(),
                pinned: pinned != 0,
                report_id: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn add_memory(&self, text: &str) -> Result<Memory> {
        self.add_memory_typed(text, "fact", false)
    }

    pub fn add_memory_typed(&self, text: &str, category: &str, pinned: bool) -> Result<Memory> {
        let memory = Memory {
            id: new_id("mem"),
            text: text.trim().to_string(),
            category: "fact".into(),
            pinned,
            created_at: stamp(),
            report_id: None,
        };
        let _ = category;
        if memory.text.is_empty() {
            anyhow::bail!("memory text is empty");
        }
        self.conn.execute(
            "INSERT INTO memories (id, text, created_at, category, pinned, report_id) VALUES (?1, ?2, ?3, 'fact', ?4, ?5)",
            params![
                memory.id,
                memory.text,
                memory.created_at,
                i64::from(memory.pinned),
                memory.report_id
            ],
        )?;
        Ok(memory)
    }

    pub fn add_report_fact(&self, text: &str, report_id: &str) -> Result<Memory> {
        let memory = Memory {
            id: new_id("mem"),
            text: text.trim().to_string(),
            category: "fact".into(),
            pinned: false,
            created_at: stamp(),
            report_id: Some(report_id.to_string()),
        };
        if memory.text.is_empty() {
            anyhow::bail!("memory text is empty");
        }
        self.conn.execute(
            "INSERT INTO memories (id, text, created_at, category, pinned, report_id) VALUES (?1, ?2, ?3, 'fact', 0, ?4)",
            params![memory.id, memory.text, memory.created_at, report_id],
        )?;
        Ok(memory)
    }

    pub fn update_memory(
        &self,
        id: &str,
        text: &str,
        category: &str,
        pinned: bool,
    ) -> Result<bool> {
        let text = text.trim();
        if text.is_empty() {
            anyhow::bail!("memory text is empty");
        }
        let _ = category;
        let n = self.conn.execute(
            "UPDATE memories SET text = ?1, category = 'fact', pinned = ?2 WHERE id = ?3",
            params![text, i64::from(pinned), id],
        )?;
        Ok(n > 0)
    }

    pub fn delete_report_bundle(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM passage_fts WHERE report_id=?1", [id])?;
        self.conn
            .execute("DELETE FROM report_passages WHERE report_id=?1", [id])?;
        self.conn
            .execute("DELETE FROM report_versions WHERE report_id=?1", [id])?;
        self.conn.execute(
            "DELETE FROM evidence_records WHERE report_id=?1 AND kind IN ('claim','mention')",
            [id],
        )?;
        self.delete_tna_graph(&crate::tna::report_key(id))?;
        self.delete_tna_graph("desk")?;
        let session = format!("report:{id}");
        self.conn
            .execute("DELETE FROM memories WHERE report_id = ?1", params![id])?;
        self.conn.execute(
            "DELETE FROM messages WHERE session_id = ?1",
            params![session],
        )?;
        self.conn
            .execute("DELETE FROM sessions WHERE id = ?1", params![session])?;
        self.conn
            .execute("DELETE FROM reports WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn delete_memory(&self, id: &str) -> Result<bool> {
        let n = self
            .conn
            .execute("DELETE FROM memories WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    pub fn add_report_metadata(&self, report: &ReportMeta) -> Result<()> {
        if let Some(id) = &report.case_id {
            self.check_case_write(id, None)?;
        }
        self.conn.execute(
            "INSERT OR REPLACE INTO reports (id, case_id, title, path, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![report.id, report.case_id, report.title, report.path, report.created_at],
        )?;

        Ok(())
    }

    pub fn add_report(&self, report: &ReportMeta) -> Result<()> {
        self.add_report_metadata(report)?;
        if let Ok(body) = std::fs::read_to_string(&report.path) {
            self.index_report(report, &body)?;
        }
        Ok(())
    }

    pub fn list_reports(&self) -> Result<Vec<ReportMeta>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, case_id, title, path, created_at FROM reports ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ReportMeta {
                id: row.get(0)?,
                case_id: row.get(1)?,
                title: row.get(2)?,
                path: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn log_call(&self, session_id: &str, name: &str, status: &str, detail: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO tool_calls (session_id, name, status, detail, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, name, status, detail, stamp()],
        )?;
        Ok(())
    }

    /// Tool-call rows for tests. Not part of the runtime API.
    #[cfg(test)]
    pub fn call_states(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, status FROM tool_calls ORDER BY id ASC")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn get_tna_graph(&self, key: &str) -> Result<Option<TnaSnapshot>> {
        let mut stmt = self
            .conn
            .prepare("SELECT snapshot_json FROM tna_graphs WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        if let Some(row) = rows.next()? {
            let json: String = row.get(0)?;
            let snap: TnaSnapshot =
                serde_json::from_str(&json).with_context(|| format!("decode tna graph {key}"))?;
            Ok((snap.pipeline_version == crate::tna::PIPELINE_VERSION).then_some(snap))
        } else {
            Ok(None)
        }
    }

    pub fn upsert_tna_graph(&self, key: &str, snapshot: &TnaSnapshot) -> Result<()> {
        let json = serde_json::to_string(snapshot).context("encode tna graph")?;
        self.conn.execute(
            "INSERT INTO tna_graphs (key, snapshot_json, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET snapshot_json = excluded.snapshot_json, updated_at = excluded.updated_at",
            params![key, json, stamp()],
        )?;
        Ok(())
    }

    pub fn delete_tna_graph(&self, key: &str) -> Result<bool> {
        let n = self
            .conn
            .execute("DELETE FROM tna_graphs WHERE key = ?1", params![key])?;
        Ok(n > 0)
    }
}

fn stamp() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case_data_fixture(store: &Store) -> (Case, Case) {
        let case = store.create_case("Clear Harbor").unwrap();
        let other = store.create_case("Other Harbor").unwrap();
        for c in [&case, &other] {
            store
                .append_message(&c.id, "user", "saved conversation")
                .unwrap();
            store
                .put_record(
                    &format!("{}:o", c.id),
                    "observation",
                    Some(&c.id),
                    None,
                    &serde_json::json!({"id":format!("{}:o",c.id)}),
                )
                .unwrap();
            store
                .review_finding(
                    &format!("{}:o", c.id),
                    crate::evidence::ReviewDecision::Accept,
                    "reviewed",
                )
                .unwrap();
            store
                .put_record(
                    "shared-entity",
                    "entity",
                    Some(&c.id),
                    None,
                    &serde_json::json!({"id":"shared-entity"}),
                )
                .unwrap();
            store
                .upsert_tna_graph(
                    &format!("case:{}", c.id),
                    &TnaSnapshot::empty(crate::tna::TnaScope::Collection),
                )
                .unwrap();
        }
        (case, other)
    }

    #[test]
    fn clear_case_removes_only_case_data_and_preserves_historical_reports() {
        let store = Store::memory().unwrap();
        let (case, other) = case_data_fixture(&store);
        let report = ReportMeta {
            id: "historic".into(),
            case_id: Some(case.id.clone()),
            title: "Historical report".into(),
            path: "missing-report.md".into(),
            created_at: "2026-09-27".into(),
        };
        store.add_report_metadata(&report).unwrap();
        store
            .index_report(&report, "# Historical\n\nSaved citation.")
            .unwrap();
        let old_body = store.report_version(&report.id, Some(1)).unwrap();
        store
            .put_record(
                "report-observation",
                "observation",
                Some(&case.id),
                Some(&report.id),
                &serde_json::json!({"id":"report-observation","case_id":case.id}),
            )
            .unwrap();
        store
            .review_finding(
                "report-observation",
                crate::evidence::ReviewDecision::Accept,
                "preserve history",
            )
            .unwrap();
        let plan = store.plan_case_data(&case.id, false).unwrap();
        assert_eq!(
            (plan.messages, plan.records, plan.decisions, plan.reports),
            (1, 2, 1, 1)
        );
        store.apply_case_data_plan(&plan).unwrap();
        assert_eq!(store.list_cases().unwrap().len(), 2);
        assert!(store.load_messages(&case.id).unwrap().is_empty());
        assert!(store
            .records::<serde_json::Value>("entity", None, Some(&case.id))
            .unwrap()
            .is_empty());
        assert_eq!(
            store
                .records::<serde_json::Value>("entity", None, Some(&other.id))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(store.load_messages(&other.id).unwrap().len(), 1);
        assert!(store
            .get_tna_graph(&format!("case:{}", case.id))
            .unwrap()
            .is_none());
        assert!(store
            .get_tna_graph(&format!("case:{}", other.id))
            .unwrap()
            .is_some());
        assert_eq!(store.report_version(&report.id, Some(1)).unwrap(), old_body);
        assert!(store.list_reports().unwrap()[0].case_id.is_none());
        assert_eq!(
            store
                .conn
                .query_row("SELECT COUNT(*) FROM finding_decisions", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(store.case_projection(&case.id).unwrap().findings.len(), 0);
        store
            .put_record(
                "new",
                "entity",
                Some(&case.id),
                None,
                &serde_json::json!({"id":"new"}),
            )
            .unwrap();
    }

    #[test]
    fn delete_case_blocks_stale_writes_and_stale_plans_are_atomic() {
        let store = Store::memory().unwrap();
        let (case, other) = case_data_fixture(&store);
        let plan = store.plan_case_data(&case.id, true).unwrap();
        store
            .append_message(&case.id, "assistant", "changed after plan")
            .unwrap();
        assert!(store
            .apply_case_data_plan(&plan)
            .unwrap_err()
            .to_string()
            .contains("changed"));
        assert_eq!(store.list_cases().unwrap().len(), 2);
        store.delete_case(&case.id).unwrap();
        assert_eq!(store.list_cases().unwrap()[0].id, other.id);
        assert!(store
            .append_message(&case.id, "assistant", "late result")
            .is_err());
        assert!(store.ensure_session(&case.id, "resurrect", "case").is_err());
        assert!(store
            .put_record(
                "late",
                "artifact",
                Some(&case.id),
                None,
                &serde_json::json!({})
            )
            .is_err());
        assert!(store.case_projection(&case.id).is_err());
    }

    #[test]
    fn cleanup_requires_idle_jobs_and_rejects_results_from_before_clear() {
        use crate::research::{JobState, ResearchInput, ResearchJob, Stage};
        let store = Store::memory().unwrap();
        let case = store.create_case("Jobs").unwrap();
        let mut job = ResearchJob {
            id: "old-job".into(),
            run_id: "run".into(),
            input: ResearchInput {
                case_id: Some(case.id.clone()),
                report_id: None,
                entity_id: "domain:harbor.example".into(),
                label: "harbor.example".into(),
                action: "domain".into(),
                depth: 0,
            },
            provider: "domain".into(),
            provider_version: None,
            metadata: vec![],
            stage: Stage::InfrastructureIp,
            state: JobState::Running,
            progress: "running".into(),
            created_at: "2020-01-01T00:00:00Z".into(),
            finished_at: None,
            elapsed_ms: 0,
            error: None,
            hits: vec![],
        };
        store.save_job("cache", &job).unwrap();
        assert!(store.plan_case_data(&case.id, false).is_err());
        job.state = JobState::Cancelled;
        store.save_job("cache", &job).unwrap();
        let plan = store.plan_case_data(&case.id, false).unwrap();
        assert_eq!(plan.jobs, 1);
        store.apply_case_data_plan(&plan).unwrap();
        assert!(store.jobs().unwrap().is_empty());
        job.state = JobState::Completed;
        assert!(store.save_job("cache", &job).is_err());
        job.id = "new-job".into();
        job.created_at = chrono::Utc::now().to_rfc3339();
        store.save_job("cache", &job).unwrap();
        assert_eq!(store.jobs().unwrap().len(), 1);
    }

    #[test]
    fn cases_messages_and_hidden_call_states() {
        let store = Store::memory().unwrap();
        let case = store.create_case("Harbor").unwrap();
        store
            .append_message(&case.id, "user", "look up the port")
            .unwrap();
        store
            .append_message(&case.id, "assistant", "searching")
            .unwrap();
        assert_eq!(store.load_messages(&case.id).unwrap().len(), 2);
        store
            .update_last_message(&case.id, "assistant", "the tide turned")
            .unwrap();
        let saved = store.load_messages(&case.id).unwrap();
        assert_eq!(saved.len(), 2);
        assert_eq!(saved[1].body, "the tide turned");
        store
            .log_call(&case.id, "web_search", "ok", "3 hits")
            .unwrap();
        assert_eq!(
            store.call_states().unwrap(),
            vec![("web_search".into(), "ok".into())]
        );
        let mem = store
            .add_memory_typed("Night desk case", "project", true)
            .unwrap();
        let listed = store.list_memories().unwrap();
        assert_eq!(listed[0].id, mem.id);
        assert_eq!(listed[0].category, "fact");
        assert!(listed[0].pinned);
        assert!(store
            .update_memory(&mem.id, "Night desk, renamed", "goal", false)
            .unwrap());
        let updated = store.list_memories().unwrap();
        assert_eq!(updated[0].text, "Night desk, renamed");
        assert_eq!(updated[0].category, "fact");
        assert!(!updated[0].pinned);
        assert!(store.delete_memory(&mem.id).unwrap());
        assert!(store.list_memories().unwrap().is_empty());
    }
    #[test]
    fn tna_unversioned_snapshot_is_stale() {
        let store = Store::memory().unwrap();
        let mut legacy = serde_json::to_value(crate::tna::TnaSnapshot::empty(
            crate::tna::TnaScope::Collection,
        ))
        .unwrap();
        legacy.as_object_mut().unwrap().remove("pipeline_version");
        legacy.as_object_mut().unwrap().remove("decisions");
        store.conn.execute("INSERT INTO tna_graphs (key, snapshot_json, updated_at) VALUES ('desk', ?1, 'old')", params![legacy.to_string()]).unwrap();
        assert!(store.get_tna_graph("desk").unwrap().is_none());
    }

    #[test]
    fn tna_graph_persist_roundtrip() {
        use crate::tna::{TnaScope, TnaSnapshot};
        let store = Store::memory().unwrap();
        let snap = TnaSnapshot::empty(TnaScope::Collection);
        store.upsert_tna_graph("desk", &snap).unwrap();
        let loaded = store.get_tna_graph("desk").unwrap().unwrap();
        assert_eq!(loaded.title, "TNA · collection");
        assert!(store.delete_tna_graph("desk").unwrap());
        assert!(store.get_tna_graph("desk").unwrap().is_none());
    }
}
