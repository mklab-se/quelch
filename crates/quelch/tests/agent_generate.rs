//! Integration tests for `quelch agent generate`.

use std::path::Path;

// Path to the minimal fixture config used across tests.
const FIXTURE: &str = "tests/fixtures/quelch.minimal.yaml";

fn fixture_path() -> &'static Path {
    Path::new(FIXTURE)
}

// ---------------------------------------------------------------------------
// claude-code target
// ---------------------------------------------------------------------------

#[test]
fn agent_generate_claude_code_creates_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let bundle_dir = dir.path().join("bundle");

    let mut cmd = assert_cmd::Command::cargo_bin("quelch").unwrap();
    cmd.arg("--config")
        .arg(fixture_path())
        .arg("agent")
        .arg("generate")
        .arg("--target")
        .arg("claude-code")
        .arg("--output")
        .arg(&bundle_dir);

    cmd.assert().success();

    assert!(
        bundle_dir.join("README.md").exists(),
        "README.md must exist"
    );
    assert!(
        bundle_dir.join(".claude/skills/quelch/SKILL.md").exists(),
        "SKILL.md must exist"
    );
    assert!(
        bundle_dir.join(".mcp.json").exists(),
        ".mcp.json must exist"
    );

    // Verify SKILL.md has frontmatter.
    let skill_md =
        std::fs::read_to_string(bundle_dir.join(".claude/skills/quelch/SKILL.md")).unwrap();
    assert!(
        skill_md.starts_with("---"),
        "SKILL.md must have YAML frontmatter"
    );
    assert!(skill_md.contains("description:"));
    assert!(skill_md.contains("Quelch"));

    // Verify .mcp.json uses env var reference, not a literal key.
    let mcp_json = std::fs::read_to_string(bundle_dir.join(".mcp.json")).unwrap();
    assert!(mcp_json.contains("${QUELCH_API_KEY}"));
}

// ---------------------------------------------------------------------------
// markdown target
// ---------------------------------------------------------------------------

#[test]
fn agent_generate_markdown_creates_all_files() {
    let dir = tempfile::tempdir().unwrap();
    let bundle_dir = dir.path().join("bundle");

    let mut cmd = assert_cmd::Command::cargo_bin("quelch").unwrap();
    cmd.arg("--config")
        .arg(fixture_path())
        .arg("agent")
        .arg("generate")
        .arg("--target")
        .arg("markdown")
        .arg("--output")
        .arg(&bundle_dir);

    cmd.assert().success();

    for file in &[
        "README.md",
        "connection.md",
        "tools.md",
        "schema.md",
        "howtos.md",
        "agent-prompt.md",
        "skill.md",
        "prompts.md",
    ] {
        assert!(
            bundle_dir.join(file).exists(),
            "expected file to exist: {file}"
        );
    }
}

// ---------------------------------------------------------------------------
// copilot-studio target
// ---------------------------------------------------------------------------

#[test]
fn agent_generate_copilot_studio_creates_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let bundle_dir = dir.path().join("bundle");

    let mut cmd = assert_cmd::Command::cargo_bin("quelch").unwrap();
    cmd.arg("--config")
        .arg(fixture_path())
        .arg("agent")
        .arg("generate")
        .arg("--target")
        .arg("copilot-studio")
        .arg("--output")
        .arg(&bundle_dir);

    cmd.assert().success();

    assert!(bundle_dir.join("README.md").exists());
    assert!(bundle_dir.join("agent-instructions.md").exists());
    assert!(bundle_dir.join("topics/search-jira.yaml").exists());
}

// ---------------------------------------------------------------------------
// codex target
// ---------------------------------------------------------------------------

#[test]
fn agent_generate_codex_creates_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let bundle_dir = dir.path().join("bundle");

    let mut cmd = assert_cmd::Command::cargo_bin("quelch").unwrap();
    cmd.arg("--config")
        .arg(fixture_path())
        .arg("agent")
        .arg("generate")
        .arg("--target")
        .arg("codex")
        .arg("--output")
        .arg(&bundle_dir);

    cmd.assert().success();

    assert!(bundle_dir.join("AGENTS.md").exists());
    assert!(bundle_dir.join("codex-mcp.toml").exists());

    let toml_str = std::fs::read_to_string(bundle_dir.join("codex-mcp.toml")).unwrap();
    assert!(toml_str.contains("${QUELCH_API_KEY}"));
}

// ---------------------------------------------------------------------------
// vscode-copilot target
// ---------------------------------------------------------------------------

#[test]
fn agent_generate_vscode_copilot_creates_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let bundle_dir = dir.path().join("bundle");

    let mut cmd = assert_cmd::Command::cargo_bin("quelch").unwrap();
    cmd.arg("--config")
        .arg(fixture_path())
        .arg("agent")
        .arg("generate")
        .arg("--target")
        .arg("vscode-copilot")
        .arg("--output")
        .arg(&bundle_dir);

    cmd.assert().success();

    assert!(bundle_dir.join(".vscode/mcp.json").exists());
    assert!(bundle_dir.join(".github/copilot-instructions.md").exists());
}

// ---------------------------------------------------------------------------
// copilot-cli target
// ---------------------------------------------------------------------------

#[test]
fn agent_generate_copilot_cli_creates_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let bundle_dir = dir.path().join("bundle");

    let mut cmd = assert_cmd::Command::cargo_bin("quelch").unwrap();
    cmd.arg("--config")
        .arg(fixture_path())
        .arg("agent")
        .arg("generate")
        .arg("--target")
        .arg("copilot-cli")
        .arg("--output")
        .arg(&bundle_dir);

    cmd.assert().success();

    assert!(bundle_dir.join("mcp-server.json").exists());
    assert!(bundle_dir.join("skill.md").exists());

    let json = std::fs::read_to_string(bundle_dir.join("mcp-server.json")).unwrap();
    assert!(json.contains("${QUELCH_API_KEY}"));
}

// ---------------------------------------------------------------------------
// --url override
// ---------------------------------------------------------------------------

#[test]
fn agent_generate_url_override_appears_in_output() {
    let dir = tempfile::tempdir().unwrap();
    let bundle_dir = dir.path().join("bundle");
    let custom_url = "https://my-custom-quelch.example.com";

    let mut cmd = assert_cmd::Command::cargo_bin("quelch").unwrap();
    cmd.arg("--config")
        .arg(fixture_path())
        .arg("agent")
        .arg("generate")
        .arg("--target")
        .arg("markdown")
        .arg("--url")
        .arg(custom_url)
        .arg("--output")
        .arg(&bundle_dir);

    cmd.assert().success();

    let conn_md = std::fs::read_to_string(bundle_dir.join("connection.md")).unwrap();
    assert!(
        conn_md.contains(custom_url),
        "custom URL must appear in connection.md"
    );
}

// ---------------------------------------------------------------------------
// --deployment flag
// ---------------------------------------------------------------------------

#[test]
fn agent_generate_explicit_deployment_flag() {
    let dir = tempfile::tempdir().unwrap();
    let bundle_dir = dir.path().join("bundle");

    let mut cmd = assert_cmd::Command::cargo_bin("quelch").unwrap();
    cmd.arg("--config")
        .arg(fixture_path())
        .arg("agent")
        .arg("generate")
        .arg("--target")
        .arg("markdown")
        .arg("--deployment")
        .arg("mcp")
        .arg("--output")
        .arg(&bundle_dir);

    cmd.assert().success();
    assert!(bundle_dir.join("README.md").exists());
}
