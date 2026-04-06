use anyhow::Result;
use chrono::Utc;
use rusqlite::params;

use super::{issue_from_row, Database};
use crate::models::{Issue, Relation};

impl Database {
    /// Add a typed relation between two issues.
    /// Defaults to "related" if no type is specified.
    pub fn add_typed_relation(
        &self,
        issue_id_1: i64,
        issue_id_2: i64,
        relation_type: &str,
    ) -> Result<bool> {
        if issue_id_1 == issue_id_2 {
            anyhow::bail!("Cannot relate an issue to itself");
        }
        let (a, b) = if issue_id_1 < issue_id_2 {
            (issue_id_1, issue_id_2)
        } else {
            (issue_id_2, issue_id_1)
        };
        let now = Utc::now().to_rfc3339();
        let result = self.conn.execute(
            "INSERT OR IGNORE INTO relations (issue_id_1, issue_id_2, relation_type, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![a, b, relation_type, now],
        )?;
        Ok(result > 0)
    }

    /// Backward-compatible: add a "related" relation.
    pub fn add_relation(&self, issue_id_1: i64, issue_id_2: i64) -> Result<bool> {
        self.add_typed_relation(issue_id_1, issue_id_2, "related")
    }

    /// Remove a typed relation between two issues.
    pub fn remove_typed_relation(
        &self,
        issue_id_1: i64,
        issue_id_2: i64,
        relation_type: &str,
    ) -> Result<bool> {
        let (a, b) = if issue_id_1 < issue_id_2 {
            (issue_id_1, issue_id_2)
        } else {
            (issue_id_2, issue_id_1)
        };
        let rows = self.conn.execute(
            "DELETE FROM relations WHERE issue_id_1 = ?1 AND issue_id_2 = ?2 AND relation_type = ?3",
            params![a, b, relation_type],
        )?;
        Ok(rows > 0)
    }

    /// Backward-compatible: remove a "related" relation.
    pub fn remove_relation(&self, issue_id_1: i64, issue_id_2: i64) -> Result<bool> {
        self.remove_typed_relation(issue_id_1, issue_id_2, "related")
    }

    /// Get all related issues (any relation type).
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
            .query_map([issue_id], issue_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(issues)
    }

    /// Get all relations for an issue with their types.
    pub fn get_typed_relations(&self, issue_id: i64) -> Result<Vec<Relation>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT issue_id_1, issue_id_2, relation_type, created_at
            FROM relations
            WHERE issue_id_1 = ?1 OR issue_id_2 = ?1
            ORDER BY created_at
            "#,
        )?;

        let relations = stmt
            .query_map([issue_id], |row| {
                Ok(Relation {
                    issue_id_1: row.get(0)?,
                    issue_id_2: row.get(1)?,
                    relation_type: row.get(2)?,
                    created_at: super::parse_datetime(row.get::<_, String>(3)?),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(relations)
    }

    /// Get issues related by a specific relation type.
    pub fn get_issues_by_relation_type(
        &self,
        issue_id: i64,
        relation_type: &str,
    ) -> Result<Vec<Issue>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT i.id, i.title, i.description, i.status, i.priority, i.parent_id, i.created_at, i.updated_at, i.closed_at
            FROM issues i
            WHERE i.id IN (
                SELECT issue_id_2 FROM relations WHERE issue_id_1 = ?1 AND relation_type = ?2
                UNION
                SELECT issue_id_1 FROM relations WHERE issue_id_2 = ?1 AND relation_type = ?2
            )
            ORDER BY i.id
            "#,
        )?;

        let issues = stmt
            .query_map(params![issue_id, relation_type], issue_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(issues)
    }

    /// Falsification cascade: given an issue marked as falsified, find ALL issues
    /// that transitively depend on it via parent-child, "derived", or "assumption" links.
    /// Returns the set of issue IDs that should be reassessed.
    pub fn falsification_cascade(&self, falsified_id: i64) -> Result<Vec<Issue>> {
        let mut visited = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        let mut affected = Vec::new();

        queue.push_back(falsified_id);
        visited.insert(falsified_id);

        while let Some(current_id) = queue.pop_front() {
            // 1. Children (parent_id = current) — downstream in the why-chain
            let children = self.get_subissues(current_id)?;
            for child in children {
                if visited.insert(child.id) {
                    affected.push(child.clone());
                    queue.push_back(child.id);
                }
            }

            // 2. Issues linked via "derived" relation — conclusions built on this assumption
            let derived = self.get_issues_by_relation_type(current_id, "derived")?;
            for issue in derived {
                if visited.insert(issue.id) {
                    affected.push(issue.clone());
                    queue.push_back(issue.id);
                }
            }

            // 3. Issues linked via "assumption" relation — shared assumptions
            // (only one hop — don't cascade through the entire assumption cluster)
            if current_id == falsified_id {
                let shared = self.get_issues_by_relation_type(current_id, "assumption")?;
                for issue in shared {
                    if visited.insert(issue.id) {
                        affected.push(issue.clone());
                        // Don't push to queue — one hop only for assumption links
                    }
                }
            }
        }

        Ok(affected)
    }
}
