//! Standalone operational logs, independent of scheduler ownership.

use anyhow::{ensure, Result};
use rusqlite::params;
use serde_json::{json, Value};

use crate::Store;

/// Filter for a bounded newest-first operational log page.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct LogFilter {
    pub min_rank: Option<i64>,
    pub app: String,
    pub job_ids: Vec<String>,
    pub text: String,
    pub since: String,
    pub before_seq: Option<i64>,
    pub limit: usize,
}

impl Store {
    /// Appends an idempotent log, allocating its sequence inside SQLite.
    pub async fn log_append(&self, mut event: Value) -> Result<String> {
        let id = event["id"].as_str().unwrap_or_default().to_owned();
        let ts = event["ts"].as_str().unwrap_or_default().to_owned();
        ensure!(
            !id.is_empty() && !ts.is_empty(),
            "log identity and timestamp required"
        );
        event["seq"] = json!(0);
        let body = serde_json::to_string(&event)?;
        self.with_connection(move |connection| {
            use rusqlite::OptionalExtension;
            let tx = connection.transaction()?;
            let previous: Option<String> = tx
                .query_row("SELECT body FROM logs WHERE id=?1", [&id], |row| row.get(0))
                .optional()?;
            if let Some(previous) = previous {
                ensure!(
                    previous == body,
                    "log identity reused for different content"
                );
            } else {
                tx.execute(
                    "INSERT INTO logs(id,body,ts) VALUES(?1,?2,?3)",
                    params![id, body, ts],
                )?;
            }
            tx.commit()?;
            Ok(id)
        })
        .await
    }

    /// Lists logs in descending durable sequence order.
    pub async fn log_list(&self, filter: LogFilter) -> Result<Vec<Value>> {
        self.with_connection(move |connection| {
            let jobs = serde_json::to_string(&filter.job_ids)?;
            let mut query = connection.prepare("SELECT seq,body FROM logs WHERE
                (?1 IS NULL OR CASE json_extract(body,'$.severity') WHEN 'debug' THEN 0 WHEN 'warn' THEN 2 WHEN 'error' THEN 3 ELSE 1 END >= ?1)
                AND (?2='' OR json_extract(body,'$.app')=?2)
                AND (?3='[]' OR json_extract(body,'$.job_id') IN (SELECT value FROM json_each(?3)))
                AND (?4='' OR instr(json_extract(body,'$.message'),?4)>0 OR instr(json_extract(body,'$.details'),?4)>0)
                AND (?5='' OR ts>=?5) AND (?6 IS NULL OR seq<?6)
                ORDER BY seq DESC LIMIT ?7")?;
            let rows = query.query_map(params![filter.min_rank,filter.app,jobs,filter.text,filter.since,filter.before_seq,filter.limit.clamp(1,1000) as i64], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?)))?;
            let mut events = Vec::new();
            for row in rows { let (seq,body)=row?; let mut body:Value=serde_json::from_str(&body)?; body["seq"]=json!(seq); events.push(body); }
            Ok(events)
        }).await
    }

    /// Looks up one log without scanning history.
    pub async fn log_get(&self, id: String) -> Result<Option<Value>> {
        self.with_connection(move |connection| {
            use rusqlite::OptionalExtension;
            let row = connection
                .query_row("SELECT seq,body FROM logs WHERE id=?1", [id], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .optional()?;
            row.map(|(seq, body)| {
                let mut body: Value = serde_json::from_str(&body)?;
                body["seq"] = json!(seq);
                Ok(body)
            })
            .transpose()
        })
        .await
    }

    /// Counts severities and failures since the supplied timestamp.
    pub async fn log_counts(&self, since: String) -> Result<Value> {
        self.with_connection(move |connection| {
            let mut counts=json!({"error":0,"warn":0,"info":0,"debug":0,"recent_failures":0});
            let mut statement=connection.prepare("SELECT CASE json_extract(body,'$.severity') WHEN 'error' THEN 'error' WHEN 'warn' THEN 'warn' WHEN 'debug' THEN 'debug' ELSE 'info' END AS severity,count(*) FROM logs GROUP BY severity")?;
            let rows=statement.query_map([],|row|Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?)))?;
            for row in rows { let (severity,n)=row?; counts[severity]=json!(n); }
            let recent:i64=connection.query_row("SELECT count(*) FROM logs WHERE json_extract(body,'$.severity')='error' AND ts>=?1",[since],|row|row.get(0))?;
            counts["recent_failures"]=json!(recent); Ok(counts)
        }).await
    }

    /// Returns sorted distinct nonempty apps, capped at the existing UI limit.
    pub async fn log_apps(&self) -> Result<Vec<String>> {
        self.with_connection(|connection| {
            let mut query=connection.prepare("SELECT DISTINCT json_extract(body,'$.app') AS app FROM logs WHERE app IS NOT NULL AND app!='' ORDER BY app LIMIT 32")?;
            let rows = query.query_map([], |row| row.get(0))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        }).await
    }

    /// Deletes logs older than the supplied timestamp, or all logs with None.
    pub async fn log_prune(&self, before: Option<String>) -> Result<usize> {
        self.with_connection(move |connection| {
            Ok(connection.execute("DELETE FROM logs WHERE ?1 IS NULL OR ts<?1", [before])?)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::LogFilter;
    use crate::Store;
    use serde_json::json;

    #[tokio::test]
    async fn log_reopen_retry_filter_and_retention() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("jobs.sqlite3");
        let store = Store::open(&path).await?;
        let first = json!({"id":"one","seq":0,"ts":"2026-01-01T00:00:00Z","severity":"info","app":"brain","job_id":"parent","message":"original","details":""});
        store.log_append(first.clone()).await?;
        store.log_append(first).await?;
        let second = json!({"id":"two","seq":0,"ts":"2026-01-02T00:00:00Z","severity":"error","app":"tools","job_id":"child","message":"failed needle","details":""});
        store.log_append(second).await?;
        drop(store);
        let store = Store::open(&path).await?;
        let counts = store.log_counts("2026-01-02T00:00:00Z".into()).await?;
        assert_eq!(counts["info"], 1);
        assert_eq!(counts["error"], 1);
        assert_eq!(counts["recent_failures"], 1);
        let filter = LogFilter {
            job_ids: vec!["parent".into(), "child".into()],
            min_rank: Some(2),
            text: "needle".into(),
            limit: 20,
            ..Default::default()
        };
        let rows = store.log_list(filter).await?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], "two");
        let high = rows[0]["seq"].as_i64().unwrap();
        let rows = store
            .log_list(LogFilter {
                before_seq: Some(high),
                limit: 20,
                ..Default::default()
            })
            .await?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], "one");
        assert_eq!(store.log_apps().await?, vec!["brain", "tools"]);
        assert_eq!(
            store.log_prune(Some("2026-01-02T00:00:00Z".into())).await?,
            1
        );
        assert!(store.log_get("one".into()).await?.is_none());
        assert_eq!(store.log_prune(None).await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn log_identity_cannot_overwrite_another_event() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let store = Store::open(directory.path().join("jobs.sqlite3")).await?;
        let event = json!({"id":"one","ts":"2026-01-01T00:00:00Z","message":"original"});
        store.log_append(event.clone()).await?;
        let mut conflicting = event;
        conflicting["message"] = json!("changed");
        assert!(store.log_append(conflicting).await.is_err());
        assert_eq!(
            store.log_get("one".into()).await?.unwrap()["message"],
            "original"
        );
        Ok(())
    }
}
