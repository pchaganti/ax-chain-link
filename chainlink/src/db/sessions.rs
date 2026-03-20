use anyhow::Result;
use chrono::Utc;
use rusqlite::params;

use super::{parse_datetime, Database};
use crate::models::Session;

impl Database {
    /// Convenience wrapper for tests — production code uses `start_session_with_agent`.
    #[cfg(test)]
    pub fn start_session(&self) -> Result<i64> {
        self.start_session_with_agent(None)
    }

    pub fn start_session_with_agent(&self, agent_id: Option<&str>) -> Result<i64> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO sessions (started_at, agent_id) VALUES (?1, ?2)",
            params![now, agent_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn end_session(&self, id: i64, notes: Option<&str>) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let rows = self.conn.execute(
            "UPDATE sessions SET ended_at = ?1, handoff_notes = ?2 WHERE id = ?3",
            params![now, notes, id],
        )?;
        Ok(rows > 0)
    }

    pub fn get_current_session(&self) -> Result<Option<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, started_at, ended_at, active_issue_id, handoff_notes, last_action, agent_id FROM sessions WHERE ended_at IS NULL ORDER BY id DESC LIMIT 1",
        )?;

        let session = stmt
            .query_row([], |row| {
                Ok(Session {
                    id: row.get(0)?,
                    started_at: parse_datetime(row.get::<_, String>(1)?),
                    ended_at: row.get::<_, Option<String>>(2)?.map(parse_datetime),
                    active_issue_id: row.get(3)?,
                    handoff_notes: row.get(4)?,
                    last_action: row.get(5)?,
                    agent_id: row.get(6)?,
                })
            })
            .ok();

        Ok(session)
    }

    pub fn get_last_session(&self) -> Result<Option<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, started_at, ended_at, active_issue_id, handoff_notes, last_action, agent_id FROM sessions WHERE ended_at IS NOT NULL ORDER BY id DESC LIMIT 1",
        )?;

        let session = stmt
            .query_row([], |row| {
                Ok(Session {
                    id: row.get(0)?,
                    started_at: parse_datetime(row.get::<_, String>(1)?),
                    ended_at: row.get::<_, Option<String>>(2)?.map(parse_datetime),
                    active_issue_id: row.get(3)?,
                    handoff_notes: row.get(4)?,
                    last_action: row.get(5)?,
                    agent_id: row.get(6)?,
                })
            })
            .ok();

        Ok(session)
    }

    pub fn set_session_issue(&self, session_id: i64, issue_id: i64) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE sessions SET active_issue_id = ?1 WHERE id = ?2",
            params![issue_id, session_id],
        )?;
        Ok(rows > 0)
    }

    pub fn set_session_action(&self, session_id: i64, action: &str) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE sessions SET last_action = ?1 WHERE id = ?2",
            params![action, session_id],
        )?;
        Ok(rows > 0)
    }
}
