//! GitHub Copilot CLI skill bundle target.
//!
//! Writes a skill-form bundle for the GitHub Copilot CLI (`gh copilot`).

use std::fs;
use std::path::Path;

use crate::agent::bundle::Bundle;
use crate::agent::error::TargetError;

/// Write a GitHub Copilot CLI skill bundle to `output_dir`.
///
/// Output structure:
/// ```text
/// agent-bundle/
/// ├── README.md
/// ├── mcp-server.json
/// ├── skill.md
/// └── prompts.md
/// ```
pub fn write(bundle: &Bundle, output_dir: &Path) -> Result<(), TargetError> {
    fs::create_dir_all(output_dir)?;

    fs::write(output_dir.join("README.md"), readme(bundle))?;
    fs::write(output_dir.join("mcp-server.json"), mcp_server_json(bundle))?;
    fs::write(output_dir.join("skill.md"), skill_md(bundle))?;
    fs::write(output_dir.join("prompts.md"), prompts_md(bundle))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Content generators
// ---------------------------------------------------------------------------

fn readme(bundle: &Bundle) -> String {
    format!(
        r#"# Quelch — GitHub Copilot CLI skill bundle

This directory contains the files needed to give GitHub Copilot CLI access to
your Quelch MCP server.

## What's here

| File | Purpose |
|---|---|
| `mcp-server.json` | MCP server registration for Copilot CLI |
| `skill.md` | Skill instructions (system context) |
| `prompts.md` | Example prompts to test the skill |

## Quick start

1. Register the MCP server:
   ```sh
   gh copilot mcp add quelch --config mcp-server.json
   ```
2. Set the API key:
   ```sh
   export QUELCH_API_KEY="<your-api-key>"
   ```
{}
3. Try a prompt: `gh copilot ask "What issues are in the current sprint?"`

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

fn mcp_server_json(bundle: &Bundle) -> String {
    format!(
        r#"{{
  "name": "quelch",
  "description": "{description}",
  "type": "streamable-http",
  "url": "{url}",
  "headers": {{
    "Authorization": "Bearer ${{QUELCH_API_KEY}}"
  }}
}}
"#,
        description = bundle.trigger_description.replace('"', "\\\""),
        url = bundle.connection.url,
    )
}

fn skill_md(bundle: &Bundle) -> String {
    format!(
        r#"# Quelch — enterprise knowledge

{}

{}

{}

{}
"#,
        bundle.trigger_description, bundle.tool_reference, bundle.schema_cheatsheet, bundle.howtos,
    )
}

fn prompts_md(bundle: &Bundle) -> String {
    format!(
        "# Example prompts for Copilot CLI\n\nUse with `gh copilot ask \"<prompt>\"`.\n\n{}",
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
        assert!(dir.path().join("mcp-server.json").exists());
        assert!(dir.path().join("skill.md").exists());
        assert!(dir.path().join("prompts.md").exists());
    }

    #[test]
    fn mcp_server_json_uses_env_var_not_literal_key() {
        let bundle = sample_bundle();
        let json = mcp_server_json(&bundle);
        assert!(json.contains("${QUELCH_API_KEY}"));
    }

    #[test]
    fn mcp_server_json_contains_url() {
        let bundle = sample_bundle();
        let json = mcp_server_json(&bundle);
        assert!(json.contains(&bundle.connection.url));
    }

    #[test]
    fn mcp_server_json_is_valid_json() {
        let bundle = sample_bundle();
        let json_str = mcp_server_json(&bundle);
        let normalized = json_str.replace("${QUELCH_API_KEY}", "PLACEHOLDER");
        serde_json::from_str::<serde_json::Value>(&normalized)
            .expect("mcp-server.json must be structurally valid JSON");
    }
}
