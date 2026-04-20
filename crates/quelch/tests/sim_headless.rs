//! Spawns `quelch sim` as a child process and asserts the CI-contract.

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
