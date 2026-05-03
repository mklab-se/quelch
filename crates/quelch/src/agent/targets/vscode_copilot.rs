//! VS Code Copilot skill bundle target.
//!
//! Writes a skill-form bundle for GitHub Copilot inside VS Code. Uses the
//! `.vscode/mcp.json` configuration format and Copilot's
//! `.github/copilot-instructions.md` for custom instructions.

use std::fs;
use std::path::Path;

use crate::agent::bundle::Bundle;
use crate::agent::error::TargetError;

/// Write a VS Code Copilot skill bundle to `output_dir`.
///
/// Output structure:
/// ```text
/// agent-bundle/
/// ├── README.md
/// ├── .vscode/
/// │   └── mcp.json
/// ├── .github/
/// │   └── copilot-instructions.md
/// └── prompts.md
/// ```
pub fn write(bundle: &Bundle, output_dir: &Path) -> Result<(), TargetError> {
    fs::create_dir_all(output_dir)?;
    fs::create_dir_all(output_dir.join(".vscode"))?;
    fs::create_dir_all(output_dir.join(".github"))?;

    fs::write(output_dir.join("README.md"), readme(bundle))?;
    fs::write(output_dir.join(".vscode/mcp.json"), vscode_mcp_json(bundle))?;
    fs::write(
        output_dir.join(".github/copilot-instructions.md"),
        copilot_instructions(bundle),
    )?;
    fs::write(output_dir.join("prompts.md"), prompts_md(bundle))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Content generators
// ---------------------------------------------------------------------------

fn readme(bundle: &Bundle) -> String {
    format!(
        r#"# Quelch — VS Code Copilot skill bundle

This directory contains the files needed to give GitHub Copilot in VS Code
access to your Quelch MCP server.

## What's here

| File | Purpose |
|---|---|
| `.vscode/mcp.json` | MCP server registration for VS Code Copilot |
| `.github/copilot-instructions.md` | Custom instructions for Copilot |
| `prompts.md` | Example prompts to test the skill |

## Quick start

1. Copy `.vscode/mcp.json` to your project's `.vscode/mcp.json`
   (merge if one already exists).
2. Copy `.github/copilot-instructions.md` to your project's
   `.github/copilot-instructions.md` (or append to the existing file).
3. Set the API key in your environment or VS Code settings:
   ```sh
   export QUELCH_API_KEY="<your-api-key>"
   ```
{}
4. Open VS Code, enable the MCP server in Copilot Chat settings, and try a prompt.

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

fn vscode_mcp_json(bundle: &Bundle) -> String {
    format!(
        r#"{{
  "servers": {{
    "quelch": {{
      "type": "http",
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

fn copilot_instructions(bundle: &Bundle) -> String {
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
        "# Example prompts for VS Code Copilot\n\nAsk these in GitHub Copilot Chat.\n\n{}",
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
        assert!(dir.path().join(".vscode/mcp.json").exists());
        assert!(dir.path().join(".github/copilot-instructions.md").exists());
        assert!(dir.path().join("prompts.md").exists());
    }

    #[test]
    fn vscode_mcp_json_uses_env_var_not_literal_key() {
        let bundle = sample_bundle();
        let json = vscode_mcp_json(&bundle);
        assert!(json.contains("${QUELCH_API_KEY}"));
    }

    #[test]
    fn vscode_mcp_json_is_valid_json() {
        let bundle = sample_bundle();
        let json_str = vscode_mcp_json(&bundle);
        let normalized = json_str.replace("${QUELCH_API_KEY}", "PLACEHOLDER");
        serde_json::from_str::<serde_json::Value>(&normalized)
            .expect(".vscode/mcp.json must be structurally valid JSON");
    }

    #[test]
    fn copilot_instructions_contains_tool_reference() {
        let bundle = sample_bundle();
        let md = copilot_instructions(&bundle);
        assert!(md.contains("list_sources"));
        assert!(md.contains("query"));
    }
}
