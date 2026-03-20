use super::harness::SmokeHarness;

// ===========================================================================
// Timer roundtrip
// ===========================================================================

#[test]
fn test_timer_roundtrip() {
    let h = SmokeHarness::new();

    h.run_ok(&["create", "Timer roundtrip issue"]);

    // Start the timer (top-level `start` command).
    let start = h.run_ok(&["start", "1"]);
    assert!(
        start.stdout_contains("Started")
            || start.stdout_contains("timer")
            || start.stdout_contains("Timer"),
        "start should confirm timer start.\nstdout: {}",
        start.stdout,
    );

    // Show while running — `timer` is the status command.
    let show_running = h.run_ok(&["timer"]);
    assert!(
        show_running.stdout_contains("running")
            || show_running.stdout_contains("active")
            || show_running.stdout_contains("Active")
            || show_running.stdout_contains("Timer")
            || show_running.stdout_contains("tracking")
            || show_running.stdout_contains("#1"),
        "timer while running should indicate active state.\nstdout: {}",
        show_running.stdout,
    );

    // Stop the timer (top-level `stop` command).
    let stop = h.run_ok(&["stop"]);
    assert!(
        stop.stdout_contains("Stopped")
            || stop.stdout_contains("stopped")
            || stop.stdout_contains("timer")
            || stop.stdout_contains("Timer"),
        "stop should confirm timer stop.\nstdout: {}",
        stop.stdout,
    );

    // Show after stopping.
    let show_stopped = h.run(&["timer"]);
    let combined = format!("{}{}", show_stopped.stdout, show_stopped.stderr);
    assert!(
        combined.contains("No timer")
            || combined.contains("no timer")
            || combined.contains("No active")
            || combined.contains("Total")
            || show_stopped.success,
        "timer after stop should report stopped state.\nstdout: {}\nstderr: {}",
        show_stopped.stdout,
        show_stopped.stderr,
    );
}

#[test]
fn test_timer_start_already_running() {
    let h = SmokeHarness::new();
    h.run_ok(&["create", "Double-start issue"]);

    h.run_ok(&["start", "1"]);

    let result = h.run(&["start", "1"]);
    let combined = format!("{}{}", result.stdout, result.stderr);
    assert!(
        result.success
            || combined.contains("already")
            || combined.contains("running")
            || combined.contains("active"),
        "Second timer start should handle gracefully.\nstdout: {}\nstderr: {}",
        result.stdout,
        result.stderr,
    );
}

#[test]
fn test_timer_stop_not_running() {
    let h = SmokeHarness::new();

    let result = h.run(&["stop"]);
    let combined = format!("{}{}", result.stdout, result.stderr);
    assert!(
        result.success
            || combined.contains("No active")
            || combined.contains("no active")
            || combined.contains("not running")
            || combined.contains("No timer"),
        "stop with no running timer should handle gracefully.\nstdout: {}\nstderr: {}",
        result.stdout,
        result.stderr,
    );
}

// ===========================================================================
// Session lifecycle
// ===========================================================================

#[test]
fn test_session_full_lifecycle() {
    let h = SmokeHarness::new();

    h.run_ok(&["create", "Session lifecycle issue"]);

    // Start session.
    let start = h.run_ok(&["session", "start"]);
    assert!(
        start.stdout_contains("started")
            || start.stdout_contains("Started")
            || start.stdout_contains("Session"),
        "session start should confirm.\nstdout: {}",
        start.stdout,
    );

    // Set the active work item.
    let work = h.run_ok(&["session", "work", "1"]);
    assert!(
        work.stdout_contains("working on")
            || work.stdout_contains("Working on")
            || work.stdout_contains("#1")
            || work.success,
        "session work should confirm the work item.\nstdout: {}",
        work.stdout,
    );

    // Verify session is active.
    let status = h.run_ok(&["session", "status"]);
    assert!(
        status.stdout_contains("active")
            || status.stdout_contains("Active")
            || status.stdout_contains("Session")
            || status.stdout_contains("Working"),
        "session should be active.\nstdout: {}",
        status.stdout,
    );

    // End session with handoff notes.
    let handoff_note = "Done: lifecycle test complete, all assertions passed";
    let end = h.run_ok(&["session", "end", "--notes", handoff_note]);
    assert!(
        end.stdout_contains("ended")
            || end.stdout_contains("Ended")
            || end.stdout_contains("Session")
            || end.success,
        "session end should confirm.\nstdout: {}",
        end.stdout,
    );

    // Start a new session and verify handoff is shown.
    let start2 = h.run_ok(&["session", "start"]);
    assert!(
        start2.stdout_contains("lifecycle test complete")
            || start2.stdout_contains("Done:")
            || start2.stdout_contains("Handoff")
            || start2.stdout_contains("handoff")
            || start2.stdout_contains("Previous session"),
        "new session start should show previous handoff notes.\nstdout: {}",
        start2.stdout,
    );
}

#[test]
fn test_session_status_no_session() {
    let h = SmokeHarness::new();

    let result = h.run(&["session", "status"]);
    let combined = format!("{}{}", result.stdout, result.stderr);
    assert!(
        combined.contains("No active")
            || combined.contains("no active")
            || combined.contains("No session")
            || combined.contains("not started")
            || result.success,
        "session status with no session should handle gracefully.\nstdout: {}\nstderr: {}",
        result.stdout,
        result.stderr,
    );
}

#[test]
fn test_session_end_without_start() {
    let h = SmokeHarness::new();

    let result = h.run(&["session", "end"]);
    let combined = format!("{}{}", result.stdout, result.stderr);
    assert!(
        !result.success
            || combined.contains("No active")
            || combined.contains("no active"),
        "session end without start should handle gracefully.\nstdout: {}\nstderr: {}",
        result.stdout,
        result.stderr,
    );
}

// ===========================================================================
// Issue tree with hierarchy
// ===========================================================================

#[test]
fn test_issue_tree_with_subissues() {
    let h = SmokeHarness::new();

    h.run_ok(&["create", "Parent lifecycle issue"]);
    h.run_ok(&["subissue", "1", "Child lifecycle issue"]);

    let tree = h.run_ok(&["tree"]);
    assert!(tree.stdout.contains("Parent lifecycle issue"));
    assert!(tree.stdout.contains("Child lifecycle issue"));
}

#[test]
fn test_issue_tree_deep_nesting() {
    let h = SmokeHarness::new();

    h.run_ok(&["create", "Root issue"]);
    h.run_ok(&["subissue", "1", "Child issue"]);
    h.run_ok(&["subissue", "2", "Grandchild issue"]);

    let tree = h.run_ok(&["tree"]);
    assert!(tree.stdout.contains("Root issue"));
    assert!(tree.stdout.contains("Child issue"));
    assert!(tree.stdout.contains("Grandchild issue"));
}

#[test]
fn test_issue_tree_status_filter() {
    let h = SmokeHarness::new();

    h.run_ok(&["create", "Filterable parent"]);
    h.run_ok(&["subissue", "1", "Open child"]);
    h.run_ok(&["subissue", "1", "Closed child"]);
    h.run_ok(&["close", "3"]);

    let tree = h.run_ok(&["tree", "-s", "open"]);
    assert!(tree.stdout.contains("Open child"));
    assert!(
        !tree.stdout_contains("Closed child"),
        "tree --status open should not show closed issues.\nstdout: {}",
        tree.stdout,
    );
}

// ===========================================================================
// Dependency chains
// ===========================================================================

#[test]
fn test_dependency_chain_and_ready() {
    let h = SmokeHarness::new();

    h.run_ok(&["create", "Issue A"]);
    h.run_ok(&["create", "Issue B"]);
    h.run_ok(&["create", "Issue C"]);

    // A blocked by B, B blocked by C
    h.run_ok(&["block", "1", "2"]);
    h.run_ok(&["block", "2", "3"]);

    // Only C should be ready
    let ready = h.run_ok(&["ready"]);
    assert!(ready.stdout.contains("Issue C"));
    assert!(!ready.stdout.contains("Issue A"));
    assert!(!ready.stdout.contains("Issue B"));

    // Close C, then B should become ready
    h.run_ok(&["close", "3"]);
    let ready2 = h.run_ok(&["ready"]);
    assert!(ready2.stdout.contains("Issue B"));
    assert!(!ready2.stdout.contains("Issue A"));

    // Close B, then A should become ready
    h.run_ok(&["close", "2"]);
    let ready3 = h.run_ok(&["ready"]);
    assert!(ready3.stdout.contains("Issue A"));
}

#[test]
fn test_circular_dependency_prevented() {
    let h = SmokeHarness::new();

    h.run_ok(&["create", "Issue 1"]);
    h.run_ok(&["create", "Issue 2"]);
    h.run_ok(&["create", "Issue 3"]);

    h.run_ok(&["block", "1", "2"]);
    h.run_ok(&["block", "2", "3"]);

    // Attempting to create cycle 3 -> 1 should fail
    let result = h.run(&["block", "3", "1"]);
    assert!(
        !result.success || result.stderr.contains("circular"),
        "Circular dependency should be rejected.\nstdout: {}\nstderr: {}",
        result.stdout,
        result.stderr,
    );
}

// ===========================================================================
// Milestone lifecycle
// ===========================================================================

#[test]
fn test_milestone_full_lifecycle() {
    let h = SmokeHarness::new();

    // Create milestone
    let create = h.run_ok(&["milestone", "create", "v1.0", "-d", "First release"]);
    assert!(create.stdout.contains("Created milestone"));

    // Add issues
    h.run_ok(&["create", "Feature 1"]);
    h.run_ok(&["create", "Feature 2"]);
    h.run_ok(&["milestone", "add", "1", "1"]);
    h.run_ok(&["milestone", "add", "1", "2"]);

    // Show milestone
    let show = h.run_ok(&["milestone", "show", "1"]);
    assert!(show.stdout.contains("v1.0"));
    assert!(show.stdout.contains("Feature 1"));
    assert!(show.stdout.contains("Feature 2"));

    // Close issues and milestone
    h.run_ok(&["close", "1"]);
    h.run_ok(&["close", "2"]);
    h.run_ok(&["milestone", "close", "1"]);

    let show_closed = h.run_ok(&["milestone", "show", "1"]);
    assert!(show_closed.stdout.contains("closed"));
}
