use anyhow::Result;
use chrono::Utc;
use rusqlite::params;

use super::{parse_datetime, Database};
use crate::models::TokenUsage;
use crate::token_usage::{ParsedUsage, UsageSummaryRow};

impl Database {
    pub fn create_token_usage(&self, usage: &ParsedUsage) -> Result<i64> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO token_usage (agent_id, session_id, timestamp, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, model, cost_estimate)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                usage.agent_id,
                usage.session_id,
                now,
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_creation_tokens,
                usage.model,
                usage.cost_estimate
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_token_usage(&self, id: i64) -> Result<Option<TokenUsage>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, agent_id, session_id, timestamp, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, model, cost_estimate
             FROM token_usage WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map([id], |row| {
            Ok(TokenUsage {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                session_id: row.get(2)?,
                timestamp: parse_datetime(row.get::<_, String>(3)?),
                input_tokens: row.get(4)?,
                output_tokens: row.get(5)?,
                cache_read_tokens: row.get(6)?,
                cache_creation_tokens: row.get(7)?,
                model: row.get(8)?,
                cost_estimate: row.get(9)?,
            })
        })?;
        match rows.next() {
            Some(Ok(usage)) => Ok(Some(usage)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    pub fn list_token_usage(
        &self,
        agent_id: Option<&str>,
        session_id: Option<i64>,
        model: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
        limit: Option<i64>,
    ) -> Result<Vec<TokenUsage>> {
        let mut conditions = Vec::new();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(aid) = agent_id {
            conditions.push(format!("agent_id = ?{}", param_values.len() + 1));
            param_values.push(Box::new(aid.to_string()));
        }
        if let Some(sid) = session_id {
            conditions.push(format!("session_id = ?{}", param_values.len() + 1));
            param_values.push(Box::new(sid));
        }
        if let Some(m) = model {
            conditions.push(format!("model = ?{}", param_values.len() + 1));
            param_values.push(Box::new(m.to_string()));
        }
        if let Some(f) = from {
            conditions.push(format!("timestamp >= ?{}", param_values.len() + 1));
            param_values.push(Box::new(f.to_string()));
        }
        if let Some(t) = to {
            conditions.push(format!("timestamp <= ?{}", param_values.len() + 1));
            param_values.push(Box::new(t.to_string()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let limit_clause = match limit {
            Some(l) => format!(" LIMIT {}", l),
            None => String::new(),
        };

        let sql = format!(
            "SELECT id, agent_id, session_id, timestamp, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, model, cost_estimate
             FROM token_usage{} ORDER BY timestamp DESC{}",
            where_clause, limit_clause
        );

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let results = stmt
            .query_map(params_ref.as_slice(), |row| {
                Ok(TokenUsage {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    session_id: row.get(2)?,
                    timestamp: parse_datetime(row.get::<_, String>(3)?),
                    input_tokens: row.get(4)?,
                    output_tokens: row.get(5)?,
                    cache_read_tokens: row.get(6)?,
                    cache_creation_tokens: row.get(7)?,
                    model: row.get(8)?,
                    cost_estimate: row.get(9)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(results)
    }

    pub fn get_usage_summary(
        &self,
        agent_id: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<Vec<UsageSummaryRow>> {
        let mut conditions = Vec::new();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(aid) = agent_id {
            conditions.push(format!("agent_id = ?{}", param_values.len() + 1));
            param_values.push(Box::new(aid.to_string()));
        }
        if let Some(f) = from {
            conditions.push(format!("timestamp >= ?{}", param_values.len() + 1));
            param_values.push(Box::new(f.to_string()));
        }
        if let Some(t) = to {
            conditions.push(format!("timestamp <= ?{}", param_values.len() + 1));
            param_values.push(Box::new(t.to_string()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT agent_id, model, COUNT(*) as request_count,
                    SUM(input_tokens) as total_input,
                    SUM(output_tokens) as total_output,
                    COALESCE(SUM(cache_read_tokens), 0) as total_cache_read,
                    COALESCE(SUM(cache_creation_tokens), 0) as total_cache_creation,
                    COALESCE(SUM(cost_estimate), 0.0) as total_cost
             FROM token_usage{}
             GROUP BY agent_id, model
             ORDER BY total_cost DESC",
            where_clause
        );

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let results = stmt
            .query_map(params_ref.as_slice(), |row| {
                Ok(UsageSummaryRow {
                    agent_id: row.get(0)?,
                    model: row.get(1)?,
                    request_count: row.get(2)?,
                    total_input_tokens: row.get(3)?,
                    total_output_tokens: row.get(4)?,
                    total_cache_read_tokens: row.get(5)?,
                    total_cache_creation_tokens: row.get(6)?,
                    total_cost: row.get(7)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(results)
    }
}
