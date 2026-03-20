use anyhow::{bail, Result};
use std::path::Path;

use crate::identity::AgentConfig;
use crate::sync::SyncManager;
use crate::utils::format_issue_id;

/// `chainlink agent init <agent-id> [-d "description"] [--force]`
pub fn init(
    chainlink_dir: &Path,
    agent_id: &str,
    description: Option<&str>,
    force: bool,
) -> Result<()> {
    match AgentConfig::load(chainlink_dir) {
        Ok(Some(_)) if force => {
            println!("Warning: Overwriting existing agent configuration (--force).");
        }
        Ok(Some(_)) => {
            bail!(
                "Agent already configured. Use --force to overwrite, \
                 or delete .chainlink/agent.json to reconfigure."
            );
        }
        Ok(None) => {}
        Err(e) => {
            println!(
                "Warning: Existing agent.json is malformed ({}). Overwriting with new config.",
                e
            );
        }
    }

    let config = AgentConfig::init(chainlink_dir, agent_id, description)?;

    println!("Agent initialized:");
    println!("  ID:      {}", config.agent_id);
    println!("  Machine: {}", config.machine_id);
    if let Some(desc) = &config.description {
        println!("  Description: {}", desc);
    }
    Ok(())
}

/// `chainlink agent status`
pub fn status(chainlink_dir: &Path) -> Result<()> {
    match AgentConfig::load(chainlink_dir)? {
        Some(config) => {
            println!("Agent: {}", config.agent_id);
            println!("Machine: {}", config.machine_id);
            if let Some(desc) = &config.description {
                println!("Description: {}", desc);
            }

            // Show locked issues (best-effort)
            if let Ok(sync) = SyncManager::new(chainlink_dir) {
                let _ = sync.init_cache();
                let _ = sync.fetch();
                if let Ok(locks) = sync.read_locks() {
                    let my_locks = locks.agent_locks(&config.agent_id);
                    if my_locks.is_empty() {
                        println!("Locks: none");
                    } else {
                        println!(
                            "Locks: {}",
                            my_locks
                                .iter()
                                .map(|id| format_issue_id(*id))
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                    }
                }
            }
        }
        None => {
            println!("No agent configured. Run 'chainlink agent init <id>' first.");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_init_creates_config() {
        let dir = tempdir().unwrap();
        let chainlink_dir = dir.path().join(".chainlink");
        std::fs::create_dir_all(&chainlink_dir).unwrap();

        init(&chainlink_dir, "worker-1", Some("Test agent"), false).unwrap();

        let config = AgentConfig::load(&chainlink_dir).unwrap().unwrap();
        assert_eq!(config.agent_id, "worker-1");
        assert_eq!(config.description, Some("Test agent".to_string()));
    }

    #[test]
    fn test_init_rejects_duplicate() {
        let dir = tempdir().unwrap();
        let chainlink_dir = dir.path().join(".chainlink");
        std::fs::create_dir_all(&chainlink_dir).unwrap();

        init(&chainlink_dir, "worker-1", None, false).unwrap();
        let result = init(&chainlink_dir, "worker-2", None, false);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("already configured"));
    }

    #[test]
    fn test_init_force_overwrites() {
        let dir = tempdir().unwrap();
        let chainlink_dir = dir.path().join(".chainlink");
        std::fs::create_dir_all(&chainlink_dir).unwrap();

        init(&chainlink_dir, "worker-1", None, false).unwrap();
        init(&chainlink_dir, "worker-2", Some("New agent"), true).unwrap();

        let config = AgentConfig::load(&chainlink_dir).unwrap().unwrap();
        assert_eq!(config.agent_id, "worker-2");
        assert_eq!(config.description, Some("New agent".to_string()));
    }

    #[test]
    fn test_init_overwrites_malformed_json() {
        let dir = tempdir().unwrap();
        let chainlink_dir = dir.path().join(".chainlink");
        std::fs::create_dir_all(&chainlink_dir).unwrap();

        std::fs::write(chainlink_dir.join("agent.json"), "not valid json").unwrap();

        init(&chainlink_dir, "worker-1", None, false).unwrap();

        let config = AgentConfig::load(&chainlink_dir).unwrap().unwrap();
        assert_eq!(config.agent_id, "worker-1");
    }

    #[test]
    fn test_status_no_config() {
        let dir = tempdir().unwrap();
        let chainlink_dir = dir.path().join(".chainlink");
        std::fs::create_dir_all(&chainlink_dir).unwrap();

        status(&chainlink_dir).unwrap();
    }

    #[test]
    fn test_status_with_config() {
        let dir = tempdir().unwrap();
        let chainlink_dir = dir.path().join(".chainlink");
        std::fs::create_dir_all(&chainlink_dir).unwrap();

        init(&chainlink_dir, "my-agent", Some("My agent"), false).unwrap();
        status(&chainlink_dir).unwrap();
    }
}
