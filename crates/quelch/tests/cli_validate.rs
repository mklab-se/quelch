/// Integration tests for `quelch validate`.
use assert_cmd::Command;

/// `quelch validate` succeeds on a valid minimal config.
#[test]
fn validate_succeeds_on_minimal_config() {
    Command::cargo_bin("quelch")
        .unwrap()
        .arg("--config")
        .arg("tests/fixtures/quelch.minimal.yaml")
        .arg("validate")
        .assert()
        .success();
}

/// `quelch validate` prints "Config is valid." on success.
#[test]
fn validate_prints_valid_message() {
    let output = Command::cargo_bin("quelch")
        .unwrap()
        .arg("--config")
        .arg("tests/fixtures/quelch.minimal.yaml")
        .arg("validate")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Config is valid."),
        "expected 'Config is valid.' in stdout: {stdout}"
    );
}

/// `quelch validate` fails on a missing config file.
#[test]
fn validate_fails_on_missing_config() {
    Command::cargo_bin("quelch")
        .unwrap()
        .arg("--config")
        .arg("tests/fixtures/nonexistent.yaml")
        .arg("validate")
        .assert()
        .failure();
}
