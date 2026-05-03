//! Claude Code skill bundle target.
//!
//! Writes a skill-form bundle for Claude Code. The agent calls Quelch via MCP
//! using Claude Code's native `.mcp.json` server registration.

use std::fs;
use std::path::Path;

use crate::agent::bundle::Bundle;
use crate::agent::error::TargetError;

/// Write a Claude Code skill bundle to `output_dir`.
///
/// Output structure:
/// ```text
/// agent-bundle/
/// ├── README.md
/// ├── .claude/
/// │   └── skills/
/// │       └── quelch/
/// │           └── SKILL.md
/// ├── .mcp.json
/// └── prompts.md
/// ```
pub fn write(bundle: &Bundle, output_dir: &Path) -> Result<(), TargetError> {
    fs::create_dir_all(output_dir)?;
    fs::create_dir_all(output_dir.join(".claude/skills/quelch"))?;

    fs::write(output_dir.join("README.md"), readme(bundle))?;
    fs::write(
        output_dir.join(".claude/skills/quelch/SKILL.md"),
        skill_md(bundle),
    )?;
    fs::write(output_dir.join(".mcp.json"), mcp_json(bundle))?;
    fs::write(output_dir.join("prompts.md"), prompts_md(bundle))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Content generators
// ---------------------------------------------------------------------------

fn readme(bundle: &Bundle) -> String {
    format!(
        r#"# Quelch — Claude Code skill bundle

This directory contains the files needed to give Claude Code access to your
Quelch MCP server.

## What's here

| File | Purpose |
|---|---|
| `.claude/skills/quelch/SKILL.md` | Skill definition — copy into your project's `.claude/skills/` directory |
| `.mcp.json` | MCP server registration — merge into your project's `.mcp.json` |
| `prompts.md` | Example prompts to test the skill |

## Quick start

1. Copy `.claude/skills/quelch/SKILL.md` into your project's `.claude/skills/quelch/SKILL.md`.
2. Merge the contents of `.mcp.json` into your project's `.mcp.json`
   (or create a new `.mcp.json` at your repo root).
3. Set the `QUELCH_API_KEY` environment variable:
   ```sh
   export QUELCH_API_KEY="<your-api-key>"
   ```
{}
4. Open Claude Code in your project and try one of the prompts in `prompts.md`.

## MCP server

URL: `{}`
"#,
        bundle
            .connection
            .api_key_secret_uri
            .as_deref()
            .map(|uri| format!(
                "   Fetch the key from Key Vault:\n   ```sh\n   az keyvault secret show --id \"{uri}\" --query value -o tsv\n   ```\n"
            ))
            .unwrap_or_default(),
        bundle.connection.url,
    )
}

fn skill_md(bundle: &Bundle) -> String {
    format!(
        r#"---
name: quelch
description: |
  {}
---

# Quelch — enterprise knowledge skill

You have access to a Quelch MCP server. Use these tools to answer questions about
Jira issues, Confluence pages, sprints, releases, and other enterprise data.

{}

{}

{}
"#,
        bundle.trigger_description, bundle.tool_reference, bundle.schema_cheatsheet, bundle.howtos,
    )
}

fn mcp_json(bundle: &Bundle) -> String {
    let server_name = "quelch-mcp";
    format!(
        r#"{{
  "mcpServers": {{
    "{server_name}": {{
      "type": "streamable-http",
      "url": "{}",
      "headers": {{
        "Authorization": "Bearer ${{QUELCH_API_KEY}}"
      }}
    }}
  }}
}}
"#,
        bundle.connection.url,
    )
}

fn prompts_md(bundle: &Bundle) -> String {
    format!(
        "# Example prompts for Claude Code\n\nUse these to test the Quelch skill.\n\n{}",
        bundle.example_prompts,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::bundle::sample_bundle;

    #[test]
    fn writes_expected_file_structure() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = sample_bundle();
        write(&bundle, dir.path()).unwrap();

        assert!(dir.path().join("README.md").exists());
        assert!(dir.path().join(".claude/skills/quelch/SKILL.md").exists());
        assert!(dir.path().join(".mcp.json").exists());
        assert!(dir.path().join("prompts.md").exists());
    }

    #[test]
    fn skill_md_has_yaml_frontmatter() {
        let bundle = sample_bundle();
        let md = skill_md(&bundle);
        assert!(md.starts_with("---\n"));
        assert!(md.contains("description:"));
        assert!(md.contains("Quelch MCP") || md.contains("Jira"));
    }

    #[test]
    fn skill_md_contains_tool_sections() {
        let bundle = sample_bundle();
        let md = skill_md(&bundle);
        assert!(md.contains("list_sources"));
        assert!(md.contains("query"));
        assert!(md.contains("aggregate"));
    }

    #[test]
    fn mcp_json_references_env_var_not_literal_key() {
        let bundle = sample_bundle();
        let json = mcp_json(&bundle);
        assert!(json.contains("${QUELCH_API_KEY}"));
        // The literal key must not appear.
        assert!(!json.contains("my-secret-key"));
    }

    #[test]
    fn mcp_json_contains_server_url() {
        let bundle = sample_bundle();
        let json = mcp_json(&bundle);
        assert!(json.contains(&bundle.connection.url));
    }

    #[test]
    fn mcp_json_is_valid_json() {
        let bundle = sample_bundle();
        let json_str = mcp_json(&bundle);
        // The env var syntax uses ${...} which isn't valid JSON per se, but
        // we verify the structural JSON is parseable if we strip the env var ref.
        let normalized = json_str.replace("${QUELCH_API_KEY}", "PLACEHOLDER");
        serde_json::from_str::<serde_json::Value>(&normalized)
            .expect("mcp.json must be structurally valid JSON");
    }
}
