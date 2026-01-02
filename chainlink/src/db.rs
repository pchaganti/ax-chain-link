use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::path::Path;

use crate::models::{Comment, Issue, Session};

const SCHEMA_VERSION: i32 = 6;

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).context("Failed to open database")?;
        let db = Database { conn };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        // Check if we need to initialize
        let version: i32 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM pragma_user_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if version < SCHEMA_VERSION {
            self.conn.execute_batch(
                r#"
                -- Core issues table
                CREATE TABLE IF NOT EXISTS issues (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    title TEXT NOT NULL,
                    description TEXT,
                    status TEXT NOT NULL DEFAULT 'open',
                    priority TEXT NOT NULL DEFAULT 'medium',
                    parent_id INTEGER,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    closed_at TEXT,
                    FOREIGN KEY (parent_id) REFERENCES issues(id) ON DELETE CASCADE
                );

                -- Labels (many-to-many)
                CREATE TABLE IF NOT EXISTS labels (
                    issue_id INTEGER NOT NULL,
                    label TEXT NOT NULL,
                    PRIMARY KEY (issue_id, label),
                    FOREIGN KEY (issue_id) REFERENCES issues(id) ON DELETE CASCADE
                );

                -- Dependencies (blocker blocks blocked)
                CREATE TABLE IF NOT EXISTS dependencies (
                    blocker_id INTEGER NOT NULL,
                    blocked_id INTEGER NOT NULL,
                    PRIMARY KEY (blocker_id, blocked_id),
                    FOREIGN KEY (blocker_id) REFERENCES issues(id) ON DELETE CASCADE,
                    FOREIGN KEY (blocked_id) REFERENCES issues(id) ON DELETE CASCADE
                );

                -- Comments
                CREATE TABLE IF NOT EXISTS comments (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    issue_id INTEGER NOT NULL,
                    content TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    FOREIGN KEY (issue_id) REFERENCES issues(id) ON DELETE CASCADE
                );

                -- Sessions (for context preservation)
                CREATE TABLE IF NOT EXISTS sessions (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    started_at TEXT NOT NULL,
                    ended_at TEXT,
                    active_issue_id INTEGER,
                    handoff_notes TEXT,
                    FOREIGN KEY (active_issue_id) REFERENCES issues(id)
                );

                -- Time tracking
                CREATE TABLE IF NOT EXISTS time_entries (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    issue_id INTEGER NOT NULL,
                    started_at TEXT NOT NULL,
                    ended_at TEXT,
                    duration_seconds INTEGER,
                    FOREIGN KEY (issue_id) REFERENCES issues(id) ON DELETE CASCADE
                );

                -- Relations (related issues, bidirectional)
                CREATE TABLE IF NOT EXISTS relations (
                    issue_id_1 INTEGER NOT NULL,
                    issue_id_2 INTEGER NOT NULL,
                    created_at TEXT NOT NULL,
                    PRIMARY KEY (issue_id_1, issue_id_2),
                    FOREIGN KEY (issue_id_1) REFERENCES issues(id) ON DELETE CASCADE,
                    FOREIGN KEY (issue_id_2) REFERENCES issues(id) ON DELETE CASCADE
                );

                -- Milestones
                CREATE TABLE IF NOT EXISTS milestones (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    description TEXT,
                    status TEXT NOT NULL DEFAULT 'open',
                    created_at TEXT NOT NULL,
                    closed_at TEXT
                );

                -- Milestone-Issue relationship (many-to-many)
                CREATE TABLE IF NOT EXISTS milestone_issues (
                    milestone_id INTEGER NOT NULL,
                    issue_id INTEGER NOT NULL,
                    PRIMARY KEY (milestone_id, issue_id),
                    FOREIGN KEY (milestone_id) REFERENCES milestones(id) ON DELETE CASCADE,
                    FOREIGN KEY (issue_id) REFERENCES issues(id) ON DELETE CASCADE
                );

                -- Indexes
                CREATE INDEX IF NOT EXISTS idx_issues_status ON issues(status);
                CREATE INDEX IF NOT EXISTS idx_issues_priority ON issues(priority);
                CREATE INDEX IF NOT EXISTS idx_labels_issue ON labels(issue_id);
                CREATE INDEX IF NOT EXISTS idx_comments_issue ON comments(issue_id);
                CREATE INDEX IF NOT EXISTS idx_deps_blocker ON dependencies(blocker_id);
                CREATE INDEX IF NOT EXISTS idx_deps_blocked ON dependencies(blocked_id);
                CREATE INDEX IF NOT EXISTS idx_issues_parent ON issues(parent_id);
                CREATE INDEX IF NOT EXISTS idx_time_entries_issue ON time_entries(issue_id);
                CREATE INDEX IF NOT EXISTS idx_relations_1 ON relations(issue_id_1);
                CREATE INDEX IF NOT EXISTS idx_relations_2 ON relations(issue_id_2);
                CREATE INDEX IF NOT EXISTS idx_milestone_issues_m ON milestone_issues(milestone_id);
                CREATE INDEX IF NOT EXISTS idx_milestone_issues_i ON milestone_issues(issue_id);
                "#,
            )?;

            // Migration: add parent_id column if upgrading from v1
            let _ = self.conn.execute(
                "ALTER TABLE issues ADD COLUMN parent_id INTEGER REFERENCES issues(id) ON DELETE CASCADE",
                [],
            );

            self.conn
                .execute(&format!("PRAGMA user_version = {}", SCHEMA_VERSION), [])?;
        }

        // Enable foreign keys
        self.conn.execute("PRAGMA foreign_keys = ON", [])?;

        Ok(())
    }

    // Issue CRUD
    pub fn create_issue(
        &self,
        title: &str,
        description: Option<&str>,
        priority: &str,
    ) -> Result<i64> {
        self.create_issue_with_parent(title, description, priority, None)
    }

    pub fn create_subissue(
        &self,
        parent_id: i64,
        title: &str,
        description: Option<&str>,
        priority: &str,
    ) -> Result<i64> {
        self.create_issue_with_parent(title, description, priority, Some(parent_id))
    }

    fn create_issue_with_parent(
        &self,
        title: &str,
        description: Option<&str>,
        priority: &str,
        parent_id: Option<i64>,
    ) -> Result<i64> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO issues (title, description, priority, parent_id, status, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, 'open', ?5, ?5)",
            params![title, description, priority, parent_id, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_subissues(&self, parent_id: i64) -> Result<Vec<Issue>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, description, status, priority, parent_id, created_at, updated_at, closed_at FROM issues WHERE parent_id = ?1 ORDER BY id",
        )?;

        let issues = stmt
            .query_map([parent_id], |row| {
                Ok(Issue {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    description: row.get(2)?,
                    status: row.get(3)?,
                    priority: row.get(4)?,
                    parent_id: row.get(5)?,
                    created_at: parse_datetime(row.get::<_, String>(6)?),
                    updated_at: parse_datetime(row.get::<_, String>(7)?),
                    closed_at: row.get::<_, Option<String>>(8)?.map(parse_datetime),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(issues)
    }

    pub fn get_issue(&self, id: i64) -> Result<Option<Issue>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, description, status, priority, parent_id, created_at, updated_at, closed_at FROM issues WHERE id = ?1",
        )?;

        let issue = stmt
            .query_row([id], |row| {
                Ok(Issue {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    description: row.get(2)?,
                    status: row.get(3)?,
                    priority: row.get(4)?,
                    parent_id: row.get(5)?,
                    created_at: parse_datetime(row.get::<_, String>(6)?),
                    updated_at: parse_datetime(row.get::<_, String>(7)?),
                    closed_at: row.get::<_, Option<String>>(8)?.map(parse_datetime),
                })
            })
            .ok();

        Ok(issue)
    }

    pub fn list_issues(
        &self,
        status_filter: Option<&str>,
        label_filter: Option<&str>,
        priority_filter: Option<&str>,
    ) -> Result<Vec<Issue>> {
        let mut sql = String::from(
            "SELECT DISTINCT i.id, i.title, i.description, i.status, i.priority, i.parent_id, i.created_at, i.updated_at, i.closed_at FROM issues i",
        );
        let mut conditions = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if label_filter.is_some() {
            sql.push_str(" JOIN labels l ON i.id = l.issue_id");
        }

        if let Some(status) = status_filter {
            if status != "all" {
                conditions.push("i.status = ?".to_string());
                params_vec.push(Box::new(status.to_string()));
            }
        }

        if let Some(label) = label_filter {
            conditions.push("l.label = ?".to_string());
            params_vec.push(Box::new(label.to_string()));
        }

        if let Some(priority) = priority_filter {
            conditions.push("i.priority = ?".to_string());
            params_vec.push(Box::new(priority.to_string()));
        }

        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }

        sql.push_str(" ORDER BY i.id DESC");

        let mut stmt = self.conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();

        let issues = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok(Issue {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    description: row.get(2)?,
                    status: row.get(3)?,
                    priority: row.get(4)?,
                    parent_id: row.get(5)?,
                    created_at: parse_datetime(row.get::<_, String>(6)?),
                    updated_at: parse_datetime(row.get::<_, String>(7)?),
                    closed_at: row.get::<_, Option<String>>(8)?.map(parse_datetime),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(issues)
    }

    pub fn update_issue(
        &self,
        id: i64,
        title: Option<&str>,
        description: Option<&str>,
        priority: Option<&str>,
    ) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let mut updates = vec!["updated_at = ?1".to_string()];
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(now)];

        if let Some(t) = title {
            updates.push(format!("title = ?{}", params_vec.len() + 1));
            params_vec.push(Box::new(t.to_string()));
        }

        if let Some(d) = description {
            updates.push(format!("description = ?{}", params_vec.len() + 1));
            params_vec.push(Box::new(d.to_string()));
        }

        if let Some(p) = priority {
            updates.push(format!("priority = ?{}", params_vec.len() + 1));
            params_vec.push(Box::new(p.to_string()));
        }

        params_vec.push(Box::new(id));
        let sql = format!(
            "UPDATE issues SET {} WHERE id = ?{}",
            updates.join(", "),
            params_vec.len()
        );

        let params_refs: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let rows = self.conn.execute(&sql, params_refs.as_slice())?;
        Ok(rows > 0)
    }

    pub fn close_issue(&self, id: i64) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let rows = self.conn.execute(
            "UPDATE issues SET status = 'closed', closed_at = ?1, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        Ok(rows > 0)
    }

    pub fn reopen_issue(&self, id: i64) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let rows = self.conn.execute(
            "UPDATE issues SET status = 'open', closed_at = NULL, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        Ok(rows > 0)
    }

    pub fn delete_issue(&self, id: i64) -> Result<bool> {
        let rows = self
            .conn
            .execute("DELETE FROM issues WHERE id = ?1", [id])?;
        Ok(rows > 0)
    }

    // Labels
    pub fn add_label(&self, issue_id: i64, label: &str) -> Result<bool> {
        let result = self.conn.execute(
            "INSERT OR IGNORE INTO labels (issue_id, label) VALUES (?1, ?2)",
            params![issue_id, label],
        )?;
        Ok(result > 0)
    }

    pub fn remove_label(&self, issue_id: i64, label: &str) -> Result<bool> {
        let rows = self.conn.execute(
            "DELETE FROM labels WHERE issue_id = ?1 AND label = ?2",
            params![issue_id, label],
        )?;
        Ok(rows > 0)
    }

    pub fn get_labels(&self, issue_id: i64) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT label FROM labels WHERE issue_id = ?1 ORDER BY label")?;
        let labels = stmt
            .query_map([issue_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(labels)
    }

    // Comments
    pub fn add_comment(&self, issue_id: i64, content: &str) -> Result<i64> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO comments (issue_id, content, created_at) VALUES (?1, ?2, ?3)",
            params![issue_id, content, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_comments(&self, issue_id: i64) -> Result<Vec<Comment>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, issue_id, content, created_at FROM comments WHERE issue_id = ?1 ORDER BY created_at",
        )?;
        let comments = stmt
            .query_map([issue_id], |row| {
                Ok(Comment {
                    id: row.get(0)?,
                    issue_id: row.get(1)?,
                    content: row.get(2)?,
                    created_at: parse_datetime(row.get::<_, String>(3)?),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(comments)
    }

    // Dependencies
    pub fn add_dependency(&self, blocked_id: i64, blocker_id: i64) -> Result<bool> {
        let result = self.conn.execute(
            "INSERT OR IGNORE INTO dependencies (blocker_id, blocked_id) VALUES (?1, ?2)",
            params![blocker_id, blocked_id],
        )?;
        Ok(result > 0)
    }

    pub fn remove_dependency(&self, blocked_id: i64, blocker_id: i64) -> Result<bool> {
        let rows = self.conn.execute(
            "DELETE FROM dependencies WHERE blocker_id = ?1 AND blocked_id = ?2",
            params![blocker_id, blocked_id],
        )?;
        Ok(rows > 0)
    }

    pub fn get_blockers(&self, issue_id: i64) -> Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT blocker_id FROM dependencies WHERE blocked_id = ?1")?;
        let blockers = stmt
            .query_map([issue_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<i64>, _>>()?;
        Ok(blockers)
    }

    pub fn get_blocking(&self, issue_id: i64) -> Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT blocked_id FROM dependencies WHERE blocker_id = ?1")?;
        let blocking = stmt
            .query_map([issue_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<i64>, _>>()?;
        Ok(blocking)
    }

    pub fn list_blocked_issues(&self) -> Result<Vec<Issue>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT DISTINCT i.id, i.title, i.description, i.status, i.priority, i.parent_id, i.created_at, i.updated_at, i.closed_at
            FROM issues i
            JOIN dependencies d ON i.id = d.blocked_id
            JOIN issues blocker ON d.blocker_id = blocker.id
            WHERE i.status = 'open' AND blocker.status = 'open'
            ORDER BY i.id
            "#,
        )?;

        let issues = stmt
            .query_map([], |row| {
                Ok(Issue {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    description: row.get(2)?,
                    status: row.get(3)?,
                    priority: row.get(4)?,
                    parent_id: row.get(5)?,
                    created_at: parse_datetime(row.get::<_, String>(6)?),
                    updated_at: parse_datetime(row.get::<_, String>(7)?),
                    closed_at: row.get::<_, Option<String>>(8)?.map(parse_datetime),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(issues)
    }

    pub fn list_ready_issues(&self) -> Result<Vec<Issue>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT i.id, i.title, i.description, i.status, i.priority, i.parent_id, i.created_at, i.updated_at, i.closed_at
            FROM issues i
            WHERE i.status = 'open'
            AND NOT EXISTS (
                SELECT 1 FROM dependencies d
                JOIN issues blocker ON d.blocker_id = blocker.id
                WHERE d.blocked_id = i.id AND blocker.status = 'open'
            )
            ORDER BY i.id
            "#,
        )?;

        let issues = stmt
            .query_map([], |row| {
                Ok(Issue {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    description: row.get(2)?,
                    status: row.get(3)?,
                    priority: row.get(4)?,
                    parent_id: row.get(5)?,
                    created_at: parse_datetime(row.get::<_, String>(6)?),
                    updated_at: parse_datetime(row.get::<_, String>(7)?),
                    closed_at: row.get::<_, Option<String>>(8)?.map(parse_datetime),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(issues)
    }

    // Sessions
    pub fn start_session(&self) -> Result<i64> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO sessions (started_at) VALUES (?1)",
            params![now],
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
            "SELECT id, started_at, ended_at, active_issue_id, handoff_notes FROM sessions WHERE ended_at IS NULL ORDER BY id DESC LIMIT 1",
        )?;

        let session = stmt
            .query_row([], |row| {
                Ok(Session {
                    id: row.get(0)?,
                    started_at: parse_datetime(row.get::<_, String>(1)?),
                    ended_at: row.get::<_, Option<String>>(2)?.map(parse_datetime),
                    active_issue_id: row.get(3)?,
                    handoff_notes: row.get(4)?,
                })
            })
            .ok();

        Ok(session)
    }

    pub fn get_last_session(&self) -> Result<Option<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, started_at, ended_at, active_issue_id, handoff_notes FROM sessions WHERE ended_at IS NOT NULL ORDER BY id DESC LIMIT 1",
        )?;

        let session = stmt
            .query_row([], |row| {
                Ok(Session {
                    id: row.get(0)?,
                    started_at: parse_datetime(row.get::<_, String>(1)?),
                    ended_at: row.get::<_, Option<String>>(2)?.map(parse_datetime),
                    active_issue_id: row.get(3)?,
                    handoff_notes: row.get(4)?,
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

    // Time tracking
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

        // Get the active entry
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

    /// Search issues by query string across titles, descriptions, and comments
    pub fn search_issues(&self, query: &str) -> Result<Vec<Issue>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            r#"
            SELECT DISTINCT i.id, i.title, i.description, i.status, i.priority, i.parent_id, i.created_at, i.updated_at, i.closed_at
            FROM issues i
            LEFT JOIN comments c ON i.id = c.issue_id
            WHERE i.title LIKE ?1 COLLATE NOCASE
               OR i.description LIKE ?1 COLLATE NOCASE
               OR c.content LIKE ?1 COLLATE NOCASE
            ORDER BY i.id DESC
            "#,
        )?;

        let issues = stmt
            .query_map([&pattern], |row| {
                Ok(Issue {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    description: row.get(2)?,
                    status: row.get(3)?,
                    priority: row.get(4)?,
                    parent_id: row.get(5)?,
                    created_at: parse_datetime(row.get::<_, String>(6)?),
                    updated_at: parse_datetime(row.get::<_, String>(7)?),
                    closed_at: row.get::<_, Option<String>>(8)?.map(parse_datetime),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(issues)
    }

    // Relations (bidirectional)
    pub fn add_relation(&self, issue_id_1: i64, issue_id_2: i64) -> Result<bool> {
        if issue_id_1 == issue_id_2 {
            anyhow::bail!("Cannot relate an issue to itself");
        }
        // Store with smaller ID first for consistency
        let (a, b) = if issue_id_1 < issue_id_2 {
            (issue_id_1, issue_id_2)
        } else {
            (issue_id_2, issue_id_1)
        };
        let now = Utc::now().to_rfc3339();
        let result = self.conn.execute(
            "INSERT OR IGNORE INTO relations (issue_id_1, issue_id_2, created_at) VALUES (?1, ?2, ?3)",
            params![a, b, now],
        )?;
        Ok(result > 0)
    }

    pub fn remove_relation(&self, issue_id_1: i64, issue_id_2: i64) -> Result<bool> {
        let (a, b) = if issue_id_1 < issue_id_2 {
            (issue_id_1, issue_id_2)
        } else {
            (issue_id_2, issue_id_1)
        };
        let rows = self.conn.execute(
            "DELETE FROM relations WHERE issue_id_1 = ?1 AND issue_id_2 = ?2",
            params![a, b],
        )?;
        Ok(rows > 0)
    }

    pub fn update_parent(&self, id: i64, parent_id: Option<i64>) -> Result<bool> {
        let now = chrono::Utc::now().to_rfc3339();
        let rows = self.conn.execute(
            "UPDATE issues SET parent_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![parent_id, now, id],
        )?;
        Ok(rows > 0)
    }

    pub fn get_related_issues(&self, issue_id: i64) -> Result<Vec<Issue>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT i.id, i.title, i.description, i.status, i.priority, i.parent_id, i.created_at, i.updated_at, i.closed_at
            FROM issues i
            WHERE i.id IN (
                SELECT issue_id_2 FROM relations WHERE issue_id_1 = ?1
                UNION
                SELECT issue_id_1 FROM relations WHERE issue_id_2 = ?1
            )
            ORDER BY i.id
            "#,
        )?;

        let issues = stmt
            .query_map([issue_id], |row| {
                Ok(Issue {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    description: row.get(2)?,
                    status: row.get(3)?,
                    priority: row.get(4)?,
                    parent_id: row.get(5)?,
                    created_at: parse_datetime(row.get::<_, String>(6)?),
                    updated_at: parse_datetime(row.get::<_, String>(7)?),
                    closed_at: row.get::<_, Option<String>>(8)?.map(parse_datetime),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(issues)
    }

    // Milestones
    pub fn create_milestone(&self, name: &str, description: Option<&str>) -> Result<i64> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO milestones (name, description, status, created_at) VALUES (?1, ?2, 'open', ?3)",
            params![name, description, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_milestone(&self, id: i64) -> Result<Option<crate::models::Milestone>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, description, status, created_at, closed_at FROM milestones WHERE id = ?1",
        )?;

        let milestone = stmt
            .query_row([id], |row| {
                Ok(crate::models::Milestone {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    status: row.get(3)?,
                    created_at: parse_datetime(row.get::<_, String>(4)?),
                    closed_at: row.get::<_, Option<String>>(5)?.map(parse_datetime),
                })
            })
            .ok();

        Ok(milestone)
    }

    pub fn list_milestones(&self, status: Option<&str>) -> Result<Vec<crate::models::Milestone>> {
        let sql = if let Some(s) = status {
            if s == "all" {
                "SELECT id, name, description, status, created_at, closed_at FROM milestones ORDER BY id DESC".to_string()
            } else {
                format!("SELECT id, name, description, status, created_at, closed_at FROM milestones WHERE status = '{}' ORDER BY id DESC", s)
            }
        } else {
            "SELECT id, name, description, status, created_at, closed_at FROM milestones WHERE status = 'open' ORDER BY id DESC".to_string()
        };

        let mut stmt = self.conn.prepare(&sql)?;
        let milestones = stmt
            .query_map([], |row| {
                Ok(crate::models::Milestone {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    status: row.get(3)?,
                    created_at: parse_datetime(row.get::<_, String>(4)?),
                    closed_at: row.get::<_, Option<String>>(5)?.map(parse_datetime),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(milestones)
    }

    pub fn add_issue_to_milestone(&self, milestone_id: i64, issue_id: i64) -> Result<bool> {
        let result = self.conn.execute(
            "INSERT OR IGNORE INTO milestone_issues (milestone_id, issue_id) VALUES (?1, ?2)",
            params![milestone_id, issue_id],
        )?;
        Ok(result > 0)
    }

    pub fn remove_issue_from_milestone(&self, milestone_id: i64, issue_id: i64) -> Result<bool> {
        let rows = self.conn.execute(
            "DELETE FROM milestone_issues WHERE milestone_id = ?1 AND issue_id = ?2",
            params![milestone_id, issue_id],
        )?;
        Ok(rows > 0)
    }

    pub fn get_milestone_issues(&self, milestone_id: i64) -> Result<Vec<Issue>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT i.id, i.title, i.description, i.status, i.priority, i.parent_id, i.created_at, i.updated_at, i.closed_at
            FROM issues i
            JOIN milestone_issues mi ON i.id = mi.issue_id
            WHERE mi.milestone_id = ?1
            ORDER BY i.id
            "#,
        )?;

        let issues = stmt
            .query_map([milestone_id], |row| {
                Ok(Issue {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    description: row.get(2)?,
                    status: row.get(3)?,
                    priority: row.get(4)?,
                    parent_id: row.get(5)?,
                    created_at: parse_datetime(row.get::<_, String>(6)?),
                    updated_at: parse_datetime(row.get::<_, String>(7)?),
                    closed_at: row.get::<_, Option<String>>(8)?.map(parse_datetime),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(issues)
    }

    pub fn close_milestone(&self, id: i64) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let rows = self.conn.execute(
            "UPDATE milestones SET status = 'closed', closed_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        Ok(rows > 0)
    }

    pub fn delete_milestone(&self, id: i64) -> Result<bool> {
        let rows = self
            .conn
            .execute("DELETE FROM milestones WHERE id = ?1", [id])?;
        Ok(rows > 0)
    }

    pub fn get_issue_milestone(&self, issue_id: i64) -> Result<Option<crate::models::Milestone>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT m.id, m.name, m.description, m.status, m.created_at, m.closed_at
            FROM milestones m
            JOIN milestone_issues mi ON m.id = mi.milestone_id
            WHERE mi.issue_id = ?1
            LIMIT 1
            "#,
        )?;

        let milestone = stmt
            .query_row([issue_id], |row| {
                Ok(crate::models::Milestone {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    status: row.get(3)?,
                    created_at: parse_datetime(row.get::<_, String>(4)?),
                    closed_at: row.get::<_, Option<String>>(5)?.map(parse_datetime),
                })
            })
            .ok();

        Ok(milestone)
    }

    // Archiving
    pub fn archive_issue(&self, id: i64) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let rows = self.conn.execute(
            "UPDATE issues SET status = 'archived', updated_at = ?1 WHERE id = ?2 AND status = 'closed'",
            params![now, id],
        )?;
        Ok(rows > 0)
    }

    pub fn unarchive_issue(&self, id: i64) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let rows = self.conn.execute(
            "UPDATE issues SET status = 'closed', updated_at = ?1 WHERE id = ?2 AND status = 'archived'",
            params![now, id],
        )?;
        Ok(rows > 0)
    }

    pub fn list_archived_issues(&self) -> Result<Vec<Issue>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, description, status, priority, parent_id, created_at, updated_at, closed_at FROM issues WHERE status = 'archived' ORDER BY id DESC",
        )?;

        let issues = stmt
            .query_map([], |row| {
                Ok(Issue {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    description: row.get(2)?,
                    status: row.get(3)?,
                    priority: row.get(4)?,
                    parent_id: row.get(5)?,
                    created_at: parse_datetime(row.get::<_, String>(6)?),
                    updated_at: parse_datetime(row.get::<_, String>(7)?),
                    closed_at: row.get::<_, Option<String>>(8)?.map(parse_datetime),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(issues)
    }

    pub fn archive_older_than(&self, days: i64) -> Result<i32> {
        let cutoff = Utc::now() - chrono::Duration::days(days);
        let cutoff_str = cutoff.to_rfc3339();
        let now = Utc::now().to_rfc3339();

        let rows = self.conn.execute(
            "UPDATE issues SET status = 'archived', updated_at = ?1 WHERE status = 'closed' AND closed_at < ?2",
            params![now, cutoff_str],
        )?;

        Ok(rows as i32)
    }
}

fn parse_datetime(s: String) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup_test_db() -> (Database, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).unwrap();
        (db, dir)
    }

    // ==================== Issue CRUD Tests ====================

    #[test]
    fn test_create_and_get_issue() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("Test issue", None, "medium").unwrap();
        assert!(id > 0);

        let issue = db.get_issue(id).unwrap().unwrap();
        assert_eq!(issue.id, id);
        assert_eq!(issue.title, "Test issue");
        assert_eq!(issue.description, None);
        assert_eq!(issue.status, "open");
        assert_eq!(issue.priority, "medium");
        assert_eq!(issue.parent_id, None);
        assert!(issue.closed_at.is_none());
    }

    #[test]
    fn test_create_issue_with_description() {
        let (db, _dir) = setup_test_db();

        let id = db
            .create_issue("Test issue", Some("Detailed description"), "high")
            .unwrap();
        let issue = db.get_issue(id).unwrap().unwrap();

        assert_eq!(issue.title, "Test issue");
        assert_eq!(issue.description, Some("Detailed description".to_string()));
        assert_eq!(issue.priority, "high");
    }

    #[test]
    fn test_create_subissue() {
        let (db, _dir) = setup_test_db();

        let parent_id = db.create_issue("Parent issue", None, "high").unwrap();
        let child_id = db
            .create_subissue(parent_id, "Child issue", None, "medium")
            .unwrap();

        let child = db.get_issue(child_id).unwrap().unwrap();
        assert_eq!(child.parent_id, Some(parent_id));

        let subissues = db.get_subissues(parent_id).unwrap();
        assert_eq!(subissues.len(), 1);
        assert_eq!(subissues[0].id, child_id);
    }

    #[test]
    fn test_get_nonexistent_issue() {
        let (db, _dir) = setup_test_db();
        let issue = db.get_issue(99999).unwrap();
        assert!(issue.is_none());
    }

    #[test]
    fn test_list_issues() {
        let (db, _dir) = setup_test_db();

        db.create_issue("Issue 1", None, "low").unwrap();
        db.create_issue("Issue 2", None, "medium").unwrap();
        db.create_issue("Issue 3", None, "high").unwrap();

        let issues = db.list_issues(None, None, None).unwrap();
        assert_eq!(issues.len(), 3);
    }

    #[test]
    fn test_list_issues_filter_by_status() {
        let (db, _dir) = setup_test_db();

        let id1 = db.create_issue("Open issue", None, "low").unwrap();
        let id2 = db.create_issue("To be closed", None, "medium").unwrap();
        db.close_issue(id2).unwrap();

        let open_issues = db.list_issues(Some("open"), None, None).unwrap();
        assert_eq!(open_issues.len(), 1);
        assert_eq!(open_issues[0].id, id1);

        let closed_issues = db.list_issues(Some("closed"), None, None).unwrap();
        assert_eq!(closed_issues.len(), 1);
        assert_eq!(closed_issues[0].id, id2);

        let all_issues = db.list_issues(Some("all"), None, None).unwrap();
        assert_eq!(all_issues.len(), 2);
    }

    #[test]
    fn test_list_issues_filter_by_priority() {
        let (db, _dir) = setup_test_db();

        db.create_issue("Low priority", None, "low").unwrap();
        db.create_issue("High priority", None, "high").unwrap();

        let high_issues = db.list_issues(None, None, Some("high")).unwrap();
        assert_eq!(high_issues.len(), 1);
        assert_eq!(high_issues[0].priority, "high");
    }

    #[test]
    fn test_update_issue() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("Original title", None, "low").unwrap();

        let updated = db
            .update_issue(
                id,
                Some("Updated title"),
                Some("New description"),
                Some("critical"),
            )
            .unwrap();
        assert!(updated);

        let issue = db.get_issue(id).unwrap().unwrap();
        assert_eq!(issue.title, "Updated title");
        assert_eq!(issue.description, Some("New description".to_string()));
        assert_eq!(issue.priority, "critical");
    }

    #[test]
    fn test_update_issue_partial() {
        let (db, _dir) = setup_test_db();

        let id = db
            .create_issue("Original title", Some("Original desc"), "low")
            .unwrap();

        db.update_issue(id, Some("New title"), None, None).unwrap();

        let issue = db.get_issue(id).unwrap().unwrap();
        assert_eq!(issue.title, "New title");
        assert_eq!(issue.description, Some("Original desc".to_string()));
        assert_eq!(issue.priority, "low");
    }

    #[test]
    fn test_close_and_reopen_issue() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("Test issue", None, "medium").unwrap();

        let closed = db.close_issue(id).unwrap();
        assert!(closed);

        let issue = db.get_issue(id).unwrap().unwrap();
        assert_eq!(issue.status, "closed");
        assert!(issue.closed_at.is_some());

        let reopened = db.reopen_issue(id).unwrap();
        assert!(reopened);

        let issue = db.get_issue(id).unwrap().unwrap();
        assert_eq!(issue.status, "open");
        assert!(issue.closed_at.is_none());
    }

    #[test]
    fn test_delete_issue() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("To delete", None, "low").unwrap();
        assert!(db.get_issue(id).unwrap().is_some());

        let deleted = db.delete_issue(id).unwrap();
        assert!(deleted);
        assert!(db.get_issue(id).unwrap().is_none());
    }

    #[test]
    fn test_delete_nonexistent_issue() {
        let (db, _dir) = setup_test_db();
        let deleted = db.delete_issue(99999).unwrap();
        assert!(!deleted);
    }

    // ==================== Labels Tests ====================

    #[test]
    fn test_add_and_get_labels() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("Test issue", None, "medium").unwrap();

        db.add_label(id, "bug").unwrap();
        db.add_label(id, "urgent").unwrap();

        let labels = db.get_labels(id).unwrap();
        assert_eq!(labels.len(), 2);
        assert!(labels.contains(&"bug".to_string()));
        assert!(labels.contains(&"urgent".to_string()));
    }

    #[test]
    fn test_add_duplicate_label() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("Test issue", None, "medium").unwrap();

        db.add_label(id, "bug").unwrap();
        db.add_label(id, "bug").unwrap(); // Should be ignored

        let labels = db.get_labels(id).unwrap();
        assert_eq!(labels.len(), 1);
    }

    #[test]
    fn test_remove_label() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("Test issue", None, "medium").unwrap();

        db.add_label(id, "bug").unwrap();
        db.add_label(id, "urgent").unwrap();

        let removed = db.remove_label(id, "bug").unwrap();
        assert!(removed);

        let labels = db.get_labels(id).unwrap();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0], "urgent");
    }

    #[test]
    fn test_list_issues_filter_by_label() {
        let (db, _dir) = setup_test_db();

        let id1 = db.create_issue("Bug issue", None, "high").unwrap();
        let id2 = db.create_issue("Feature issue", None, "medium").unwrap();

        db.add_label(id1, "bug").unwrap();
        db.add_label(id2, "feature").unwrap();

        let bug_issues = db.list_issues(None, Some("bug"), None).unwrap();
        assert_eq!(bug_issues.len(), 1);
        assert_eq!(bug_issues[0].id, id1);
    }

    // ==================== Comments Tests ====================

    #[test]
    fn test_add_and_get_comments() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("Test issue", None, "medium").unwrap();

        let comment_id = db.add_comment(id, "First comment").unwrap();
        assert!(comment_id > 0);

        db.add_comment(id, "Second comment").unwrap();

        let comments = db.get_comments(id).unwrap();
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].content, "First comment");
        assert_eq!(comments[1].content, "Second comment");
    }

    // ==================== Dependencies Tests ====================

    #[test]
    fn test_add_and_get_dependencies() {
        let (db, _dir) = setup_test_db();

        let blocker = db.create_issue("Blocker issue", None, "high").unwrap();
        let blocked = db.create_issue("Blocked issue", None, "medium").unwrap();

        db.add_dependency(blocked, blocker).unwrap();

        let blockers = db.get_blockers(blocked).unwrap();
        assert_eq!(blockers.len(), 1);
        assert_eq!(blockers[0], blocker);

        let blocking = db.get_blocking(blocker).unwrap();
        assert_eq!(blocking.len(), 1);
        assert_eq!(blocking[0], blocked);
    }

    #[test]
    fn test_remove_dependency() {
        let (db, _dir) = setup_test_db();

        let blocker = db.create_issue("Blocker", None, "high").unwrap();
        let blocked = db.create_issue("Blocked", None, "medium").unwrap();

        db.add_dependency(blocked, blocker).unwrap();
        let removed = db.remove_dependency(blocked, blocker).unwrap();
        assert!(removed);

        let blockers = db.get_blockers(blocked).unwrap();
        assert!(blockers.is_empty());
    }

    #[test]
    fn test_list_blocked_issues() {
        let (db, _dir) = setup_test_db();

        let blocker = db.create_issue("Blocker", None, "high").unwrap();
        let blocked = db.create_issue("Blocked", None, "medium").unwrap();
        let unblocked = db.create_issue("Unblocked", None, "low").unwrap();

        db.add_dependency(blocked, blocker).unwrap();

        let blocked_issues = db.list_blocked_issues().unwrap();
        assert_eq!(blocked_issues.len(), 1);
        assert_eq!(blocked_issues[0].id, blocked);

        // Unblocked issue should not appear
        assert!(!blocked_issues.iter().any(|i| i.id == unblocked));
    }

    #[test]
    fn test_list_ready_issues() {
        let (db, _dir) = setup_test_db();

        let blocker = db.create_issue("Blocker", None, "high").unwrap();
        let blocked = db.create_issue("Blocked", None, "medium").unwrap();
        let ready = db.create_issue("Ready", None, "low").unwrap();

        db.add_dependency(blocked, blocker).unwrap();

        let ready_issues = db.list_ready_issues().unwrap();

        // Blocker and ready should be in ready list (not blocked by anything)
        let ready_ids: Vec<i64> = ready_issues.iter().map(|i| i.id).collect();
        assert!(ready_ids.contains(&blocker));
        assert!(ready_ids.contains(&ready));
        assert!(!ready_ids.contains(&blocked));
    }

    #[test]
    fn test_blocked_becomes_ready_when_blocker_closed() {
        let (db, _dir) = setup_test_db();

        let blocker = db.create_issue("Blocker", None, "high").unwrap();
        let blocked = db.create_issue("Blocked", None, "medium").unwrap();

        db.add_dependency(blocked, blocker).unwrap();

        // Initially blocked
        let blocked_issues = db.list_blocked_issues().unwrap();
        assert_eq!(blocked_issues.len(), 1);

        // Close blocker
        db.close_issue(blocker).unwrap();

        // Now should be ready
        let blocked_issues = db.list_blocked_issues().unwrap();
        assert!(blocked_issues.is_empty());

        let ready_issues = db.list_ready_issues().unwrap();
        assert!(ready_issues.iter().any(|i| i.id == blocked));
    }

    // ==================== Sessions Tests ====================

    #[test]
    fn test_start_and_get_session() {
        let (db, _dir) = setup_test_db();

        let id = db.start_session().unwrap();
        assert!(id > 0);

        let session = db.get_current_session().unwrap().unwrap();
        assert_eq!(session.id, id);
        assert!(session.ended_at.is_none());
        assert!(session.active_issue_id.is_none());
    }

    #[test]
    fn test_end_session() {
        let (db, _dir) = setup_test_db();

        let id = db.start_session().unwrap();
        db.end_session(id, Some("Handoff notes")).unwrap();

        let current = db.get_current_session().unwrap();
        assert!(current.is_none());

        let last = db.get_last_session().unwrap().unwrap();
        assert_eq!(last.id, id);
        assert!(last.ended_at.is_some());
        assert_eq!(last.handoff_notes, Some("Handoff notes".to_string()));
    }

    #[test]
    fn test_set_session_issue() {
        let (db, _dir) = setup_test_db();

        let issue_id = db.create_issue("Test issue", None, "medium").unwrap();
        let session_id = db.start_session().unwrap();

        db.set_session_issue(session_id, issue_id).unwrap();

        let session = db.get_current_session().unwrap().unwrap();
        assert_eq!(session.active_issue_id, Some(issue_id));
    }

    // ==================== Time Tracking Tests ====================

    #[test]
    fn test_start_and_stop_timer() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("Test issue", None, "medium").unwrap();

        let timer_id = db.start_timer(id).unwrap();
        assert!(timer_id > 0);

        let active = db.get_active_timer().unwrap();
        assert!(active.is_some());
        assert_eq!(active.unwrap().0, id);

        std::thread::sleep(std::time::Duration::from_millis(100));

        db.stop_timer(id).unwrap();

        let active = db.get_active_timer().unwrap();
        assert!(active.is_none());
    }

    #[test]
    fn test_get_total_time() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("Test issue", None, "medium").unwrap();

        // No time tracked yet
        let total = db.get_total_time(id).unwrap();
        assert_eq!(total, 0);
    }

    // ==================== Search Tests ====================

    #[test]
    fn test_search_issues_by_title() {
        let (db, _dir) = setup_test_db();

        db.create_issue("Fix authentication bug", None, "high")
            .unwrap();
        db.create_issue("Add dark mode", None, "medium").unwrap();
        db.create_issue("Auth improvements", None, "low").unwrap();

        let results = db.search_issues("auth").unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_search_issues_by_description() {
        let (db, _dir) = setup_test_db();

        db.create_issue(
            "Feature A",
            Some("This relates to authentication"),
            "medium",
        )
        .unwrap();
        db.create_issue("Feature B", Some("Something else"), "medium")
            .unwrap();

        let results = db.search_issues("authentication").unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_issues_by_comment() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("Some issue", None, "medium").unwrap();
        db.add_comment(id, "Found the root cause in authentication module")
            .unwrap();

        let results = db.search_issues("authentication").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, id);
    }

    // ==================== Relations Tests ====================

    #[test]
    fn test_add_and_get_relations() {
        let (db, _dir) = setup_test_db();

        let id1 = db.create_issue("Issue 1", None, "medium").unwrap();
        let id2 = db.create_issue("Issue 2", None, "medium").unwrap();

        db.add_relation(id1, id2).unwrap();

        let related = db.get_related_issues(id1).unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].id, id2);

        // Bidirectional
        let related = db.get_related_issues(id2).unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].id, id1);
    }

    #[test]
    fn test_relation_to_self_fails() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("Issue", None, "medium").unwrap();

        let result = db.add_relation(id, id);
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_relation() {
        let (db, _dir) = setup_test_db();

        let id1 = db.create_issue("Issue 1", None, "medium").unwrap();
        let id2 = db.create_issue("Issue 2", None, "medium").unwrap();

        db.add_relation(id1, id2).unwrap();
        db.remove_relation(id1, id2).unwrap();

        let related = db.get_related_issues(id1).unwrap();
        assert!(related.is_empty());
    }

    // ==================== Milestones Tests ====================

    #[test]
    fn test_create_and_get_milestone() {
        let (db, _dir) = setup_test_db();

        let id = db.create_milestone("v1.0", Some("First release")).unwrap();
        assert!(id > 0);

        let milestone = db.get_milestone(id).unwrap().unwrap();
        assert_eq!(milestone.name, "v1.0");
        assert_eq!(milestone.description, Some("First release".to_string()));
        assert_eq!(milestone.status, "open");
    }

    #[test]
    fn test_list_milestones() {
        let (db, _dir) = setup_test_db();

        db.create_milestone("v1.0", None).unwrap();
        db.create_milestone("v2.0", None).unwrap();

        let milestones = db.list_milestones(None).unwrap();
        assert_eq!(milestones.len(), 2);
    }

    #[test]
    fn test_add_issue_to_milestone() {
        let (db, _dir) = setup_test_db();

        let milestone_id = db.create_milestone("v1.0", None).unwrap();
        let issue_id = db.create_issue("Feature", None, "medium").unwrap();

        db.add_issue_to_milestone(milestone_id, issue_id).unwrap();

        let issues = db.get_milestone_issues(milestone_id).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, issue_id);

        let milestone = db.get_issue_milestone(issue_id).unwrap().unwrap();
        assert_eq!(milestone.id, milestone_id);
    }

    #[test]
    fn test_close_milestone() {
        let (db, _dir) = setup_test_db();

        let id = db.create_milestone("v1.0", None).unwrap();
        db.close_milestone(id).unwrap();

        let milestone = db.get_milestone(id).unwrap().unwrap();
        assert_eq!(milestone.status, "closed");
        assert!(milestone.closed_at.is_some());
    }

    // ==================== Archive Tests ====================

    #[test]
    fn test_archive_closed_issue() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("Test", None, "medium").unwrap();
        db.close_issue(id).unwrap();

        let archived = db.archive_issue(id).unwrap();
        assert!(archived);

        let issue = db.get_issue(id).unwrap().unwrap();
        assert_eq!(issue.status, "archived");
    }

    #[test]
    fn test_archive_open_issue_fails() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("Test", None, "medium").unwrap();

        let archived = db.archive_issue(id).unwrap();
        assert!(!archived);

        let issue = db.get_issue(id).unwrap().unwrap();
        assert_eq!(issue.status, "open");
    }

    #[test]
    fn test_unarchive_issue() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("Test", None, "medium").unwrap();
        db.close_issue(id).unwrap();
        db.archive_issue(id).unwrap();

        let unarchived = db.unarchive_issue(id).unwrap();
        assert!(unarchived);

        let issue = db.get_issue(id).unwrap().unwrap();
        assert_eq!(issue.status, "closed");
    }

    #[test]
    fn test_list_archived_issues() {
        let (db, _dir) = setup_test_db();

        let id1 = db.create_issue("Archived", None, "medium").unwrap();
        let _id2 = db.create_issue("Open", None, "medium").unwrap();

        db.close_issue(id1).unwrap();
        db.archive_issue(id1).unwrap();

        let archived = db.list_archived_issues().unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].id, id1);
    }

    // ==================== Security Tests ====================

    #[test]
    fn test_sql_injection_in_title() {
        let (db, _dir) = setup_test_db();

        // Attempt SQL injection via title
        let malicious = "'; DROP TABLE issues; --";
        let id = db.create_issue(malicious, None, "medium").unwrap();

        // Should have created issue with literal string, not executed SQL
        let issue = db.get_issue(id).unwrap().unwrap();
        assert_eq!(issue.title, malicious);

        // Database should still be intact
        let issues = db.list_issues(None, None, None).unwrap();
        assert!(!issues.is_empty());
    }

    #[test]
    fn test_sql_injection_in_description() {
        let (db, _dir) = setup_test_db();

        let malicious = "test'); DELETE FROM issues; --";
        let id = db
            .create_issue("Normal title", Some(malicious), "medium")
            .unwrap();

        let issue = db.get_issue(id).unwrap().unwrap();
        assert_eq!(issue.description, Some(malicious.to_string()));
    }

    #[test]
    fn test_sql_injection_in_label() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("Test", None, "medium").unwrap();
        let malicious = "bug'; DROP TABLE labels; --";

        db.add_label(id, malicious).unwrap();

        let labels = db.get_labels(id).unwrap();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0], malicious);
    }

    #[test]
    fn test_sql_injection_in_search() {
        let (db, _dir) = setup_test_db();

        db.create_issue("Normal issue", None, "medium").unwrap();

        // Attempt injection in search
        let malicious = "%'; DROP TABLE issues; --";
        let results = db.search_issues(malicious).unwrap();

        // Should return empty results, not crash
        assert!(results.is_empty());

        // Database should still be intact
        let issues = db.list_issues(None, None, None).unwrap();
        assert!(!issues.is_empty());
    }

    #[test]
    fn test_sql_injection_in_comment() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("Test", None, "medium").unwrap();
        let malicious = "comment'); DELETE FROM comments; --";

        db.add_comment(id, malicious).unwrap();

        let comments = db.get_comments(id).unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].content, malicious);
    }

    #[test]
    fn test_unicode_in_fields() {
        let (db, _dir) = setup_test_db();

        let title = "测试问题 🐛 αβγ";
        let description = "Description with émojis 🎉 and ñ";

        let id = db.create_issue(title, Some(description), "medium").unwrap();

        let issue = db.get_issue(id).unwrap().unwrap();
        assert_eq!(issue.title, title);
        assert_eq!(issue.description, Some(description.to_string()));
    }

    #[test]
    fn test_very_long_strings() {
        let (db, _dir) = setup_test_db();

        let long_title = "a".repeat(10000);
        let long_desc = "b".repeat(100000);

        let id = db
            .create_issue(&long_title, Some(&long_desc), "medium")
            .unwrap();

        let issue = db.get_issue(id).unwrap().unwrap();
        assert_eq!(issue.title.len(), 10000);
        assert_eq!(issue.description.unwrap().len(), 100000);
    }

    #[test]
    fn test_null_bytes_in_strings() {
        let (db, _dir) = setup_test_db();

        let title = "test\0null\0bytes";
        let id = db.create_issue(title, None, "medium").unwrap();

        let issue = db.get_issue(id).unwrap().unwrap();
        assert_eq!(issue.title, title);
    }

    // ==================== Cascade Delete Tests ====================

    #[test]
    fn test_delete_issue_cascades_labels() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("Test", None, "medium").unwrap();
        db.add_label(id, "bug").unwrap();
        db.add_label(id, "urgent").unwrap();

        db.delete_issue(id).unwrap();

        // Labels should be gone (via CASCADE)
        let labels = db.get_labels(id).unwrap();
        assert!(labels.is_empty());
    }

    #[test]
    fn test_delete_issue_cascades_comments() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("Test", None, "medium").unwrap();
        db.add_comment(id, "Comment 1").unwrap();
        db.add_comment(id, "Comment 2").unwrap();

        db.delete_issue(id).unwrap();

        let comments = db.get_comments(id).unwrap();
        assert!(comments.is_empty());
    }

    #[test]
    fn test_delete_parent_cascades_subissues() {
        let (db, _dir) = setup_test_db();

        let parent_id = db.create_issue("Parent", None, "high").unwrap();
        let child_id = db
            .create_subissue(parent_id, "Child", None, "medium")
            .unwrap();

        db.delete_issue(parent_id).unwrap();

        // Child should be deleted too
        assert!(db.get_issue(child_id).unwrap().is_none());
    }

    // ==================== Edge Cases ====================

    #[test]
    fn test_empty_title() {
        let (db, _dir) = setup_test_db();

        let id = db.create_issue("", None, "medium").unwrap();
        let issue = db.get_issue(id).unwrap().unwrap();
        assert_eq!(issue.title, "");
    }

    #[test]
    fn test_update_parent() {
        let (db, _dir) = setup_test_db();

        let parent1 = db.create_issue("Parent 1", None, "high").unwrap();
        let parent2 = db.create_issue("Parent 2", None, "high").unwrap();
        let child = db.create_issue("Child", None, "medium").unwrap();

        db.update_parent(child, Some(parent1)).unwrap();
        let issue = db.get_issue(child).unwrap().unwrap();
        assert_eq!(issue.parent_id, Some(parent1));

        db.update_parent(child, Some(parent2)).unwrap();
        let issue = db.get_issue(child).unwrap().unwrap();
        assert_eq!(issue.parent_id, Some(parent2));

        db.update_parent(child, None).unwrap();
        let issue = db.get_issue(child).unwrap().unwrap();
        assert_eq!(issue.parent_id, None);
    }
}
