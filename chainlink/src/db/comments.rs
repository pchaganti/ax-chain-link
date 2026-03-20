use anyhow::Result;
use chrono::Utc;
use rusqlite::params;

use super::{parse_datetime, Database, MAX_COMMENT_LEN};
use crate::models::Comment;

impl Database {
    pub fn add_comment(&self, issue_id: i64, content: &str, kind: &str) -> Result<i64> {
        if content.len() > MAX_COMMENT_LEN {
            anyhow::bail!(
                "Comment exceeds maximum length of {} bytes",
                MAX_COMMENT_LEN
            );
        }
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO comments (issue_id, content, created_at, kind) VALUES (?1, ?2, ?3, ?4)",
            params![issue_id, content, now, kind],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_comments(&self, issue_id: i64) -> Result<Vec<Comment>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, issue_id, content, created_at, kind FROM comments WHERE issue_id = ?1 ORDER BY created_at",
        )?;
        let comments = stmt
            .query_map([issue_id], |row| {
                Ok(Comment {
                    id: row.get(0)?,
                    issue_id: row.get(1)?,
                    content: row.get(2)?,
                    created_at: parse_datetime(row.get::<_, String>(3)?),
                    kind: row.get::<_, Option<String>>(4)?.unwrap_or_else(|| "note".to_string()),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(comments)
    }
}
