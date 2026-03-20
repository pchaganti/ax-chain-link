use anyhow::{bail, Result};
use chrono::Utc;
use serde::Serialize;

use std::path::Path;

use crate::db::Database;
use crate::lock_check;
use crate::utils::format_issue_id;

pub fn start(db: &Database, chainlink_dir: &Path) -> Result<()> {
    // Check if there's already an active session
    if let Some(current) = db.get_current_session()? {
        println!(
            "Session {} is already active (started {})",
            format_issue_id(current.id),
            current.started_at.format("%Y-%m-%d %H:%M")
        );
        return Ok(());
    }

    // Show previous session's handoff notes
    if let Some(last) = db.get_last_session()? {
        if let Some(ended) = last.ended_at {
            println!("Previous session ended: {}", ended.format("%Y-%m-%d %H:%M"));
        }
        if let Some(notes) = &last.handoff_notes {
            if !notes.is_empty() {
                println!("Handoff notes:");
                for line in notes.lines() {
                    println!("  {}", line);
                }
                println!();
            }
        }
    }

    // Load agent identity if configured
    let agent_id = crate::identity::AgentConfig::load(chainlink_dir)
        .ok()
        .flatten()
        .map(|a| a.agent_id);

    let id = db.start_session_with_agent(agent_id.as_deref())?;
    println!("Session {} started.", format_issue_id(id));
    if let Some(ref aid) = agent_id {
        println!("Agent: {}", aid);
    }
    Ok(())
}

pub fn end(db: &Database, notes: Option<&str>) -> Result<()> {
    let session = match db.get_current_session()? {
        Some(s) => s,
        None => bail!("No active session"),
    };

    db.end_session(session.id, notes)?;
    println!("Session {} ended.", format_issue_id(session.id));
    if notes.is_some() {
        println!("Handoff notes saved.");
    }
    Ok(())
}

pub fn status(db: &Database) -> Result<()> {
    let session = match db.get_current_session()? {
        Some(s) => s,
        None => {
            println!("No active session. Use 'chainlink session start' to begin.");
            return Ok(());
        }
    };

    let duration = Utc::now() - session.started_at;
    let minutes = duration.num_minutes();

    println!(
        "Session {} (started {})",
        format_issue_id(session.id),
        session.started_at.format("%Y-%m-%d %H:%M")
    );

    if let Some(issue_id) = session.active_issue_id {
        if let Some(issue) = db.get_issue(issue_id)? {
            println!("Working on: {} {}", format_issue_id(issue.id), issue.title);
        } else {
            println!("Working on: {} (issue not found)", format_issue_id(issue_id));
        }
    } else {
        println!("Working on: (none)");
    }

    if let Some(ref action) = session.last_action {
        println!("Last action: {}", action);
    }

    println!("Duration: {} minutes", minutes);
    Ok(())
}

#[derive(Serialize)]
struct SessionStatus {
    session_id: i64,
    started_at: String,
    duration_minutes: i64,
    active_issue: Option<ActiveIssue>,
    last_action: Option<String>,
}

#[derive(Serialize)]
struct ActiveIssue {
    id: i64,
    title: String,
}

pub fn status_json(db: &Database) -> Result<()> {
    let session = match db.get_current_session()? {
        Some(s) => s,
        None => {
            println!("null");
            return Ok(());
        }
    };

    let duration = Utc::now() - session.started_at;

    let active_issue = if let Some(issue_id) = session.active_issue_id {
        db.get_issue(issue_id)?.map(|issue| ActiveIssue {
            id: issue.id,
            title: issue.title,
        })
    } else {
        None
    };

    let status = SessionStatus {
        session_id: session.id,
        started_at: session.started_at.to_rfc3339(),
        duration_minutes: duration.num_minutes(),
        active_issue,
        last_action: session.last_action,
    };

    println!("{}", serde_json::to_string_pretty(&status)?);
    Ok(())
}

pub fn work(db: &Database, issue_id: i64, chainlink_dir: &Path) -> Result<()> {
    let session = match db.get_current_session()? {
        Some(s) => s,
        None => bail!("No active session. Use 'chainlink session start' first."),
    };

    let issue = match db.get_issue(issue_id)? {
        Some(i) => i,
        None => bail!("Issue {} not found", format_issue_id(issue_id)),
    };

    // Check lock before allowing work on this issue
    lock_check::enforce_lock(chainlink_dir, issue_id, db)?;

    db.set_session_issue(session.id, issue_id)?;
    println!("Now working on: {} {}", format_issue_id(issue.id), issue.title);
    Ok(())
}

pub fn action(db: &Database, text: &str) -> Result<()> {
    let session = match db.get_current_session()? {
        Some(s) => s,
        None => bail!("No active session. Use 'chainlink session start' first."),
    };

    db.set_session_action(session.id, text)?;
    println!("Action recorded: {}", text);

    // Auto-comment on the active issue if one is set
    if let Some(issue_id) = session.active_issue_id {
        db.add_comment(issue_id, &format!("[action] {}", text), "note")?;
    }

    Ok(())
}

pub fn last_handoff(db: &Database) -> Result<()> {
    match db.get_last_session()? {
        Some(session) => {
            if let Some(notes) = &session.handoff_notes {
                if !notes.is_empty() {
                    println!("{}", notes);
                    return Ok(());
                }
            }
            println!("No previous handoff notes.");
        }
        None => {
            println!("No previous session found.");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use tempfile::tempdir;

    fn setup_test_db() -> (Database, std::path::PathBuf, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).unwrap();
        let chainlink_dir = dir.path().join(".chainlink");
        std::fs::create_dir_all(&chainlink_dir).unwrap();
        (db, chainlink_dir, dir)
    }

    // ==================== Start Tests ====================

    #[test]
    fn test_start_session() {
        let (db, cl, _dir) = setup_test_db();

        let result = start(&db, &cl);
        assert!(result.is_ok());

        let session = db.get_current_session().unwrap();
        assert!(session.is_some());
    }

    #[test]
    fn test_start_already_active() {
        let (db, cl, _dir) = setup_test_db();

        start(&db, &cl).unwrap();
        let first_session = db.get_current_session().unwrap().unwrap();

        // Starting again should not create new session
        let result = start(&db, &cl);
        assert!(result.is_ok());

        let current = db.get_current_session().unwrap().unwrap();
        assert_eq!(current.id, first_session.id);
    }

    // ==================== End Tests ====================

    #[test]
    fn test_end_session() {
        let (db, cl, _dir) = setup_test_db();

        start(&db, &cl).unwrap();
        let result = end(&db, None);
        assert!(result.is_ok());

        let session = db.get_current_session().unwrap();
        assert!(session.is_none());
    }

    #[test]
    fn test_end_session_with_notes() {
        let (db, cl, _dir) = setup_test_db();

        start(&db, &cl).unwrap();
        let result = end(&db, Some("Completed auth feature"));
        assert!(result.is_ok());

        let last = db.get_last_session().unwrap().unwrap();
        assert_eq!(
            last.handoff_notes,
            Some("Completed auth feature".to_string())
        );
    }

    #[test]
    fn test_end_no_active_session() {
        let (db, _cl, _dir) = setup_test_db();

        let result = end(&db, None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No active session"));
    }

    // ==================== Status Tests ====================

    #[test]
    fn test_status_no_session() {
        let (db, _cl, _dir) = setup_test_db();

        let result = status(&db);
        assert!(result.is_ok());
    }

    #[test]
    fn test_status_with_session() {
        let (db, cl, _dir) = setup_test_db();

        start(&db, &cl).unwrap();
        let result = status(&db);
        assert!(result.is_ok());
    }

    #[test]
    fn test_status_with_active_issue() {
        let (db, cl, _dir) = setup_test_db();

        let issue_id = db.create_issue("Test issue", None, "medium").unwrap();
        start(&db, &cl).unwrap();
        work(&db, issue_id, &cl).unwrap();

        let result = status(&db);
        assert!(result.is_ok());
    }

    // ==================== Work Tests ====================

    #[test]
    fn test_work_sets_active_issue() {
        let (db, cl, _dir) = setup_test_db();

        let issue_id = db.create_issue("Test issue", None, "medium").unwrap();
        start(&db, &cl).unwrap();

        let result = work(&db, issue_id, &cl);
        assert!(result.is_ok());

        let session = db.get_current_session().unwrap().unwrap();
        assert_eq!(session.active_issue_id, Some(issue_id));
    }

    #[test]
    fn test_work_no_session() {
        let (db, cl, _dir) = setup_test_db();

        let issue_id = db.create_issue("Test issue", None, "medium").unwrap();

        let result = work(&db, issue_id, &cl);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No active session"));
    }

    #[test]
    fn test_work_nonexistent_issue() {
        let (db, cl, _dir) = setup_test_db();

        start(&db, &cl).unwrap();

        let result = work(&db, 99999, &cl);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_work_change_active_issue() {
        let (db, cl, _dir) = setup_test_db();

        let issue1 = db.create_issue("Issue 1", None, "medium").unwrap();
        let issue2 = db.create_issue("Issue 2", None, "medium").unwrap();
        start(&db, &cl).unwrap();

        work(&db, issue1, &cl).unwrap();
        let session = db.get_current_session().unwrap().unwrap();
        assert_eq!(session.active_issue_id, Some(issue1));

        work(&db, issue2, &cl).unwrap();
        let session = db.get_current_session().unwrap().unwrap();
        assert_eq!(session.active_issue_id, Some(issue2));
    }

    // ==================== Last Handoff Tests ====================

    #[test]
    fn test_last_handoff_no_sessions() {
        let (db, _cl, _dir) = setup_test_db();

        let result = last_handoff(&db);
        assert!(result.is_ok());
    }

    #[test]
    fn test_last_handoff_no_notes() {
        let (db, cl, _dir) = setup_test_db();

        start(&db, &cl).unwrap();
        end(&db, None).unwrap();

        let result = last_handoff(&db);
        assert!(result.is_ok());
    }

    #[test]
    fn test_last_handoff_with_notes() {
        let (db, cl, _dir) = setup_test_db();

        start(&db, &cl).unwrap();
        end(&db, Some("Important handoff notes")).unwrap();

        let result = last_handoff(&db);
        assert!(result.is_ok());
        let last = db.get_last_session().unwrap().unwrap();
        assert_eq!(
            last.handoff_notes,
            Some("Important handoff notes".to_string())
        );
    }

    // ==================== Full Workflow Tests ====================

    #[test]
    fn test_full_session_workflow() {
        let (db, cl, _dir) = setup_test_db();

        // Start session
        start(&db, &cl).unwrap();
        assert!(db.get_current_session().unwrap().is_some());

        // Create and work on issue
        let issue_id = db.create_issue("Feature", None, "high").unwrap();
        work(&db, issue_id, &cl).unwrap();

        // Check status
        status(&db).unwrap();

        // End with notes
        end(&db, Some("Made progress on feature")).unwrap();
        assert!(db.get_current_session().unwrap().is_none());

        // Start new session
        start(&db, &cl).unwrap();
        let last = db.get_last_session().unwrap().unwrap();
        assert_eq!(
            last.handoff_notes,
            Some("Made progress on feature".to_string())
        );
    }

    // ==================== Property-Based Tests ====================

    proptest! {
        #[test]
        fn prop_start_end_cycle(iterations in 1usize..5) {
            let (db, cl, _dir) = setup_test_db();

            for _ in 0..iterations {
                start(&db, &cl).unwrap();
                prop_assert!(db.get_current_session().unwrap().is_some());
                end(&db, None).unwrap();
                prop_assert!(db.get_current_session().unwrap().is_none());
            }
        }

        #[test]
        fn prop_handoff_notes_roundtrip(notes in "[a-zA-Z0-9 ]{0,100}") {
            let (db, cl, _dir) = setup_test_db();

            start(&db, &cl).unwrap();
            end(&db, Some(&notes)).unwrap();

            let last = db.get_last_session().unwrap().unwrap();
            prop_assert_eq!(last.handoff_notes, Some(notes));
        }

        #[test]
        fn prop_work_nonexistent_fails(issue_id in 1000i64..10000) {
            let (db, cl, _dir) = setup_test_db();

            start(&db, &cl).unwrap();
            let result = work(&db, issue_id, &cl);
            prop_assert!(result.is_err());
        }
    }
}
