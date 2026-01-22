use anyhow::Result;

use crate::db::Database;

pub fn run(
    db: &Database,
    status: Option<&str>,
    label: Option<&str>,
    priority: Option<&str>,
) -> Result<()> {
    let issues = db.list_issues(status, label, priority)?;

    if issues.is_empty() {
        println!("No issues found.");
        return Ok(());
    }

    for issue in issues {
        let status_display = format!("[{}]", issue.status);
        let date = issue.created_at.format("%Y-%m-%d");
        println!(
            "#{:<4} {:8} {:<40} {:8} {}",
            issue.id,
            status_display,
            truncate(&issue.title, 40),
            issue.priority,
            date
        );
    }

    Ok(())
}

fn truncate(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars - 3).collect();
        format!("{}...", truncated)
    }
}
