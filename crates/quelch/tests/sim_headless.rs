//! Spawns `quelch sim --no-tui` and asserts both the CI exit-code contract
//! and that stdout contains the expected structured-log content.

use assert_cmd::Command;
use std::time::Duration;

#[test]
fn sim_runs_briefly_and_syncs_some_docs() {
    let mut cmd = Command::cargo_bin("quelch").unwrap();
    cmd.arg("sim")
        .arg("--duration")
        .arg("10s")
        .arg("--seed")
        .arg("42")
        .arg("--no-tui")
        .arg("--rate-multiplier")
        .arg("5.0")
        .arg("--assert-docs")
        .arg("5")
        .timeout(Duration::from_secs(60))
        .assert()
        .success();
}

#[test]
fn log_mode_stdout_contains_summary_and_key_phases() {
    let output = Command::cargo_bin("quelch")
        .unwrap()
        .arg("sim")
        .arg("--duration")
        .arg("10s")
        .arg("--seed")
        .arg("42")
        .arg("--no-tui")
        .arg("--rate-multiplier")
        .arg("4.0")
        .arg("-v")
        .timeout(Duration::from_secs(60))
        .output()
        .expect("run quelch sim");

    assert!(
        output.status.success(),
        "sim failed: status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");

    // The summary line is emitted to stdout at the end.
    assert!(
        stdout.contains("docs synced"),
        "expected summary line with 'docs synced' in stdout:\n{stdout}"
    );

    // Structured tracing events should appear in stderr (default fmt output
    // goes to stderr in tracing-subscriber 0.3).
    for phase in ["cycle_started", "source_started", "subsource_started"] {
        assert!(
            combined.contains(phase),
            "expected phase '{phase}' in log output:\n{combined}"
        );
    }
}
