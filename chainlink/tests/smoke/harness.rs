#![allow(dead_code)]

use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Result of running a chainlink CLI command.
#[derive(Debug)]
pub struct CmdResult {
    pub success: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CmdResult {
    pub fn stdout_contains(&self, expected: &str) -> bool {
        self.stdout.contains(expected)
    }

    pub fn stderr_contains(&self, expected: &str) -> bool {
        self.stderr.contains(expected)
    }
}

/// Isolated test environment for smoke-testing the chainlink CLI.
pub struct SmokeHarness {
    pub temp_dir: TempDir,
    pub chainlink_bin: PathBuf,
}

impl SmokeHarness {
    /// Create a fully initialised test environment.
    pub fn new() -> Self {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let bin = PathBuf::from(env!("CARGO_BIN_EXE_chainlink"));

        let harness = SmokeHarness {
            temp_dir,
            chainlink_bin: bin,
        };

        // Run chainlink init
        let result = harness.run(&["init"]);
        assert!(
            result.success,
            "chainlink init failed: {}",
            result.stderr
        );

        harness
    }

    /// Create a harness without running `chainlink init`.
    pub fn new_bare() -> Self {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let bin = PathBuf::from(env!("CARGO_BIN_EXE_chainlink"));
        SmokeHarness {
            temp_dir,
            chainlink_bin: bin,
        }
    }

    /// Run a chainlink CLI command and return the full result.
    pub fn run(&self, args: &[&str]) -> CmdResult {
        let output = Command::new(&self.chainlink_bin)
            .current_dir(self.temp_dir.path())
            .args(args)
            .output()
            .expect("failed to execute chainlink");

        CmdResult {
            success: output.status.success(),
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        }
    }

    /// Run a chainlink CLI command and assert it succeeds.
    pub fn run_ok(&self, args: &[&str]) -> CmdResult {
        let result = self.run(args);
        assert!(
            result.success,
            "expected chainlink {:?} to succeed but got exit code {}.\nstdout: {}\nstderr: {}",
            args, result.exit_code, result.stdout, result.stderr,
        );
        result
    }

    /// Run a chainlink CLI command and assert it fails.
    pub fn run_err(&self, args: &[&str]) -> CmdResult {
        let result = self.run(args);
        assert!(
            !result.success,
            "expected chainlink {:?} to fail but it succeeded.\nstdout: {}\nstderr: {}",
            args, result.stdout, result.stderr,
        );
        result
    }

    /// Path to the `.chainlink/` directory.
    pub fn chainlink_dir(&self) -> PathBuf {
        self.temp_dir.path().join(".chainlink")
    }

    /// Path to the SQLite database.
    pub fn db_path(&self) -> PathBuf {
        self.chainlink_dir().join("issues.db")
    }
}

/// Assert that `result.stdout` contains `expected`.
pub fn assert_stdout_contains(result: &CmdResult, expected: &str) {
    assert!(
        result.stdout_contains(expected),
        "expected stdout to contain {:?} but got:\n{}",
        expected,
        result.stdout,
    );
}

/// Assert that `result.stderr` contains `expected`.
pub fn assert_stderr_contains(result: &CmdResult, expected: &str) {
    assert!(
        result.stderr_contains(expected),
        "expected stderr to contain {:?} but got:\n{}",
        expected,
        result.stderr,
    );
}
