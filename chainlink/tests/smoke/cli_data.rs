use std::fs;

use super::harness::{assert_stdout_contains, SmokeHarness};

// ==================== Import/Export Tests ====================

#[test]
fn test_export_empty_db_json() {
    let h = SmokeHarness::new();
    let export_path = h.temp_dir.path().join("export.json");
    h.run_ok(&["export", "-o", export_path.to_str().unwrap(), "-f", "json"]);

    let content = fs::read_to_string(&export_path).expect("Failed to read export file");
    let parsed: serde_json::Value =
        serde_json::from_str(&content).expect("Export is not valid JSON");
    // Should be valid JSON (either empty array or wrapper object)
    assert!(
        parsed.is_array() || parsed.is_object(),
        "Export should be a JSON array or object, got: {}",
        content
    );
}

#[test]
fn test_export_json_format() {
    let h = SmokeHarness::new();

    h.run_ok(&["create", "First issue", "-p", "high"]);
    h.run_ok(&["create", "Second issue", "-d", "Has a description"]);
    h.run_ok(&["label", "1", "bug"]);
    h.run_ok(&["comment", "1", "A comment on issue 1"]);

    let export_path = h.temp_dir.path().join("export.json");
    h.run_ok(&["export", "-o", export_path.to_str().unwrap(), "-f", "json"]);

    let content = fs::read_to_string(&export_path).expect("Failed to read export file");
    let parsed: serde_json::Value =
        serde_json::from_str(&content).expect("Export is not valid JSON");

    // Find the issues array (may be top-level or nested under "issues")
    let issues = if parsed.is_array() {
        parsed.as_array().unwrap().clone()
    } else if let Some(arr) = parsed.get("issues").and_then(|v| v.as_array()) {
        arr.clone()
    } else {
        panic!("Could not find issues array in export: {}", content);
    };

    assert_eq!(issues.len(), 2, "Should export 2 issues");

    let first = issues
        .iter()
        .find(|i| i["title"].as_str() == Some("First issue"))
        .expect("Should find 'First issue' in export");

    assert_eq!(first["priority"].as_str().unwrap(), "high");
}

#[test]
fn test_export_markdown_format() {
    let h = SmokeHarness::new();

    h.run_ok(&["create", "Open issue", "-p", "high"]);
    h.run_ok(&["create", "Closed issue"]);
    h.run_ok(&["close", "2"]);

    let export_path = h.temp_dir.path().join("export.md");
    h.run_ok(&[
        "export",
        "-o",
        export_path.to_str().unwrap(),
        "-f",
        "markdown",
    ]);

    let content = fs::read_to_string(&export_path).expect("Failed to read export file");

    assert!(
        content.contains("Open"),
        "Markdown should have Open section"
    );
    assert!(
        content.contains("Closed"),
        "Markdown should have Closed section"
    );
    assert!(
        content.contains("Open issue"),
        "Markdown should contain issue title"
    );
    assert!(
        content.contains("Closed issue"),
        "Markdown should contain closed issue title"
    );
}

#[test]
fn test_import_malformed_json() {
    let h = SmokeHarness::new();

    let malformed = r#"[{"id": 1, "title": "Trunc"#;
    let import_path = h.temp_dir.path().join("malformed.json");
    fs::write(&import_path, malformed).unwrap();

    let result = h.run_err(&["import", import_path.to_str().unwrap()]);
    assert!(
        result.stderr.contains("parse")
            || result.stderr.contains("JSON")
            || result.stderr.contains("error")
            || result.stderr.contains("invalid"),
        "Should indicate JSON parse error, got stderr: {}",
        result.stderr
    );
}

#[test]
fn test_import_export_roundtrip() {
    let h = SmokeHarness::new();

    // Create 10 issues with labels and comments
    for i in 1..=10 {
        h.run_ok(&["create", &format!("Roundtrip issue {}", i), "-p", "medium"]);
    }
    h.run_ok(&["label", "1", "bug"]);
    h.run_ok(&["label", "2", "feature"]);
    h.run_ok(&["label", "3", "bug"]);
    h.run_ok(&["comment", "1", "Comment on issue 1"]);
    h.run_ok(&["comment", "2", "Comment on issue 2"]);
    h.run_ok(&["comment", "5", "Comment on issue 5"]);
    h.run_ok(&["close", "4"]);
    h.run_ok(&["close", "7"]);

    // Export
    let export1_path = h.temp_dir.path().join("export1.json");
    h.run_ok(&["export", "-o", export1_path.to_str().unwrap(), "-f", "json"]);
    let export1 = fs::read_to_string(&export1_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&export1).unwrap();
    let issues1 = if parsed.is_array() {
        parsed.as_array().unwrap().clone()
    } else {
        parsed["issues"].as_array().unwrap().clone()
    };
    assert_eq!(issues1.len(), 10);

    // Delete the database and reinitialize
    fs::remove_file(h.db_path()).expect("Failed to remove database");
    h.run_ok(&["init"]);

    // Import
    h.run_ok(&["import", export1_path.to_str().unwrap()]);

    // Export again
    let export2_path = h.temp_dir.path().join("export2.json");
    h.run_ok(&["export", "-o", export2_path.to_str().unwrap(), "-f", "json"]);
    let export2 = fs::read_to_string(&export2_path).unwrap();
    let parsed2: serde_json::Value = serde_json::from_str(&export2).unwrap();
    let issues2 = if parsed2.is_array() {
        parsed2.as_array().unwrap().clone()
    } else {
        parsed2["issues"].as_array().unwrap().clone()
    };

    assert_eq!(
        issues1.len(),
        issues2.len(),
        "Roundtrip should preserve issue count"
    );

    let mut titles1: Vec<String> = issues1
        .iter()
        .map(|i| i["title"].as_str().unwrap().to_string())
        .collect();
    let mut titles2: Vec<String> = issues2
        .iter()
        .map(|i| i["title"].as_str().unwrap().to_string())
        .collect();
    titles1.sort();
    titles2.sort();
    assert_eq!(titles1, titles2, "Roundtrip should preserve issue titles");
}

// ==================== Archive Tests ====================

#[test]
fn test_archive_full_lifecycle() {
    let h = SmokeHarness::new();

    h.run_ok(&["create", "Archive me"]);
    h.run_ok(&["close", "1"]);

    let result = h.run_ok(&["archive", "add", "1"]);
    assert_stdout_contains(&result, "Archived");

    let list_result = h.run_ok(&["archive", "list"]);
    assert!(
        list_result.stdout.contains("Archive me"),
        "Archived issue should appear in archive list"
    );

    // Should not appear in open or closed lists
    let open_list = h.run_ok(&["list", "-s", "open"]);
    assert!(
        !open_list.stdout.contains("Archive me"),
        "Archived issue should not appear in open list"
    );
    let closed_list = h.run_ok(&["list", "-s", "closed"]);
    assert!(
        !closed_list.stdout.contains("Archive me"),
        "Archived issue should not appear in closed list"
    );

    // Unarchive
    let unarchive_result = h.run_ok(&["archive", "remove", "1"]);
    assert_stdout_contains(&unarchive_result, "Unarchived");

    // Should now appear in closed list
    let closed_list = h.run_ok(&["list", "-s", "closed"]);
    assert!(
        closed_list.stdout.contains("Archive me"),
        "Unarchived issue should appear in closed list"
    );
}

#[test]
fn test_archive_open_issue_fails_smoke() {
    let h = SmokeHarness::new();

    h.run_ok(&["create", "Open issue"]);

    let result = h.run_err(&["archive", "add", "1"]);
    assert!(
        result.stderr.contains("closed")
            || result.stderr.contains("only archive closed")
            || result.stderr.contains("Can only archive")
            || result.stderr.contains("not closed"),
        "Should indicate only closed issues can be archived, got stderr: {}",
        result.stderr
    );
}

#[test]
fn test_archive_older_batch() {
    let h = SmokeHarness::new();

    for i in 1..=5 {
        h.run_ok(&["create", &format!("Issue {}", i)]);
    }
    for i in 1..=5 {
        h.run_ok(&["close", &i.to_string()]);
    }

    let result = h.run_ok(&["archive", "older", "0"]);
    assert!(
        result.stdout.contains("Archived") || result.stdout.contains("archived"),
        "Should indicate issues were archived, got: {}",
        result.stdout
    );

    let archive_list = h.run_ok(&["archive", "list"]);
    for i in 1..=5 {
        assert!(
            archive_list.stdout.contains(&format!("Issue {}", i)),
            "Issue {} should be in archive list",
            i
        );
    }
}

#[test]
fn test_unarchive_not_archived() {
    let h = SmokeHarness::new();

    h.run_ok(&["create", "Not archived"]);

    let result = h.run_err(&["archive", "remove", "1"]);
    assert!(
        result.stderr.contains("not found or not archived")
            || result.stderr.contains("not archived")
            || result.stderr.contains("not found"),
        "Should indicate issue is not archived, got stderr: {}",
        result.stderr
    );
}

// ==================== Next command ====================

#[test]
fn test_next_suggests_highest_priority() {
    let h = SmokeHarness::new();

    h.run_ok(&["create", "Low prio task", "-p", "low"]);
    h.run_ok(&["create", "Critical task", "-p", "critical"]);
    h.run_ok(&["create", "Medium task", "-p", "medium"]);

    let next = h.run_ok(&["next"]);
    // The critical task should be suggested first
    assert!(
        next.stdout.contains("Critical task"),
        "next should suggest highest priority issue.\nstdout: {}",
        next.stdout
    );
}

#[test]
fn test_next_skips_blocked() {
    let h = SmokeHarness::new();

    h.run_ok(&["create", "Blocked task", "-p", "critical"]);
    h.run_ok(&["create", "Blocker task", "-p", "low"]);
    h.run_ok(&["block", "1", "2"]);

    let next = h.run_ok(&["next"]);
    // Should suggest the blocker (which is unblocked) not the blocked task
    assert!(
        !next.stdout.contains("Blocked task") || next.stdout.contains("Blocker task"),
        "next should not suggest blocked issues.\nstdout: {}",
        next.stdout
    );
}
