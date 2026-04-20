//! TUI snapshot verification. Runs `quelch sim --snapshot-to FILE` and asserts
//! the dumped frames contain everything the redesigned TUI is supposed to show.

use assert_cmd::Command;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn tui_snapshot_contains_spec_mandated_content() {
    let dir = tempdir().unwrap();
    let snap_path = dir.path().join("snap.txt");

    let mut cmd = Command::cargo_bin("quelch").unwrap();
    cmd.arg("sim")
        .arg("--snapshot-to")
        .arg(&snap_path)
        .arg("--snapshot-frames")
        .arg("8")
        .arg("--seed")
        .arg("42")
        .arg("--rate-multiplier")
        .arg("4.0")
        .arg("--fault-rate")
        .arg("0.2")
        .timeout(Duration::from_secs(120))
        .assert()
        .success();

    let snap = std::fs::read_to_string(&snap_path).expect("snapshot file");
    assert!(!snap.is_empty(), "snapshot file empty");

    // Header: identifies the binary and shows a clear status word.
    assert!(snap.contains("quelch"), "header: quelch banner missing");

    // Sources pane: column headings (the main v0.4.0 complaint).
    for heading in ["Source", "Status", "Items", "Rate", "Last item", "Updated"] {
        assert!(
            snap.contains(heading),
            "sources heading missing: {heading}\n{snap}"
        );
    }

    // Subsource rows for the sim's configured projects/spaces.
    for expected in ["sim-jira", "sim-confluence", "QUELCH", "INFRA"] {
        assert!(
            snap.contains(expected),
            "expected subsource row: {expected}"
        );
    }

    // Azure panel: plain-English labels (the second major v0.4.0 complaint).
    for label in [
        "Total requests",
        "Failed (4xx)",
        "Failed (5xx)",
        "Throttled",
        "Latency",
        "median",
    ] {
        assert!(snap.contains(label), "azure label missing: {label}");
    }

    // Footer: single keybinding line, no duplication (v0.4.0 shipped two).
    let footer_key_hits = snap.matches("sync now").count();
    assert!(
        footer_key_hits >= 1,
        "expected sync-now keybinding in footer"
    );
    assert!(
        footer_key_hits <= 10,
        "footer appears duplicated — {} occurrences, expected ≤8 (one per frame)",
        footer_key_hits
    );

    // At least one frame should show engine activity.
    assert!(
        snap.contains("Syncing") || snap.contains("Ready"),
        "expected Syncing or Ready state to appear in snapshot"
    );
}

#[test]
fn tui_snapshot_azure_chart_renders_something() {
    let dir = tempdir().unwrap();
    let snap_path = dir.path().join("snap.txt");

    let mut cmd = Command::cargo_bin("quelch").unwrap();
    cmd.arg("sim")
        .arg("--snapshot-to")
        .arg(&snap_path)
        .arg("--snapshot-frames")
        .arg("6")
        .arg("--seed")
        .arg("7")
        .arg("--rate-multiplier")
        .arg("6.0")
        .timeout(Duration::from_secs(120))
        .assert()
        .success();

    let snap = std::fs::read_to_string(&snap_path).unwrap();
    assert!(snap.contains("-60s"), "chart x-axis label missing");
    assert!(snap.contains("now"), "chart x-axis label missing");
}

#[test]
fn tui_snapshot_renders_at_narrow_terminal_100x30() {
    let dir = tempdir().unwrap();
    let snap_path = dir.path().join("snap.txt");

    let mut cmd = Command::cargo_bin("quelch").unwrap();
    cmd.arg("sim")
        .arg("--snapshot-to")
        .arg(&snap_path)
        .arg("--snapshot-frames")
        .arg("4")
        .arg("--snapshot-width")
        .arg("100")
        .arg("--snapshot-height")
        .arg("30")
        .arg("--seed")
        .arg("42")
        .arg("--rate-multiplier")
        .arg("6.0")
        .timeout(Duration::from_secs(120))
        .assert()
        .success();

    let snap = std::fs::read_to_string(&snap_path).expect("snapshot file");
    assert!(!snap.is_empty());

    // Same content assertions, proving the layout stays intact at 100x30.
    for heading in ["Source", "Status", "Items", "Rate", "Last item"] {
        assert!(
            snap.contains(heading),
            "heading missing at 100x30: {heading}\n{snap}"
        );
    }
    assert!(
        snap.contains("Total requests"),
        "azure label missing at 100x30"
    );
    // No line should exceed 100 chars of content (+ possible trailing newline).
    for line in snap.lines() {
        assert!(
            line.chars().count() <= 101,
            "line exceeds 100 cols at 100x30:\n{line}"
        );
    }
}
