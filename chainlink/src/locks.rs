use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// A single issue lock entry in locks.json.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Lock {
    pub agent_id: String,
    #[serde(default)]
    pub branch: Option<String>,
    pub claimed_at: DateTime<Utc>,
    pub signed_by: String,
}

/// Settings embedded in locks.json.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LockSettings {
    #[serde(default = "default_stale_timeout")]
    pub stale_lock_timeout_minutes: u64,
}

fn default_stale_timeout() -> u64 {
    60
}

impl Default for LockSettings {
    fn default() -> Self {
        Self {
            stale_lock_timeout_minutes: default_stale_timeout(),
        }
    }
}

/// The top-level locks.json structure on the coordination branch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocksFile {
    pub version: u32,
    /// Map from issue ID (as string) to Lock.
    pub locks: HashMap<String, Lock>,
    #[serde(default)]
    pub settings: LockSettings,
}

impl LocksFile {
    /// Load and parse a locks.json file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))
    }

    /// Check if a specific issue is locked.
    pub fn is_locked(&self, issue_id: i64) -> bool {
        self.locks.contains_key(&issue_id.to_string())
    }

    /// Get the lock for a specific issue.
    pub fn get_lock(&self, issue_id: i64) -> Option<&Lock> {
        self.locks.get(&issue_id.to_string())
    }

    /// Check if an issue is locked by a specific agent.
    pub fn is_locked_by(&self, issue_id: i64, agent_id: &str) -> bool {
        self.locks
            .get(&issue_id.to_string())
            .map(|l| l.agent_id == agent_id)
            .unwrap_or(false)
    }

    /// List all issue IDs locked by a specific agent.
    pub fn agent_locks(&self, agent_id: &str) -> Vec<i64> {
        self.locks
            .iter()
            .filter(|(_, lock)| lock.agent_id == agent_id)
            .filter_map(|(id, _)| id.parse().ok())
            .collect()
    }

    /// Create an empty locks file.
    pub fn empty() -> Self {
        LocksFile {
            version: 1,
            locks: HashMap::new(),
            settings: LockSettings::default(),
        }
    }

    /// Save the locks file to disk.
    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json).with_context(|| format!("Failed to write {}", path.display()))
    }
}

/// Heartbeat file for an agent (lives at heartbeats/{agent_id}.json).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Heartbeat {
    pub agent_id: String,
    pub last_heartbeat: DateTime<Utc>,
    pub active_issue_id: Option<i64>,
    pub machine_id: String,
}

/// Trust keyring — list of trusted GPG fingerprints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Keyring {
    pub trusted_fingerprints: Vec<String>,
}

impl Keyring {
    /// Load and parse a keyring.json file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))
    }

    /// Check if a fingerprint is trusted.
    pub fn is_trusted(&self, fingerprint: &str) -> bool {
        self.trusted_fingerprints.iter().any(|f| f == fingerprint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_lock() -> Lock {
        Lock {
            agent_id: "worker-1".to_string(),
            branch: Some("feature/auth".to_string()),
            claimed_at: Utc::now(),
            signed_by: "ABCD1234".to_string(),
        }
    }

    fn sample_locks_file() -> LocksFile {
        let mut locks = HashMap::new();
        locks.insert("5".to_string(), sample_lock());
        locks.insert(
            "8".to_string(),
            Lock {
                agent_id: "worker-2".to_string(),
                branch: Some("fix/api-timeout".to_string()),
                claimed_at: Utc::now(),
                signed_by: "EFGH5678".to_string(),
            },
        );
        LocksFile {
            version: 1,
            locks,
            settings: LockSettings::default(),
        }
    }

    #[test]
    fn test_empty_locks() {
        let locks = LocksFile::empty();
        assert_eq!(locks.version, 1);
        assert!(locks.locks.is_empty());
        assert_eq!(locks.settings.stale_lock_timeout_minutes, 60);
    }

    #[test]
    fn test_is_locked() {
        let locks = sample_locks_file();
        assert!(locks.is_locked(5));
        assert!(locks.is_locked(8));
        assert!(!locks.is_locked(1));
    }

    #[test]
    fn test_get_lock() {
        let locks = sample_locks_file();
        let lock = locks.get_lock(5).unwrap();
        assert_eq!(lock.agent_id, "worker-1");
        assert!(locks.get_lock(99).is_none());
    }

    #[test]
    fn test_is_locked_by() {
        let locks = sample_locks_file();
        assert!(locks.is_locked_by(5, "worker-1"));
        assert!(!locks.is_locked_by(5, "worker-2"));
        assert!(locks.is_locked_by(8, "worker-2"));
        assert!(!locks.is_locked_by(99, "worker-1"));
    }

    #[test]
    fn test_agent_locks() {
        let locks = sample_locks_file();
        let mut w1 = locks.agent_locks("worker-1");
        w1.sort();
        assert_eq!(w1, vec![5]);
        let mut w2 = locks.agent_locks("worker-2");
        w2.sort();
        assert_eq!(w2, vec![8]);
        assert!(locks.agent_locks("nobody").is_empty());
    }

    #[test]
    fn test_load_and_save_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("locks.json");
        let locks = sample_locks_file();
        let json = serde_json::to_string_pretty(&locks).unwrap();
        std::fs::write(&path, json).unwrap();

        let loaded = LocksFile::load(&path).unwrap();
        assert_eq!(loaded.version, locks.version);
        assert_eq!(loaded.locks.len(), locks.locks.len());
        assert_eq!(
            loaded.settings.stale_lock_timeout_minutes,
            locks.settings.stale_lock_timeout_minutes
        );
    }

    #[test]
    fn test_default_settings() {
        let settings = LockSettings::default();
        assert_eq!(settings.stale_lock_timeout_minutes, 60);
    }

    #[test]
    fn test_keyring_is_trusted() {
        let keyring = Keyring {
            trusted_fingerprints: vec!["AAAA1111".to_string(), "BBBB2222".to_string()],
        };
        assert!(keyring.is_trusted("AAAA1111"));
        assert!(keyring.is_trusted("BBBB2222"));
        assert!(!keyring.is_trusted("CCCC3333"));
    }

    #[test]
    fn test_heartbeat_json_roundtrip() {
        let hb = Heartbeat {
            agent_id: "worker-1".to_string(),
            last_heartbeat: Utc::now(),
            active_issue_id: Some(42),
            machine_id: "my-host".to_string(),
        };
        let json = serde_json::to_string(&hb).unwrap();
        let parsed: Heartbeat = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.agent_id, hb.agent_id);
        assert_eq!(parsed.active_issue_id, Some(42));
    }
}
