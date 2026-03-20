use anyhow::Result;

use crate::db::Database;
use crate::utils::format_issue_id;

pub fn record(
    db: &Database,
    agent_id: &str,
    model: &str,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: Option<i64>,
    cache_creation_tokens: Option<i64>,
    session_id: Option<i64>,
) -> Result<()> {
    let cost = crate::token_usage::estimate_cost(
        model,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
    );

    let id = db.create_token_usage(
        agent_id,
        session_id,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        model,
        cost,
    )?;

    println!("Recorded usage #{}", id);
    if let Some(c) = cost {
        println!("  Estimated cost: ${:.4}", c);
    }
    Ok(())
}

pub fn list(
    db: &Database,
    agent_id: Option<&str>,
    model: Option<&str>,
    limit: Option<i64>,
    json: bool,
) -> Result<()> {
    let entries = db.list_token_usage(agent_id, None, model, None, None, limit)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    if entries.is_empty() {
        println!("No usage records found.");
        return Ok(());
    }

    println!(
        "{:<5} {:<12} {:<10} {:<22} {:>10} {:>10} {:>8}",
        "ID", "Agent", "Model", "Timestamp", "Input", "Output", "Cost"
    );
    println!("{}", "-".repeat(80));

    for entry in &entries {
        let model_short = if entry.model.len() > 10 {
            &entry.model[..10]
        } else {
            &entry.model
        };
        let ts = entry.timestamp.format("%Y-%m-%d %H:%M:%S");
        let cost_str = entry
            .cost_estimate
            .map(|c| format!("${:.4}", c))
            .unwrap_or_else(|| "-".to_string());

        println!(
            "{:<5} {:<12} {:<10} {:<22} {:>10} {:>10} {:>8}",
            format_issue_id(entry.id),
            truncate(&entry.agent_id, 12),
            model_short,
            ts,
            entry.input_tokens,
            entry.output_tokens,
            cost_str,
        );
    }

    Ok(())
}

pub fn summary(db: &Database, agent_id: Option<&str>, json: bool) -> Result<()> {
    let rows = db.get_usage_summary(agent_id, None, None)?;

    if json {
        let total_input: i64 = rows.iter().map(|r| r.total_input_tokens).sum();
        let total_output: i64 = rows.iter().map(|r| r.total_output_tokens).sum();
        let total_cost: f64 = rows.iter().map(|r| r.total_cost).sum();

        let response = serde_json::json!({
            "items": rows,
            "total_input_tokens": total_input,
            "total_output_tokens": total_output,
            "total_cost": total_cost,
        });
        println!("{}", serde_json::to_string_pretty(&response)?);
        return Ok(());
    }

    if rows.is_empty() {
        println!("No usage records found.");
        return Ok(());
    }

    println!(
        "{:<12} {:<16} {:>6} {:>12} {:>12} {:>10}",
        "Agent", "Model", "Reqs", "Input Tok", "Output Tok", "Cost"
    );
    println!("{}", "-".repeat(72));

    let mut total_cost = 0.0;
    for row in &rows {
        let model_short = if row.model.len() > 16 {
            &row.model[..16]
        } else {
            &row.model
        };
        println!(
            "{:<12} {:<16} {:>6} {:>12} {:>12} {:>10}",
            truncate(&row.agent_id, 12),
            model_short,
            row.request_count,
            row.total_input_tokens,
            row.total_output_tokens,
            format!("${:.4}", row.total_cost),
        );
        total_cost += row.total_cost;
    }

    println!("{}", "-".repeat(72));
    println!("{:>68} ${:.4}", "Total:", total_cost);

    Ok(())
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() > max {
        &s[..max]
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup_test_db() -> (Database, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let db = Database::open(&dir.path().join("test.db")).unwrap();
        (db, dir)
    }

    #[test]
    fn test_record_and_list() {
        let (db, _dir) = setup_test_db();

        record(&db, "worker-1", "claude-sonnet-4-6", 1000, 500, None, None, None).unwrap();

        let entries = db.list_token_usage(None, None, None, None, None, None).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].agent_id, "worker-1");
        assert_eq!(entries[0].input_tokens, 1000);
        assert_eq!(entries[0].output_tokens, 500);
        assert!(entries[0].cost_estimate.is_some());
    }

    #[test]
    fn test_summary_aggregation() {
        let (db, _dir) = setup_test_db();

        record(&db, "worker-1", "claude-opus-4-6", 1000, 500, None, None, None).unwrap();
        record(&db, "worker-1", "claude-opus-4-6", 2000, 1000, None, None, None).unwrap();
        record(&db, "worker-2", "claude-sonnet-4-6", 500, 200, None, None, None).unwrap();

        let rows = db.get_usage_summary(None, None, None).unwrap();
        assert_eq!(rows.len(), 2); // Two agent+model groups

        let opus_row = rows.iter().find(|r| r.model.contains("opus")).unwrap();
        assert_eq!(opus_row.request_count, 2);
        assert_eq!(opus_row.total_input_tokens, 3000);
        assert_eq!(opus_row.total_output_tokens, 1500);
    }

    #[test]
    fn test_list_filter_by_agent() {
        let (db, _dir) = setup_test_db();

        record(&db, "worker-1", "claude-opus-4-6", 1000, 500, None, None, None).unwrap();
        record(&db, "worker-2", "claude-opus-4-6", 2000, 1000, None, None, None).unwrap();

        let entries = db.list_token_usage(Some("worker-1"), None, None, None, None, None).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].agent_id, "worker-1");
    }
}
