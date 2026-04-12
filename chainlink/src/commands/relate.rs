use anyhow::Result;

use crate::db::{validate_relation_type, Database};
use crate::utils::format_issue_id;

#[allow(dead_code)]
pub fn add(db: &Database, issue_id: i64, related_id: i64) -> Result<()> {
    add_typed(db, issue_id, related_id, "related")
}

pub fn add_typed(db: &Database, issue_id: i64, related_id: i64, relation_type: &str) -> Result<()> {
    validate_relation_type(relation_type)?;
    db.require_issue(issue_id)?;
    db.require_issue(related_id)?;

    if db.add_typed_relation(issue_id, related_id, relation_type)? {
        println!(
            "Linked {} ↔ {} ({})",
            format_issue_id(issue_id),
            format_issue_id(related_id),
            relation_type
        );
    } else {
        println!(
            "Issues {} and {} already have a '{}' relation",
            format_issue_id(issue_id),
            format_issue_id(related_id),
            relation_type
        );
    }

    Ok(())
}

#[allow(dead_code)]
pub fn remove(db: &Database, issue_id: i64, related_id: i64) -> Result<()> {
    remove_typed(db, issue_id, related_id, "related")
}

pub fn remove_typed(
    db: &Database,
    issue_id: i64,
    related_id: i64,
    relation_type: &str,
) -> Result<()> {
    validate_relation_type(relation_type)?;

    if db.remove_typed_relation(issue_id, related_id, relation_type)? {
        println!(
            "Unlinked {} ↔ {} ({})",
            format_issue_id(issue_id),
            format_issue_id(related_id),
            relation_type
        );
    } else {
        println!(
            "No '{}' relation found between {} and {}",
            relation_type,
            format_issue_id(issue_id),
            format_issue_id(related_id)
        );
    }

    Ok(())
}

pub fn list(db: &Database, issue_id: i64) -> Result<()> {
    db.require_issue(issue_id)?;

    let relations = db.get_typed_relations(issue_id)?;

    if relations.is_empty() {
        println!("No related issues for {}", format_issue_id(issue_id));
        return Ok(());
    }

    println!("Relations for {}:", format_issue_id(issue_id));

    // Group by relation type for cleaner display
    let mut by_type: std::collections::BTreeMap<String, Vec<i64>> =
        std::collections::BTreeMap::new();
    for rel in &relations {
        let other_id = if rel.issue_id_1 == issue_id {
            rel.issue_id_2
        } else {
            rel.issue_id_1
        };
        by_type
            .entry(rel.relation_type.clone())
            .or_default()
            .push(other_id);
    }

    for (rel_type, ids) in &by_type {
        println!("\n  [{}]:", rel_type);
        for &id in ids {
            if let Some(issue) = db.get_issue(id)? {
                let status_marker = if issue.status == "closed" { "✓" } else { " " };
                println!(
                    "    {:<5} [{}] {:8} {}",
                    format_issue_id(id),
                    status_marker,
                    issue.priority,
                    issue.title
                );
            }
        }
    }

    Ok(())
}

pub fn cascade(db: &Database, issue_id: i64) -> Result<()> {
    db.require_issue(issue_id)?;

    let affected = db.falsification_cascade(issue_id)?;

    if affected.is_empty() {
        println!(
            "No downstream issues affected by falsifying {}",
            format_issue_id(issue_id)
        );
        return Ok(());
    }

    println!(
        "Falsifying {} affects {} issue(s):\n",
        format_issue_id(issue_id),
        affected.len()
    );

    for issue in &affected {
        let status_marker = if issue.status == "closed" { "✓" } else { " " };
        let parent_note = if let Some(pid) = issue.parent_id {
            format!(" (child of {})", format_issue_id(pid))
        } else {
            String::new()
        };
        println!(
            "  {:<5} [{}] {:8} {}{}",
            format_issue_id(issue.id),
            status_marker,
            issue.priority,
            issue.title,
            parent_note
        );
    }

    println!(
        "\nThese issues were built on assumptions that trace back to {}.",
        format_issue_id(issue_id)
    );
    println!("Consider reassessing each one.");

    Ok(())
}

/// Mark an issue as falsified: label it, close it, and show the cascade.
pub fn falsify(db: &Database, issue_id: i64) -> Result<()> {
    db.require_issue(issue_id)?;

    // Add "falsified" label
    db.add_label(issue_id, "falsified")?;

    // Close the issue
    db.close_issue(issue_id)?;

    let issue = db
        .get_issue(issue_id)?
        .ok_or_else(|| anyhow::anyhow!("Issue {} not found", issue_id))?;
    println!("Falsified {}: {}", format_issue_id(issue_id), issue.title);

    // Add audit comment
    db.add_comment(
        issue_id,
        "Marked as falsified. Running cascade to identify affected downstream issues.",
        "falsification",
    )?;

    // Show cascade
    let affected = db.falsification_cascade(issue_id)?;

    if affected.is_empty() {
        println!("No downstream issues affected.");
    } else {
        println!(
            "\n⚠ {} downstream issue(s) should be reassessed:\n",
            affected.len()
        );

        for issue in &affected {
            let status_marker = if issue.status == "closed" { "✓" } else { " " };
            println!(
                "  {:<5} [{}] {:8} {}",
                format_issue_id(issue.id),
                status_marker,
                issue.priority,
                issue.title
            );

            // Add a comment on each affected issue
            db.add_comment(
                issue.id,
                &format!(
                    "⚠ Upstream assumption {} was falsified. This issue may need reassessment.",
                    format_issue_id(issue_id)
                ),
                "falsification",
            )?;

            // Label affected issues
            db.add_label(issue.id, "needs-reassessment")?;
        }
    }

    Ok(())
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

    #[test]
    fn test_add_relation() {
        let (db, _dir) = setup_test_db();
        let id1 = db.create_issue("Issue 1", None, "medium").unwrap();
        let id2 = db.create_issue("Issue 2", None, "medium").unwrap();

        let result = add(&db, id1, id2);
        assert!(result.is_ok());

        let related = db.get_related_issues(id1).unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].id, id2);
    }

    #[test]
    fn test_add_typed_relation() {
        let (db, _dir) = setup_test_db();
        let id1 = db.create_issue("Assumption A", None, "medium").unwrap();
        let id2 = db.create_issue("Assumption B", None, "medium").unwrap();

        let result = add_typed(&db, id1, id2, "assumption");
        assert!(result.is_ok());

        let by_type = db.get_issues_by_relation_type(id1, "assumption").unwrap();
        assert_eq!(by_type.len(), 1);
        assert_eq!(by_type[0].id, id2);

        // Should not appear under "related"
        let by_related = db.get_issues_by_relation_type(id1, "related").unwrap();
        assert_eq!(by_related.len(), 0);
    }

    #[test]
    fn test_multiple_relation_types_between_same_issues() {
        let (db, _dir) = setup_test_db();
        let id1 = db.create_issue("Issue 1", None, "medium").unwrap();
        let id2 = db.create_issue("Issue 2", None, "medium").unwrap();

        add_typed(&db, id1, id2, "related").unwrap();
        add_typed(&db, id1, id2, "assumption").unwrap();

        let relations = db.get_typed_relations(id1).unwrap();
        assert_eq!(relations.len(), 2);
    }

    #[test]
    fn test_custom_relation_type() {
        let (db, _dir) = setup_test_db();
        let id1 = db.create_issue("Issue 1", None, "medium").unwrap();
        let id2 = db.create_issue("Issue 2", None, "medium").unwrap();

        // Unknown types are accepted with a warning
        let result = add_typed(&db, id1, id2, "caused-by");
        assert!(result.is_ok());

        let relations = db.get_typed_relations(id1).unwrap();
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0].relation_type, "caused-by");
    }

    #[test]
    fn test_empty_relation_type_rejected() {
        let (db, _dir) = setup_test_db();
        let id1 = db.create_issue("Issue 1", None, "medium").unwrap();
        let id2 = db.create_issue("Issue 2", None, "medium").unwrap();

        let result = add_typed(&db, id1, id2, "");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_add_relation_bidirectional() {
        let (db, _dir) = setup_test_db();
        let id1 = db.create_issue("Issue 1", None, "medium").unwrap();
        let id2 = db.create_issue("Issue 2", None, "medium").unwrap();

        add(&db, id1, id2).unwrap();

        let related1 = db.get_related_issues(id1).unwrap();
        let related2 = db.get_related_issues(id2).unwrap();
        assert_eq!(related1.len(), 1);
        assert_eq!(related2.len(), 1);
    }

    #[test]
    fn test_add_relation_nonexistent_issue() {
        let (db, _dir) = setup_test_db();
        let id = db.create_issue("Issue 1", None, "medium").unwrap();

        let result = add(&db, id, 99999);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_add_duplicate_relation() {
        let (db, _dir) = setup_test_db();
        let id1 = db.create_issue("Issue 1", None, "medium").unwrap();
        let id2 = db.create_issue("Issue 2", None, "medium").unwrap();

        add(&db, id1, id2).unwrap();
        let result = add(&db, id1, id2);
        assert!(result.is_ok());

        let related = db.get_related_issues(id1).unwrap();
        assert_eq!(related.len(), 1);
    }

    #[test]
    fn test_remove_relation() {
        let (db, _dir) = setup_test_db();
        let id1 = db.create_issue("Issue 1", None, "medium").unwrap();
        let id2 = db.create_issue("Issue 2", None, "medium").unwrap();

        add(&db, id1, id2).unwrap();
        let result = remove(&db, id1, id2);
        assert!(result.is_ok());

        let related = db.get_related_issues(id1).unwrap();
        assert_eq!(related.len(), 0);
    }

    #[test]
    fn test_remove_typed_relation() {
        let (db, _dir) = setup_test_db();
        let id1 = db.create_issue("Issue 1", None, "medium").unwrap();
        let id2 = db.create_issue("Issue 2", None, "medium").unwrap();

        add_typed(&db, id1, id2, "assumption").unwrap();
        add_typed(&db, id1, id2, "related").unwrap();

        remove_typed(&db, id1, id2, "assumption").unwrap();

        // "related" should still exist
        let relations = db.get_typed_relations(id1).unwrap();
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0].relation_type, "related");
    }

    #[test]
    fn test_remove_nonexistent_relation() {
        let (db, _dir) = setup_test_db();
        let id1 = db.create_issue("Issue 1", None, "medium").unwrap();
        let id2 = db.create_issue("Issue 2", None, "medium").unwrap();

        let result = remove(&db, id1, id2);
        assert!(result.is_ok());
    }

    #[test]
    fn test_list_relations() {
        let (db, _dir) = setup_test_db();
        let id1 = db.create_issue("Issue 1", None, "medium").unwrap();
        let id2 = db.create_issue("Issue 2", None, "medium").unwrap();
        let id3 = db.create_issue("Issue 3", None, "medium").unwrap();

        add(&db, id1, id2).unwrap();
        add(&db, id1, id3).unwrap();

        let result = list(&db, id1);
        assert!(result.is_ok());
    }

    #[test]
    fn test_list_relations_nonexistent() {
        let (db, _dir) = setup_test_db();

        let result = list(&db, 99999);
        assert!(result.is_err());
    }

    #[test]
    fn test_list_no_relations() {
        let (db, _dir) = setup_test_db();
        let id = db.create_issue("Lonely issue", None, "medium").unwrap();

        let result = list(&db, id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_falsification_cascade_parent_child() {
        let (db, _dir) = setup_test_db();
        let root = db.create_issue("Root assumption", None, "high").unwrap();
        let child1 = db.create_subissue(root, "Why 1", None, "medium").unwrap();
        let child2 = db.create_subissue(root, "Why 2", None, "medium").unwrap();
        let grandchild = db
            .create_subissue(child1, "Why 1.1", None, "medium")
            .unwrap();

        let affected = db.falsification_cascade(root).unwrap();
        let affected_ids: Vec<i64> = affected.iter().map(|i| i.id).collect();

        assert!(affected_ids.contains(&child1));
        assert!(affected_ids.contains(&child2));
        assert!(affected_ids.contains(&grandchild));
        assert_eq!(affected.len(), 3);
    }

    #[test]
    fn test_falsification_cascade_derived_relations() {
        let (db, _dir) = setup_test_db();
        let assumption = db.create_issue("Core assumption", None, "high").unwrap();
        let conclusion = db
            .create_issue("Conclusion built on assumption", None, "medium")
            .unwrap();

        db.add_typed_relation(assumption, conclusion, "derived")
            .unwrap();

        let affected = db.falsification_cascade(assumption).unwrap();
        assert_eq!(affected.len(), 1);
        assert_eq!(affected[0].id, conclusion);
    }

    #[test]
    fn test_falsification_cascade_assumption_one_hop() {
        let (db, _dir) = setup_test_db();
        let a1 = db.create_issue("Assumption A", None, "high").unwrap();
        let a2 = db
            .create_issue("Assumption B (shared)", None, "medium")
            .unwrap();
        let a3 = db
            .create_issue("Assumption C (shared with B)", None, "medium")
            .unwrap();

        db.add_typed_relation(a1, a2, "assumption").unwrap();
        db.add_typed_relation(a2, a3, "assumption").unwrap();

        // Falsifying a1 should flag a2 (one hop) but NOT a3 (two hops via assumption)
        let affected = db.falsification_cascade(a1).unwrap();
        let affected_ids: Vec<i64> = affected.iter().map(|i| i.id).collect();

        assert!(affected_ids.contains(&a2));
        assert!(!affected_ids.contains(&a3));
    }

    #[test]
    fn test_falsify_command() {
        let (db, _dir) = setup_test_db();
        let root = db.create_issue("Bad assumption", None, "high").unwrap();
        let child = db
            .create_subissue(root, "Built on bad assumption", None, "medium")
            .unwrap();

        falsify(&db, root).unwrap();

        // Root should be closed and labeled
        let root_issue = db.get_issue(root).unwrap().unwrap();
        assert_eq!(root_issue.status, "closed");
        let labels = db.get_labels(root).unwrap();
        assert!(labels.contains(&"falsified".to_string()));

        // Child should be labeled for reassessment
        let child_labels = db.get_labels(child).unwrap();
        assert!(child_labels.contains(&"needs-reassessment".to_string()));
    }
}
