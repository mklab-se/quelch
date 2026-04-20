//! TUI snapshot verification. Runs `quelch sim --snapshot-to FILE` and asserts
//! the rendered frames contain everything the redesigned TUI promises — all
//! keyed to destination-side quantities (documents landing in Azure AI
//! Search), not source-side (documents fetched from Jira/Confluence).

use assert_cmd::Command;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn tui_snapshot_contains_spec_mandated_content() {
    let dir = tempdir().unwrap();
    let snap_path = dir.path().join("snap.txt");

    // 30 frames × 500ms ≈ 15s wall time — enough for seed 42 at 4x rate to
    // push at least a handful of docs through the pipeline so the live feed
    // and counters are non-zero in the captured frames.
    let mut cmd = Command::cargo_bin("quelch").unwrap();
    cmd.arg("sim")
        .arg("--snapshot-to")
        .arg(&snap_path)
        .arg("--snapshot-frames")
        .arg("30")
        .arg("--seed")
        .arg("42")
        .arg("--rate-multiplier")
        .arg("4.0")
        .arg("--fault-rate")
        .arg("0.1")
        .timeout(Duration::from_secs(180))
        .assert()
        .success();

    let snap = std::fs::read_to_string(&snap_path).expect("snapshot file");
    assert!(!snap.is_empty(), "snapshot file empty");

    // Header identifies the binary.
    assert!(snap.contains("quelch"), "header: quelch banner missing");

    // Sources pane: destination-side column headings.
    for heading in [
        "Source",
        "Stage",
        "Pushed",
        "Per min",
        "Latest ID",
        "Pushed at",
    ] {
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

    // Live feed pane — the user's explicit ask: show what's been pushed.
    assert!(
        snap.contains("Pushed to Azure AI Search"),
        "live feed header missing"
    );

    // Azure panel: destination-side labels. No "Total requests", no "Latency".
    for label in [
        "Total pushed",
        "Per minute",
        "4xx",
        "5xx",
        "Throttled",
        "Dropped",
    ] {
        assert!(snap.contains(label), "azure label missing: {label}");
    }
    assert!(
        !snap.contains("Total requests"),
        "Total requests label resurfaced (should have been replaced with 'Total pushed')"
    );
    assert!(
        !snap.contains("median"),
        "latency median resurfaced (dropped as not actionable)"
    );

    // Footer: single keybinding line, no duplication.
    let footer_key_hits = snap.matches("sync now").count();
    assert!(
        footer_key_hits >= 1,
        "expected sync-now keybinding in footer"
    );
    assert!(
        footer_key_hits <= 35,
        "footer appears duplicated — {footer_key_hits} occurrences"
    );

    // At least one frame should show engine activity (Stage column populated
    // with fetching/embedding/pushing, OR the header's "Syncing" status).
    assert!(
        snap.contains("Syncing") || snap.contains("fetching") || snap.contains("embed"),
        "expected engine activity (Syncing/fetching/embed) somewhere in the snapshot"
    );
}

#[test]
fn tui_snapshot_azure_chart_renders_axes() {
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
    assert!(
        snap.contains("Documents pushed per second"),
        "chart subtitle missing"
    );
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

    for heading in ["Source", "Stage", "Pushed", "Latest ID"] {
        assert!(
            snap.contains(heading),
            "heading missing at 100x30: {heading}\n{snap}"
        );
    }
    assert!(
        snap.contains("Total pushed"),
        "azure label missing at 100x30"
    );
    for line in snap.lines() {
        assert!(
            line.chars().count() <= 101,
            "line exceeds 100 cols at 100x30:\n{line}"
        );
    }
}
