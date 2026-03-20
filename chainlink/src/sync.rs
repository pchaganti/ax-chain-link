use anyhow::{bail, Context, Result};
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::identity::AgentConfig;
use crate::locks::{Heartbeat, Keyring, LocksFile};

/// Directory name under .chainlink for the hub cache worktree.
const HUB_CACHE_DIR: &str = ".locks-cache";

/// The coordination branch name.
const HUB_BRANCH: &str = "chainlink/locks";

/// Maximum number of local commits ahead of remote before bailing.
const MAX_DIVERGENCE: usize = 10;

/// GPG signature verification result.
#[derive(Debug, Clone, PartialEq)]
pub enum GpgVerification {
    /// Commit is signed with a valid GPG signature.
    Valid {
        commit: String,
        fingerprint: Option<String>,
    },
    /// Commit exists but is not signed.
    Unsigned { commit: String },
    /// Commit has an invalid or untrusted signature.
    Invalid { commit: String, reason: String },
    /// No commits exist on the branch.
    NoCommits,
}

/// Read the configured tracker remote name from `.chainlink/hook-config.json`.
///
/// Returns the value of `tracker_remote` if set, otherwise `"origin"`.
pub fn read_tracker_remote(chainlink_dir: &Path) -> String {
    let config_path = chainlink_dir.join("hook-config.json");
    let configured = std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        .and_then(|v| {
            v.get("tracker_remote")
                .and_then(|r| r.as_str().map(|s| s.to_string()))
        });

    if let Some(remote) = configured {
        return remote;
    }

    // Warn once when falling back to "origin".
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !WARNED.swap(true, Ordering::Relaxed) {
        tracing::warn!(
            "no tracker_remote configured in {}, defaulting to \"origin\"",
            config_path.display()
        );
    }

    "origin".to_string()
}

/// Manages synchronization with the `chainlink/locks` coordination branch.
///
/// Uses a git worktree at `.chainlink/.locks-cache/` to avoid disturbing
/// the user's working tree.
pub struct SyncManager {
    /// Path to the .chainlink directory.
    chainlink_dir: PathBuf,
    /// Path to .chainlink/.locks-cache (worktree of chainlink/locks branch).
    cache_dir: PathBuf,
    /// The repo root (parent of .chainlink).
    repo_root: PathBuf,
    /// Git remote name (from config, defaults to "origin").
    remote: String,
}

impl SyncManager {
    /// Create a new SyncManager for the given .chainlink directory.
    pub fn new(chainlink_dir: &Path) -> Result<Self> {
        let repo_root = chainlink_dir
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Cannot determine repo root from .chainlink dir"))?
            .to_path_buf();

        let cache_dir = chainlink_dir.join(HUB_CACHE_DIR);
        let remote = read_tracker_remote(chainlink_dir);

        Ok(SyncManager {
            chainlink_dir: chainlink_dir.to_path_buf(),
            cache_dir,
            repo_root,
            remote,
        })
    }

    /// Get the path to the .chainlink directory.
    pub fn chainlink_dir(&self) -> &Path {
        &self.chainlink_dir
    }

    /// Check if the cache directory is initialized.
    pub fn is_initialized(&self) -> bool {
        self.cache_dir.exists()
    }

    /// Get the path to the cache directory.
    pub fn cache_path(&self) -> &Path {
        &self.cache_dir
    }

    /// Get the configured git remote name.
    pub fn remote(&self) -> &str {
        &self.remote
    }

    // --- Git helpers ---

    fn cache_path_str(&self) -> String {
        self.cache_dir.to_string_lossy().to_string()
    }

    fn git_in_repo(&self, args: &[&str]) -> Result<std::process::Output> {
        let output = Command::new("git")
            .current_dir(&self.repo_root)
            .args(args)
            .output()
            .with_context(|| format!("Failed to run git {:?}", args))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("git {:?} failed: {}", args, stderr);
        }
        Ok(output)
    }

    fn git_in_cache(&self, args: &[&str]) -> Result<std::process::Output> {
        let output = Command::new("git")
            .current_dir(&self.cache_dir)
            .args(args)
            .output()
            .with_context(|| format!("Failed to run git {:?} in cache", args))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("git {:?} in cache failed: {}", args, stderr);
        }
        Ok(output)
    }

    /// Ensure the cache worktree has a git identity configured.
    fn ensure_cache_git_identity(&self) -> Result<()> {
        let has_email = Command::new("git")
            .current_dir(&self.cache_dir)
            .args(["config", "user.email"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !has_email {
            let _ = Command::new("git")
                .current_dir(&self.cache_dir)
                .args(["config", "--local", "user.email", "chainlink@localhost"])
                .output();
            let _ = Command::new("git")
                .current_dir(&self.cache_dir)
                .args(["config", "--local", "user.name", "chainlink"])
                .output();
        }
        Ok(())
    }

    /// Count how many commits the local hub branch is ahead of the remote.
    fn count_unpushed_commits(&self) -> usize {
        let remote_ref = format!("{}/{}", self.remote, HUB_BRANCH);
        let range = format!("{}..HEAD", remote_ref);
        match self.git_in_cache(&["rev-list", "--count", &range]) {
            Ok(output) => String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse::<usize>()
                .unwrap_or(0),
            Err(_) => 0,
        }
    }

    /// Check if local has diverged too far from remote and bail if so.
    fn check_divergence(&self) -> Result<()> {
        let ahead = self.count_unpushed_commits();
        if ahead > MAX_DIVERGENCE {
            bail!(
                "Hub branch has diverged: {} local commits ahead of remote \
                 (threshold: {}). Resolve manually with: cd {} && git log --oneline {}/{}..HEAD",
                ahead,
                MAX_DIVERGENCE,
                self.cache_dir.display(),
                self.remote,
                HUB_BRANCH
            );
        }
        Ok(())
    }

    // --- Cache initialization ---

    /// Initialize the hub cache directory.
    ///
    /// If the `chainlink/locks` branch exists on the remote, fetches it and
    /// creates a worktree. If not, creates an orphan branch with an empty
    /// locks.json.
    pub fn init_cache(&self) -> Result<()> {
        if self.cache_dir.exists() {
            return Ok(());
        }

        // Check if remote branch exists
        let has_remote = self
            .git_in_repo(&["ls-remote", "--heads", &self.remote, HUB_BRANCH])
            .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
            .unwrap_or(false);

        if has_remote {
            // Fetch the remote branch
            self.git_in_repo(&["fetch", &self.remote, HUB_BRANCH])?;

            // Check if a local branch already exists
            let has_local = self
                .git_in_repo(&["rev-parse", "--verify", HUB_BRANCH])
                .is_ok();

            if has_local {
                self.git_in_repo(&["worktree", "add", &self.cache_path_str(), HUB_BRANCH])?;
            } else {
                let remote_ref = format!("{}/{}", self.remote, HUB_BRANCH);
                self.git_in_repo(&[
                    "worktree",
                    "add",
                    "-b",
                    HUB_BRANCH,
                    &self.cache_path_str(),
                    &remote_ref,
                ])?;
            }
        } else {
            // No remote branch — create orphan branch with worktree
            self.git_in_repo(&[
                "worktree",
                "add",
                "--orphan",
                "-b",
                HUB_BRANCH,
                &self.cache_path_str(),
            ])?;

            // Initialize with empty locks.json and directory structure
            let locks = LocksFile::empty();
            locks.save(&self.cache_dir.join("locks.json"))?;
            std::fs::create_dir_all(self.cache_dir.join("heartbeats"))?;
            std::fs::create_dir_all(self.cache_dir.join("trust"))?;

            // Commit the initial state
            self.git_in_cache(&["add", "locks.json"])?;
            self.ensure_cache_git_identity()?;
            self.git_in_cache(&["commit", "-m", "Initialize chainlink/locks branch"])?;
        }

        self.ensure_cache_git_identity()?;
        Ok(())
    }

    // --- Health checks ---

    /// Detect and recover from broken git states in the hub cache worktree.
    pub fn hub_health_check(&self) -> Result<()> {
        if !self.cache_dir.exists() {
            return Ok(());
        }

        let git_dir = match self.git_in_cache(&["rev-parse", "--git-dir"]) {
            Ok(output) => {
                let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let path = PathBuf::from(&raw);
                if path.is_absolute() {
                    path
                } else {
                    self.cache_dir.join(path)
                }
            }
            Err(_) => return Ok(()),
        };

        // Fix 0: Remove index.lock first
        let index_lock = git_dir.join("index.lock");
        if index_lock.exists() {
            tracing::warn!("removing index.lock from hub cache before recovery");
            let _ = std::fs::remove_file(&index_lock);
        }

        // Fix 1: Mid-rebase state
        let rebase_merge = git_dir.join("rebase-merge");
        let rebase_apply = git_dir.join("rebase-apply");
        if rebase_merge.exists() || rebase_apply.exists() {
            tracing::warn!("hub cache is stuck in mid-rebase state, aborting to recover");
            let _ = self.git_in_cache(&["rebase", "--abort"]);
            if rebase_merge.exists() {
                let _ = std::fs::remove_dir_all(&rebase_merge);
            }
            if rebase_apply.exists() {
                let _ = std::fs::remove_dir_all(&rebase_apply);
            }
            if index_lock.exists() {
                let _ = std::fs::remove_file(&index_lock);
            }
        }

        // Fix 2: Detached HEAD
        if self.git_in_cache(&["symbolic-ref", "HEAD"]).is_err() {
            tracing::warn!("hub cache HEAD is detached, re-attaching to {}", HUB_BRANCH);
            if self.git_in_cache(&["checkout", HUB_BRANCH]).is_err() {
                let _ = self.git_in_cache(&["branch", "-f", HUB_BRANCH, "HEAD"]);
                let _ = self.git_in_cache(&["checkout", HUB_BRANCH]);
            }
        }

        Ok(())
    }

    /// Detect and resolve dirty hub cache state.
    fn clean_dirty_state(&self) -> Result<bool> {
        let status = self.git_in_cache(&["status", "--porcelain"]);
        match status {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.trim().is_empty() {
                    return Ok(false);
                }
                if self.git_in_cache(&["add", "-A"]).is_err() {
                    tracing::warn!("git add -A failed in dirty state cleanup, escalating to reset --hard HEAD");
                    self.git_in_cache(&["reset", "--hard", "HEAD"])?;
                    return Ok(true);
                }
                let commit_result = self.git_in_cache(&[
                    "commit",
                    "-m",
                    "sync: auto-stage dirty hub state (recovery)",
                ]);
                match commit_result {
                    Ok(_) => Ok(true),
                    Err(e) => {
                        let err_str = e.to_string();
                        if err_str.contains("nothing to commit")
                            || err_str.contains("no changes added")
                        {
                            Ok(false)
                        } else {
                            Err(e)
                        }
                    }
                }
            }
            Err(_) => Ok(false),
        }
    }

    // --- Fetch ---

    /// Fetch the latest state from remote and integrate changes.
    pub fn fetch(&self) -> Result<()> {
        self.hub_health_check()?;

        let fetch_result = self.git_in_cache(&["fetch", &self.remote, HUB_BRANCH]);
        if let Err(e) = &fetch_result {
            let err_str = e.to_string();
            if err_str.contains("Could not resolve host")
                || err_str.contains("Could not read from remote")
                || err_str.contains("does not appear to be a git repository")
                || err_str.contains("No such remote")
                || err_str.contains("couldn't find remote ref")
            {
                return Ok(());
            }
            fetch_result?;
        }

        // Check for unpushed local commits
        let remote_ref = format!("{}/{}", self.remote, HUB_BRANCH);
        let log_result =
            self.git_in_cache(&["log", &format!("{}..HEAD", remote_ref), "--oneline"]);

        match &log_result {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if !stdout.trim().is_empty() {
                    self.rebase_preserving_local(&remote_ref)?;
                    return Ok(());
                }
            }
            Err(_) => {
                tracing::warn!(
                    "cannot determine unpushed commit count; keeping local state"
                );
                return Ok(());
            }
        }

        // No unpushed commits — safe to reset to match remote
        let reset_result = self.git_in_cache(&["reset", "--hard", &remote_ref]);
        if let Err(e) = &reset_result {
            let err_str = e.to_string();
            if err_str.contains("unknown revision") || err_str.contains("ambiguous argument") {
                return Ok(());
            }
            reset_result?;
        }

        Ok(())
    }

    /// Rebase local unpushed commits on top of the remote ref.
    fn rebase_preserving_local(&self, remote_ref: &str) -> Result<()> {
        self.check_divergence()?;
        self.clean_dirty_state()?;

        let rebase_result = self.git_in_cache(&["rebase", remote_ref]);
        if let Err(e) = &rebase_result {
            let err_str = e.to_string();
            if err_str.contains("unknown revision") || err_str.contains("ambiguous argument") {
                return Ok(());
            }
            if let Err(abort_err) = self.git_in_cache(&["rebase", "--abort"]) {
                tracing::warn!("rebase --abort failed during recovery: {}", abort_err);
            }
            tracing::warn!(
                "rebase onto {} failed; aborted to preserve local commits",
                remote_ref
            );
            return Ok(());
        }

        Ok(())
    }

    // --- Locks ---

    /// Read the current locks file from the cache.
    pub fn read_locks(&self) -> Result<LocksFile> {
        let path = self.cache_dir.join("locks.json");
        if !path.exists() {
            return Ok(LocksFile::empty());
        }
        LocksFile::load(&path)
    }

    /// Read the trust keyring from the cache.
    pub fn read_keyring(&self) -> Result<Keyring> {
        let path = self.cache_dir.join("trust").join("keyring.json");
        if !path.exists() {
            return Ok(Keyring {
                trusted_fingerprints: Vec::new(),
            });
        }
        Keyring::load(&path)
    }

    /// Verify GPG signature on the last commit that touched locks.json.
    pub fn verify_locks_signature(&self) -> Result<GpgVerification> {
        // Find the last commit that modified locks.json
        let log_result = self.git_in_cache(&["log", "-1", "--format=%H", "--", "locks.json"]);
        let commit = match log_result {
            Ok(output) => {
                let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if hash.is_empty() {
                    return Ok(GpgVerification::NoCommits);
                }
                hash
            }
            Err(_) => return Ok(GpgVerification::NoCommits),
        };

        // Try to verify the commit's signature
        let verify_output = Command::new("git")
            .current_dir(&self.cache_dir)
            .args(["verify-commit", "--raw", &commit])
            .output();

        match verify_output {
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("VALIDSIG") {
                    // Extract fingerprint from VALIDSIG line
                    let fingerprint = stderr
                        .lines()
                        .find(|l| l.contains("VALIDSIG"))
                        .and_then(|l| l.split_whitespace().nth(2))
                        .map(|s| s.to_string());
                    Ok(GpgVerification::Valid {
                        commit,
                        fingerprint,
                    })
                } else if stderr.contains("ERRSIG") || stderr.contains("BADSIG") {
                    Ok(GpgVerification::Invalid {
                        commit: commit.clone(),
                        reason: stderr.to_string(),
                    })
                } else if output.status.success() {
                    // verify-commit succeeded but no VALIDSIG — unusual
                    Ok(GpgVerification::Valid {
                        commit,
                        fingerprint: None,
                    })
                } else {
                    // verify-commit failed — commit is unsigned
                    Ok(GpgVerification::Unsigned { commit })
                }
            }
            Err(_) => {
                // GPG not available — can't verify
                Ok(GpgVerification::Unsigned { commit })
            }
        }
    }

    /// Claim a lock on an issue for the given agent.
    ///
    /// Returns `Ok(true)` if newly claimed, `Ok(false)` if already held by self.
    /// Fails if locked by another agent (unless `force` is true for steal).
    pub fn claim_lock(
        &self,
        agent: &AgentConfig,
        issue_id: i64,
        branch: Option<&str>,
        force: bool,
    ) -> Result<bool> {
        for _attempt in 0..3 {
            let mut locks = self.read_locks()?;

            if let Some(existing) = locks.get_lock(issue_id) {
                if existing.agent_id == agent.agent_id {
                    return Ok(false); // Already held by self
                }
                if !force {
                    bail!(
                        "Issue {} is locked by '{}' (claimed {}). \
                         Use 'chainlink locks steal {}' if the lock is stale.",
                        crate::utils::format_issue_id(issue_id),
                        existing.agent_id,
                        existing.claimed_at.format("%Y-%m-%d %H:%M"),
                        issue_id
                    );
                }
            }

            let lock = crate::locks::Lock {
                agent_id: agent.agent_id.clone(),
                branch: branch.map(|s| s.to_string()),
                claimed_at: Utc::now(),
                signed_by: agent.agent_id.clone(),
            };

            locks.locks.insert(issue_id.to_string(), lock);
            locks.save(&self.cache_dir.join("locks.json"))?;

            match self
                .commit_and_push_locks(&format!("{}: claim lock on #{}", agent.agent_id, issue_id))
            {
                Ok(()) => {
                    // Verify our claim survived any rebase during push
                    let verified = LocksFile::load(&self.cache_dir.join("locks.json"))?;
                    match verified.get_lock(issue_id) {
                        Some(lock) if lock.agent_id == agent.agent_id => {
                            return Ok(true);
                        }
                        Some(lock) => {
                            tracing::warn!(
                                "lock claim for issue {} was overwritten by '{}', retrying",
                                crate::utils::format_issue_id(issue_id),
                                lock.agent_id
                            );
                        }
                        None => {
                            tracing::warn!(
                                "lock claim for issue {} was lost during push, retrying",
                                crate::utils::format_issue_id(issue_id)
                            );
                        }
                    }
                }
                Err(e) => {
                    let err_str = e.to_string();
                    if err_str.contains("Push failed after") {
                        if self
                            .git_in_cache(&["pull", "--rebase", &self.remote, HUB_BRANCH])
                            .is_err()
                        {
                            self.hub_health_check()?;
                            self.git_in_cache(&[
                                "pull",
                                "--rebase",
                                &self.remote,
                                HUB_BRANCH,
                            ])?;
                        }
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        bail!(
            "Failed to claim lock on #{} after 3 attempts due to concurrent updates",
            issue_id
        )
    }

    /// Release a lock on an issue.
    ///
    /// Returns `Ok(true)` if released, `Ok(false)` if not locked.
    pub fn release_lock(&self, agent: &AgentConfig, issue_id: i64, force: bool) -> Result<bool> {
        let locks = self.read_locks()?;

        match locks.get_lock(issue_id) {
            None => return Ok(false),
            Some(existing) => {
                if existing.agent_id != agent.agent_id && !force {
                    bail!(
                        "Issue {} is locked by '{}', not by you ('{}').",
                        crate::utils::format_issue_id(issue_id),
                        existing.agent_id,
                        agent.agent_id
                    );
                }
            }
        }

        for _release_attempt in 0..3 {
            let mut current_locks = self.read_locks()?;
            current_locks.locks.remove(&issue_id.to_string());
            current_locks.save(&self.cache_dir.join("locks.json"))?;

            self.commit_and_push_locks(&format!(
                "{}: release lock on #{}",
                agent.agent_id, issue_id
            ))?;

            let verified = LocksFile::load(&self.cache_dir.join("locks.json"))?;
            if verified.get_lock(issue_id).is_none() {
                break;
            }
            tracing::warn!(
                "lock release for issue {} was undone during push, retrying",
                crate::utils::format_issue_id(issue_id)
            );
        }

        Ok(true)
    }

    /// Stage locks.json, commit, and push with rebase-retry.
    fn commit_and_push_locks(&self, message: &str) -> Result<()> {
        self.git_in_cache(&["add", "locks.json"])?;

        let commit_result = self.git_in_cache(&["commit", "-m", message]);
        if let Err(e) = &commit_result {
            let err_str = e.to_string();
            if err_str.contains("nothing to commit") || err_str.contains("no changes added") {
                return Ok(());
            }
            commit_result?;
        }

        for attempt in 0..3 {
            let push_result = self.git_in_cache(&["push", &self.remote, HUB_BRANCH]);
            match push_result {
                Ok(_) => return Ok(()),
                Err(e) => {
                    let err_str = e.to_string();
                    if err_str.contains("Could not resolve host")
                        || err_str.contains("Could not read from remote")
                    {
                        return Ok(()); // Offline — commit is local
                    }
                    if err_str.contains("rejected") || err_str.contains("non-fast-forward") {
                        if attempt < 2 {
                            self.check_divergence()?;
                            if self
                                .git_in_cache(&[
                                    "pull",
                                    "--rebase",
                                    &self.remote,
                                    HUB_BRANCH,
                                ])
                                .is_err()
                            {
                                self.hub_health_check()?;
                                self.git_in_cache(&[
                                    "pull",
                                    "--rebase",
                                    &self.remote,
                                    HUB_BRANCH,
                                ])?;
                            }
                            continue;
                        }
                        bail!("Push failed after 3 retries for locks.json");
                    }
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    // --- Heartbeats ---

    /// Write and push a heartbeat file for this agent.
    pub fn push_heartbeat(
        &self,
        agent: &AgentConfig,
        active_issue_id: Option<i64>,
    ) -> Result<()> {
        let heartbeat = Heartbeat {
            agent_id: agent.agent_id.clone(),
            last_heartbeat: Utc::now(),
            active_issue_id,
            machine_id: agent.machine_id.clone(),
        };

        let hb_dir = self.cache_dir.join("heartbeats");
        std::fs::create_dir_all(&hb_dir)?;

        let filename = format!("{}.json", agent.agent_id);
        let path = hb_dir.join(&filename);
        let json = serde_json::to_string_pretty(&heartbeat)?;
        std::fs::write(&path, json)?;

        self.git_in_cache(&["add", &format!("heartbeats/{}", filename)])?;

        let msg = format!(
            "heartbeat: {} at {}",
            agent.agent_id,
            Utc::now().format("%Y-%m-%dT%H:%M:%SZ")
        );
        let commit_result = self.git_in_cache(&["commit", "-m", &msg]);
        if let Err(e) = &commit_result {
            let err_str = e.to_string();
            if err_str.contains("nothing to commit") || err_str.contains("no changes added") {
                return Ok(());
            }
            commit_result?;
        }

        // Push (best-effort)
        let push_result = self.git_in_cache(&["push", &self.remote, HUB_BRANCH]);
        if let Err(e) = &push_result {
            let err_str = e.to_string();
            if err_str.contains("Could not resolve host")
                || err_str.contains("Could not read from remote")
            {
                tracing::warn!("heartbeat push failed (offline), changes saved locally only");
                return Ok(());
            }
            if err_str.contains("rejected") || err_str.contains("non-fast-forward") {
                self.check_divergence()?;
                self.clean_dirty_state()?;
                if self
                    .git_in_cache(&["pull", "--rebase", &self.remote, HUB_BRANCH])
                    .is_err()
                {
                    self.hub_health_check()?;
                    self.git_in_cache(&["pull", "--rebase", &self.remote, HUB_BRANCH])?;
                }
                if let Err(retry_err) =
                    self.git_in_cache(&["push", &self.remote, HUB_BRANCH])
                {
                    tracing::warn!(
                        "heartbeat push failed after retry: {}",
                        retry_err
                    );
                }
            }
        }

        Ok(())
    }

    /// Read all heartbeat files from the cache.
    pub fn read_heartbeats(&self) -> Result<Vec<Heartbeat>> {
        let dir = self.cache_dir.join("heartbeats");
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut heartbeats = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                let content = std::fs::read_to_string(&path)?;
                if let Ok(hb) = serde_json::from_str::<Heartbeat>(&content) {
                    heartbeats.push(hb);
                }
            }
        }
        Ok(heartbeats)
    }

    /// Find locks that have gone stale (no heartbeat within the timeout).
    pub fn find_stale_locks(&self) -> Result<Vec<(i64, String)>> {
        let locks = self.read_locks()?;
        let heartbeats = self.read_heartbeats()?;
        let timeout =
            chrono::Duration::minutes(locks.settings.stale_lock_timeout_minutes as i64);
        let now = Utc::now();

        let mut stale = Vec::new();
        for (issue_id_str, lock) in &locks.locks {
            let has_fresh_heartbeat = heartbeats.iter().any(|hb| {
                hb.agent_id == lock.agent_id
                    && now
                        .signed_duration_since(hb.last_heartbeat)
                        .max(chrono::Duration::zero())
                        < timeout
            });
            if !has_fresh_heartbeat {
                if let Ok(id) = issue_id_str.parse::<i64>() {
                    stale.push((id, lock.agent_id.clone()));
                }
            }
        }
        Ok(stale)
    }

    /// Find stale locks with their age in minutes.
    pub fn find_stale_locks_with_age(&self) -> Result<Vec<(i64, String, u64)>> {
        let locks = self.read_locks()?;
        let heartbeats = self.read_heartbeats()?;
        let timeout =
            chrono::Duration::minutes(locks.settings.stale_lock_timeout_minutes as i64);
        let now = Utc::now();

        let mut stale = Vec::new();
        for (issue_id_str, lock) in &locks.locks {
            let latest_heartbeat = heartbeats
                .iter()
                .filter(|hb| hb.agent_id == lock.agent_id)
                .map(|hb| hb.last_heartbeat)
                .max();

            let age = match latest_heartbeat {
                Some(hb_time) => now
                    .signed_duration_since(hb_time)
                    .max(chrono::Duration::zero()),
                None => now
                    .signed_duration_since(lock.claimed_at)
                    .max(chrono::Duration::zero()),
            };

            if age >= timeout {
                if let Ok(id) = issue_id_str.parse::<i64>() {
                    stale.push((id, lock.agent_id.clone(), age.num_minutes() as u64));
                }
            }
        }
        Ok(stale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_sync_manager_new() {
        let dir = tempdir().unwrap();
        let chainlink_dir = dir.path().join(".chainlink");
        std::fs::create_dir_all(&chainlink_dir).unwrap();

        let sync = SyncManager::new(&chainlink_dir).unwrap();
        assert!(!sync.is_initialized());
        assert_eq!(sync.remote(), "origin");
    }

    #[test]
    fn test_sync_manager_cache_path() {
        let dir = tempdir().unwrap();
        let chainlink_dir = dir.path().join(".chainlink");
        std::fs::create_dir_all(&chainlink_dir).unwrap();

        let sync = SyncManager::new(&chainlink_dir).unwrap();
        assert!(sync.cache_path().ends_with(".locks-cache"));
    }

    #[test]
    fn test_read_tracker_remote_default() {
        let dir = tempdir().unwrap();
        // No hook-config.json — should default to "origin"
        let remote = read_tracker_remote(dir.path());
        assert_eq!(remote, "origin");
    }

    #[test]
    fn test_read_tracker_remote_configured() {
        let dir = tempdir().unwrap();
        let config = r#"{"tracker_remote": "upstream"}"#;
        std::fs::write(dir.path().join("hook-config.json"), config).unwrap();

        let remote = read_tracker_remote(dir.path());
        assert_eq!(remote, "upstream");
    }

    #[test]
    fn test_read_locks_no_cache() {
        let dir = tempdir().unwrap();
        let chainlink_dir = dir.path().join(".chainlink");
        std::fs::create_dir_all(&chainlink_dir).unwrap();

        let sync = SyncManager::new(&chainlink_dir).unwrap();
        // Cache doesn't exist, so this should return empty
        let locks = sync.read_locks().unwrap();
        assert!(locks.locks.is_empty());
    }

    #[test]
    fn test_read_locks_with_file() {
        let dir = tempdir().unwrap();
        let chainlink_dir = dir.path().join(".chainlink");
        let cache_dir = chainlink_dir.join(".locks-cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        // Write a locks.json into the cache dir
        let locks = LocksFile::empty();
        locks.save(&cache_dir.join("locks.json")).unwrap();

        let sync = SyncManager::new(&chainlink_dir).unwrap();
        let loaded = sync.read_locks().unwrap();
        assert_eq!(loaded.version, 1);
        assert!(loaded.locks.is_empty());
    }

    #[test]
    fn test_read_heartbeats_empty() {
        let dir = tempdir().unwrap();
        let chainlink_dir = dir.path().join(".chainlink");
        let cache_dir = chainlink_dir.join(".locks-cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let sync = SyncManager::new(&chainlink_dir).unwrap();
        let heartbeats = sync.read_heartbeats().unwrap();
        assert!(heartbeats.is_empty());
    }

    #[test]
    fn test_read_heartbeats_with_files() {
        let dir = tempdir().unwrap();
        let chainlink_dir = dir.path().join(".chainlink");
        let cache_dir = chainlink_dir.join(".locks-cache");
        let hb_dir = cache_dir.join("heartbeats");
        std::fs::create_dir_all(&hb_dir).unwrap();

        let hb = Heartbeat {
            agent_id: "worker-1".to_string(),
            last_heartbeat: Utc::now(),
            active_issue_id: Some(5),
            machine_id: "test-host".to_string(),
        };
        let json = serde_json::to_string_pretty(&hb).unwrap();
        std::fs::write(hb_dir.join("worker-1.json"), json).unwrap();

        let sync = SyncManager::new(&chainlink_dir).unwrap();
        let heartbeats = sync.read_heartbeats().unwrap();
        assert_eq!(heartbeats.len(), 1);
        assert_eq!(heartbeats[0].agent_id, "worker-1");
        assert_eq!(heartbeats[0].active_issue_id, Some(5));
    }

    #[test]
    fn test_read_keyring_missing() {
        let dir = tempdir().unwrap();
        let chainlink_dir = dir.path().join(".chainlink");
        let cache_dir = chainlink_dir.join(".locks-cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let sync = SyncManager::new(&chainlink_dir).unwrap();
        let keyring = sync.read_keyring().unwrap();
        assert!(keyring.trusted_fingerprints.is_empty());
    }

    #[test]
    fn test_find_stale_locks_no_heartbeats() {
        let dir = tempdir().unwrap();
        let chainlink_dir = dir.path().join(".chainlink");
        let cache_dir = chainlink_dir.join(".locks-cache");
        std::fs::create_dir_all(&cache_dir).unwrap();

        // Write locks with one entry
        let mut locks = LocksFile::empty();
        locks.locks.insert(
            "5".to_string(),
            crate::locks::Lock {
                agent_id: "worker-1".to_string(),
                branch: None,
                claimed_at: Utc::now() - chrono::Duration::hours(2),
                signed_by: "worker-1".to_string(),
            },
        );
        locks.save(&cache_dir.join("locks.json")).unwrap();

        let sync = SyncManager::new(&chainlink_dir).unwrap();
        let stale = sync.find_stale_locks().unwrap();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0], (5, "worker-1".to_string()));
    }

    #[test]
    fn test_find_stale_locks_fresh_heartbeat() {
        let dir = tempdir().unwrap();
        let chainlink_dir = dir.path().join(".chainlink");
        let cache_dir = chainlink_dir.join(".locks-cache");
        let hb_dir = cache_dir.join("heartbeats");
        std::fs::create_dir_all(&hb_dir).unwrap();

        // Write lock
        let mut locks = LocksFile::empty();
        locks.locks.insert(
            "5".to_string(),
            crate::locks::Lock {
                agent_id: "worker-1".to_string(),
                branch: None,
                claimed_at: Utc::now() - chrono::Duration::hours(2),
                signed_by: "worker-1".to_string(),
            },
        );
        locks.save(&cache_dir.join("locks.json")).unwrap();

        // Write fresh heartbeat
        let hb = Heartbeat {
            agent_id: "worker-1".to_string(),
            last_heartbeat: Utc::now(),
            active_issue_id: Some(5),
            machine_id: "test".to_string(),
        };
        std::fs::write(
            hb_dir.join("worker-1.json"),
            serde_json::to_string(&hb).unwrap(),
        )
        .unwrap();

        let sync = SyncManager::new(&chainlink_dir).unwrap();
        let stale = sync.find_stale_locks().unwrap();
        assert!(stale.is_empty());
    }

    #[test]
    fn test_gpg_verification_debug() {
        let variants = vec![
            GpgVerification::Valid {
                commit: "abc".to_string(),
                fingerprint: Some("FP123".to_string()),
            },
            GpgVerification::Unsigned {
                commit: "def".to_string(),
            },
            GpgVerification::Invalid {
                commit: "ghi".to_string(),
                reason: "bad sig".to_string(),
            },
            GpgVerification::NoCommits,
        ];
        for v in variants {
            let _ = format!("{:?}", v);
        }
    }
}
