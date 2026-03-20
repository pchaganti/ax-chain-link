use anyhow::Result;
use std::path::Path;

use crate::db::Database;
use crate::identity::AgentConfig;
use crate::sync::{GpgVerification, SyncManager};
use crate::utils::{format_issue_id, truncate};

/// `chainlink locks list` — show current lock state
pub fn list(chainlink_dir: &Path, db: &Database, json_output: bool) -> Result<()> {
    let sync = SyncManager::new(chainlink_dir)?;
    sync.init_cache()?;
    sync.fetch()?;

    let locks_file = sync.read_locks()?;

    if json_output {
        let json = serde_json::to_string_pretty(&locks_file)?;
        println!("{}", json);
        return Ok(());
    }

    if locks_file.locks.is_empty() {
        println!("No active locks.");
        return Ok(());
    }

    let stale = sync.find_stale_locks()?;
    let stale_ids: Vec<i64> = stale.iter().map(|(id, _)| *id).collect();

    println!("Active locks:");
    for (issue_id_str, lock) in &locks_file.locks {
        let issue_id: i64 = issue_id_str.parse().unwrap_or(0);
        let title = db
            .get_issue(issue_id)?
            .map(|i| truncate(&i.title, 40))
            .unwrap_or_else(|| "(unknown issue)".to_string());

        let stale_marker = if stale_ids.contains(&issue_id) {
            " [STALE]"
        } else {
            ""
        };

        println!(
            "  {:<5} {} -- claimed by {} on {}{}",
            format_issue_id(issue_id),
            title,
            lock.agent_id,
            lock.claimed_at.format("%Y-%m-%d %H:%M"),
            stale_marker
        );
        if let Some(branch) = &lock.branch {
            println!("         branch: {}", branch);
        }
    }
    Ok(())
}

/// `chainlink locks check <id>` — check if an issue is locked
pub fn check(chainlink_dir: &Path, issue_id: i64) -> Result<()> {
    let sync = SyncManager::new(chainlink_dir)?;
    sync.init_cache()?;
    sync.fetch()?;

    let locks_file = sync.read_locks()?;

    match locks_file.get_lock(issue_id) {
        Some(lock) => {
            println!(
                "Issue {} is locked by '{}' (claimed {})",
                format_issue_id(issue_id),
                lock.agent_id,
                lock.claimed_at.format("%Y-%m-%d %H:%M")
            );
            if let Some(branch) = &lock.branch {
                println!("  Branch: {}", branch);
            }
            let stale = sync.find_stale_locks()?;
            if stale.iter().any(|(id, _)| *id == issue_id) {
                println!("  Warning: this lock appears STALE (no recent heartbeat)");
            }
        }
        None => {
            println!(
                "Issue {} is not locked. Available for claiming.",
                format_issue_id(issue_id)
            );
        }
    }
    Ok(())
}

/// `chainlink locks claim <id> [--branch <branch>]` — claim a lock
pub fn claim(chainlink_dir: &Path, issue_id: i64, branch: Option<&str>) -> Result<()> {
    let agent = AgentConfig::load(chainlink_dir)?.ok_or_else(|| {
        anyhow::anyhow!("No agent configured. Run 'chainlink agent init <id>' first.")
    })?;

    let sync = SyncManager::new(chainlink_dir)?;
    sync.init_cache()?;
    sync.fetch()?;

    match sync.claim_lock(&agent, issue_id, branch, false)? {
        true => {
            println!("Claimed lock on issue {}", format_issue_id(issue_id));
            if let Some(b) = branch {
                println!("  Branch: {}", b);
            }
        }
        false => {
            println!(
                "You already hold the lock on issue {}",
                format_issue_id(issue_id)
            );
        }
    }
    Ok(())
}

/// `chainlink locks release <id>` — release a lock
pub fn release(chainlink_dir: &Path, issue_id: i64) -> Result<()> {
    let agent = AgentConfig::load(chainlink_dir)?.ok_or_else(|| {
        anyhow::anyhow!("No agent configured. Run 'chainlink agent init <id>' first.")
    })?;

    let sync = SyncManager::new(chainlink_dir)?;
    sync.init_cache()?;
    sync.fetch()?;

    match sync.release_lock(&agent, issue_id, false)? {
        true => println!("Released lock on issue {}", format_issue_id(issue_id)),
        false => println!("Issue {} is not locked.", format_issue_id(issue_id)),
    }
    Ok(())
}

/// `chainlink locks steal <id>` — force-steal a stale lock
pub fn steal(chainlink_dir: &Path, issue_id: i64) -> Result<()> {
    let agent = AgentConfig::load(chainlink_dir)?.ok_or_else(|| {
        anyhow::anyhow!("No agent configured. Run 'chainlink agent init <id>' first.")
    })?;

    let sync = SyncManager::new(chainlink_dir)?;
    sync.init_cache()?;
    sync.fetch()?;

    sync.claim_lock(&agent, issue_id, None, true)?;
    println!(
        "Stole lock on issue {} for agent '{}'",
        format_issue_id(issue_id),
        agent.agent_id
    );
    Ok(())
}

/// `chainlink sync` — fetch locks, verify signatures, report status
pub fn sync_cmd(chainlink_dir: &Path) -> Result<()> {
    let sync = SyncManager::new(chainlink_dir)?;
    sync.init_cache()?;
    sync.fetch()?;

    println!("Sync complete.");
    println!("  Remote: {}", sync.remote());
    println!("  Cache:  {}", sync.cache_path().display());

    // Show lock count
    let locks = sync.read_locks()?;
    println!("  Active locks: {}", locks.locks.len());

    // Show stale locks
    let stale = sync.find_stale_locks()?;
    if !stale.is_empty() {
        println!("  Stale locks:");
        for (id, agent) in &stale {
            println!("    {} (held by '{}')", format_issue_id(*id), agent);
        }
    }

    // Load trust keyring for signature verification
    let keyring = sync.read_keyring()?;

    // Verify GPG signature and check trust
    match sync.verify_locks_signature()? {
        GpgVerification::Valid {
            commit,
            fingerprint,
        } => {
            let trust_status = match &fingerprint {
                Some(fp) if keyring.is_trusted(fp) => "TRUSTED",
                Some(_) if keyring.trusted_fingerprints.is_empty() => "no keyring configured",
                Some(_) => "NOT IN KEYRING",
                None => "fingerprint unknown",
            };
            println!(
                "  Signature: VALID (commit {} fingerprint {})",
                &commit[..8.min(commit.len())],
                fingerprint.as_deref().unwrap_or("unknown")
            );
            println!("  Trust: {}", trust_status);
        }
        GpgVerification::Unsigned { commit } => {
            println!(
                "  Signature: UNSIGNED (commit {})",
                &commit[..8.min(commit.len())]
            );
        }
        GpgVerification::Invalid { commit, reason } => {
            println!(
                "  Signature: INVALID (commit {}): {}",
                &commit[..8.min(commit.len())],
                reason.lines().next().unwrap_or("unknown")
            );
        }
        GpgVerification::NoCommits => {
            println!("  Signature: no commits on locks branch yet");
        }
    }

    // Show agent identity if configured
    if let Ok(Some(agent)) = AgentConfig::load(sync.chainlink_dir()) {
        println!("  Agent: {}", agent.agent_id);
        let my_locks = locks.agent_locks(&agent.agent_id);
        if !my_locks.is_empty() {
            println!(
                "  Your locks: {}",
                my_locks
                    .iter()
                    .map(|id| format_issue_id(*id))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }

    Ok(())
}
