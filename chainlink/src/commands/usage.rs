use anyhow::Result;

use crate::db::Database;
use crate::token_usage::ParsedUsage;
use crate::utils::format_issue_id;

pub fn record(db: &Database, usage: &ParsedUsage) -> Result<()> {
    let id = db.create_token_usage(usage)?;

    println!("Recorded usage #{}", id);
    if let Some(c) = usage.cost_estimate {
        println!("  Estimated cost: ${:.4}", c);
    }
    Ok(())
}

pub fn show(db: &Database, id: i64, json: bool) -> Result<()> {
    match db.get_token_usage(id)? {
        Some(entry) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&entry)?);
                return Ok(());
            }
            println!("Usage #{}", entry.id);
            println!("  Agent:    {}", entry.agent_id);
            println!("  Model:    {}", entry.model);
            println!(
                "  Time:     {}",
                entry.timestamp.format("%Y-%m-%d %H:%M:%S")
            );
            if let Some(sid) = entry.session_id {
                println!("  Session:  #{}", sid);
            }
            println!("  Input:    {} tokens", entry.input_tokens);
            println!("  Output:   {} tokens", entry.output_tokens);
            if let Some(cr) = entry.cache_read_tokens {
                println!("  Cache read:     {} tokens", cr);
            }
            if let Some(cc) = entry.cache_creation_tokens {
                println!("  Cache creation: {} tokens", cc);
            }
            if let Some(cost) = entry.cost_estimate {
                println!("  Cost:     ${:.4}", cost);
            }
        }
        None => {
            println!("Usage record #{} not found.", id);
        }
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
    use crate::token_usage::{parse_api_usage, RawTokenUsage};
    use tempfile::tempdir;

    fn setup_test_db() -> (Database, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let db = Database::open(&dir.path().join("test.db")).unwrap();
        (db, dir)
    }

    fn make_usage(agent: &str, model: &str, input: i64, output: i64) -> ParsedUsage {
        let raw = RawTokenUsage {
            input_tokens: input,
            output_tokens: output,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        };
        parse_api_usage(&raw, model, agent, None)
    }

    #[test]
    fn test_record_and_list() {
        let (db, _dir) = setup_test_db();

        let usage = make_usage("worker-1", "claude-sonnet-4-6", 1000, 500);
        record(&db, &usage).unwrap();

        let entries = db
            .list_token_usage(None, None, None, None, None, None)
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].agent_id, "worker-1");
        assert_eq!(entries[0].input_tokens, 1000);
        assert_eq!(entries[0].output_tokens, 500);
        assert!(entries[0].cost_estimate.is_some());
    }

    #[test]
    fn test_show_existing() {
        let (db, _dir) = setup_test_db();

        let usage = make_usage("worker-1", "claude-sonnet-4-6", 1000, 500);
        record(&db, &usage).unwrap();
        show(&db, 1, false).unwrap();
        show(&db, 1, true).unwrap();
    }

    #[test]
    fn test_show_missing() {
        let (db, _dir) = setup_test_db();
        show(&db, 999, false).unwrap();
    }

    #[test]
    fn test_summary_aggregation() {
        let (db, _dir) = setup_test_db();

        record(&db, &make_usage("worker-1", "claude-opus-4-6", 1000, 500)).unwrap();
        record(&db, &make_usage("worker-1", "claude-opus-4-6", 2000, 1000)).unwrap();
        record(&db, &make_usage("worker-2", "claude-sonnet-4-6", 500, 200)).unwrap();

        let rows = db.get_usage_summary(None, None, None).unwrap();
        assert_eq!(rows.len(), 2);

        let opus_row = rows.iter().find(|r| r.model.contains("opus")).unwrap();
        assert_eq!(opus_row.request_count, 2);
        assert_eq!(opus_row.total_input_tokens, 3000);
        assert_eq!(opus_row.total_output_tokens, 1500);
    }

    #[test]
    fn test_list_filter_by_agent() {
        let (db, _dir) = setup_test_db();

        record(&db, &make_usage("worker-1", "claude-opus-4-6", 1000, 500)).unwrap();
        record(&db, &make_usage("worker-2", "claude-opus-4-6", 2000, 1000)).unwrap();

        let entries = db
            .list_token_usage(Some("worker-1"), None, None, None, None, None)
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].agent_id, "worker-1");
    }
}
