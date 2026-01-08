use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;

use crate::db::Database;

pub fn close(db: &Database, id: i64, update_changelog: bool, chainlink_dir: &Path) -> Result<()> {
    // Get issue details before closing
    let issue = db.get_issue(id)?;
    let issue = match issue {
        Some(i) => i,
        None => bail!("Issue #{} not found", id),
    };
    let labels = db.get_labels(id)?;

    if db.close_issue(id)? {
        println!("Closed issue #{}", id);
    } else {
        bail!("Issue #{} not found", id);
    }

    // Update changelog if requested
    if update_changelog {
        let project_root = chainlink_dir.parent().unwrap_or(chainlink_dir);
        let changelog_path = project_root.join("CHANGELOG.md");

        // Create CHANGELOG.md if it doesn't exist
        if !changelog_path.exists() {
            if let Err(e) = create_changelog(&changelog_path) {
                eprintln!("Warning: Could not create CHANGELOG.md: {}", e);
            } else {
                println!("Created CHANGELOG.md");
            }
        }

        if changelog_path.exists() {
            let category = determine_changelog_category(&labels);
            let entry = format!("- {} (#{})\n", issue.title, id);

            if let Err(e) = append_to_changelog(&changelog_path, &category, &entry) {
                eprintln!("Warning: Could not update CHANGELOG.md: {}", e);
            } else {
                println!("Added to CHANGELOG.md under {}", category);
            }
        }
    }

    Ok(())
}

fn create_changelog(path: &Path) -> Result<()> {
    let template = r#"# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

### Added

### Fixed

### Changed
"#;
    fs::write(path, template).context("Failed to create CHANGELOG.md")?;
    Ok(())
}

fn determine_changelog_category(labels: &[String]) -> String {
    for label in labels {
        match label.to_lowercase().as_str() {
            "bug" | "fix" | "bugfix" => return "Fixed".to_string(),
            "feature" | "enhancement" => return "Added".to_string(),
            "breaking" | "breaking-change" => return "Changed".to_string(),
            "deprecated" => return "Deprecated".to_string(),
            "removed" => return "Removed".to_string(),
            "security" => return "Security".to_string(),
            _ => continue,
        }
    }
    "Changed".to_string() // Default category
}

fn append_to_changelog(path: &Path, category: &str, entry: &str) -> Result<()> {
    let content = fs::read_to_string(path).context("Failed to read CHANGELOG.md")?;
    let heading = format!("### {}", category);

    let new_content = if content.contains(&heading) {
        // Insert after the heading
        let mut result = String::new();
        let mut found = false;
        for line in content.lines() {
            result.push_str(line);
            result.push('\n');
            if !found && line.trim() == heading {
                result.push_str(entry);
                found = true;
            }
        }
        result
    } else {
        // Add new section after first ## heading (usually ## [Unreleased])
        let mut result = String::new();
        let mut added = false;
        for line in content.lines() {
            result.push_str(line);
            result.push('\n');
            if !added && line.starts_with("## ") {
                result.push('\n');
                result.push_str(&format!("{}\n", heading));
                result.push_str(entry);
                added = true;
            }
        }
        if !added {
            // No ## heading found, append at end
            result.push_str(&format!("\n{}\n", heading));
            result.push_str(entry);
        }
        result
    };

    fs::write(path, new_content).context("Failed to write CHANGELOG.md")?;
    Ok(())
}

pub fn reopen(db: &Database, id: i64) -> Result<()> {
    if db.reopen_issue(id)? {
        println!("Reopened issue #{}", id);
    } else {
        bail!("Issue #{} not found", id);
    }
    Ok(())
}
