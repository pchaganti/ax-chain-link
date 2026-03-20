use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::params;

use super::{parse_datetime, Database};

impl Database {
    pub fn start_timer(&self, issue_id: i64) -> Result<i64> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO time_entries (issue_id, started_at) VALUES (?1, ?2)",
            params![issue_id, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn stop_timer(&self, issue_id: i64) -> Result<bool> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        let started_at: Option<String> = self
            .conn
            .query_row(
                "SELECT started_at FROM time_entries WHERE issue_id = ?1 AND ended_at IS NULL",
                [issue_id],
                |row| row.get(0),
            )
            .ok();

        if let Some(started) = started_at {
            let start_dt = DateTime::parse_from_rfc3339(&started)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or(now);
            let duration = now.signed_duration_since(start_dt).num_seconds();

            let rows = self.conn.execute(
                "UPDATE time_entries SET ended_at = ?1, duration_seconds = ?2 WHERE issue_id = ?3 AND ended_at IS NULL",
                params![now_str, duration, issue_id],
            )?;
            Ok(rows > 0)
        } else {
            Ok(false)
        }
    }

    pub fn get_active_timer(&self) -> Result<Option<(i64, DateTime<Utc>)>> {
        let result: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT issue_id, started_at FROM time_entries WHERE ended_at IS NULL ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();

        Ok(result.map(|(id, started)| (id, parse_datetime(started))))
    }

    pub fn get_total_time(&self, issue_id: i64) -> Result<i64> {
        let total: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(duration_seconds), 0) FROM time_entries WHERE issue_id = ?1 AND duration_seconds IS NOT NULL",
                [issue_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(total)
    }
}
