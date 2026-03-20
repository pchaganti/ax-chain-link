use std::process::Command;
use std::thread;

use super::harness::SmokeHarness;

/// Concurrent creates should not corrupt the database.
#[test]
fn test_concurrent_creates_10() {
    let h = SmokeHarness::new();
    let bin = h.chainlink_bin.clone();
    let dir = h.temp_dir.path().to_path_buf();

    let handles: Vec<_> = (0..10)
        .map(|i| {
            let bin = bin.clone();
            let dir = dir.clone();
            thread::spawn(move || {
                let output = Command::new(&bin)
                    .current_dir(&dir)
                    .args(["create", &format!("Concurrent issue {}", i)])
                    .output()
                    .expect("failed to execute chainlink");
                output.status.success()
            })
        })
        .collect();

    let mut successes = 0u32;
    for handle in handles {
        if handle.join().expect("thread panicked") {
            successes += 1;
        }
    }

    // At least one create should succeed; ideally all 10.
    assert!(
        successes >= 1,
        "At least one concurrent create should succeed, got 0",
    );

    // DB should be consistent — listing should work
    let result = h.run_ok(&["list", "-s", "all"]);
    assert!(result.success);
}

/// Concurrent reads should never fail.
#[test]
fn test_concurrent_reads() {
    let h = SmokeHarness::new();

    // Create some issues first
    for i in 0..5 {
        h.run_ok(&["create", &format!("Issue {}", i)]);
    }

    let bin = h.chainlink_bin.clone();
    let dir = h.temp_dir.path().to_path_buf();

    let handles: Vec<_> = (0..10)
        .map(|_| {
            let bin = bin.clone();
            let dir = dir.clone();
            thread::spawn(move || {
                let output = Command::new(&bin)
                    .current_dir(&dir)
                    .args(["list", "-s", "all"])
                    .output()
                    .expect("failed to execute chainlink");
                output.status.success()
            })
        })
        .collect();

    for handle in handles {
        assert!(
            handle.join().expect("thread panicked"),
            "Concurrent reads should always succeed"
        );
    }
}

/// Mixed concurrent reads and writes should not corrupt the database.
#[test]
fn test_concurrent_mixed_operations() {
    let h = SmokeHarness::new();

    // Seed with some issues
    for i in 0..3 {
        h.run_ok(&["create", &format!("Seed issue {}", i)]);
    }

    let bin = h.chainlink_bin.clone();
    let dir = h.temp_dir.path().to_path_buf();

    let mut handles = Vec::new();

    // 5 writers
    for i in 0..5 {
        let bin = bin.clone();
        let dir = dir.clone();
        handles.push(thread::spawn(move || {
            let output = Command::new(&bin)
                .current_dir(&dir)
                .args(["create", &format!("Concurrent write {}", i)])
                .output()
                .expect("failed to execute chainlink");
            output.status.success()
        }));
    }

    // 5 readers
    for _ in 0..5 {
        let bin = bin.clone();
        let dir = dir.clone();
        handles.push(thread::spawn(move || {
            let output = Command::new(&bin)
                .current_dir(&dir)
                .args(["list", "-s", "all"])
                .output()
                .expect("failed to execute chainlink");
            output.status.success()
        }));
    }

    let mut write_successes = 0u32;
    let mut read_successes = 0u32;
    for (i, handle) in handles.into_iter().enumerate() {
        if handle.join().expect("thread panicked") {
            if i < 5 {
                write_successes += 1;
            } else {
                read_successes += 1;
            }
        }
    }

    assert!(
        write_successes >= 1,
        "At least one concurrent write should succeed"
    );
    assert!(
        read_successes >= 1,
        "At least one concurrent read should succeed"
    );

    // DB should still be consistent after all operations
    let result = h.run_ok(&["list", "-s", "all"]);
    assert!(result.success);
}
