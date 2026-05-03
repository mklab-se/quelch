//! OpenAI Codex CLI skill bundle target.
//!
//! Writes a skill-form bundle for the OpenAI Codex CLI. Uses `AGENTS.md` for
//! agent instructions and `codex-mcp.toml` for MCP server registration.

use std::fs;
use std::path::Path;

use crate::agent::bundle::Bundle;
use crate::agent::error::TargetError;

/// Write a Codex CLI skill bundle to `output_dir`.
///
/// Output structure:
/// ```text
/// agent-bundle/
/// ├── README.md
/// ├── AGENTS.md           (or AGENTS.quelch.md if the project already has AGENTS.md)
/// ├── codex-mcp.toml
/// └── prompts.md
/// ```
///
/// Note: the bundle always writes `AGENTS.md`. If the target project already
/// has an `AGENTS.md`, the user should rename this file to `AGENTS.quelch.md`
/// and merge manually — the README explains how.
pub fn write(bundle: &Bundle, output_dir: &Path) -> Result<(), TargetError> {
    fs::create_dir_all(output_dir)?;

    fs::write(output_dir.join("README.md"), readme(bundle))?;
    fs::write(output_dir.join("AGENTS.md"), agents_md(bundle))?;
    fs::write(output_dir.join("codex-mcp.toml"), codex_mcp_toml(bundle))?;
    fs::write(output_dir.join("prompts.md"), prompts_md(bundle))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Content generators
// ---------------------------------------------------------------------------

fn readme(bundle: &Bundle) -> String {
    format!(
        r#"# Quelch — Codex CLI skill bundle

This directory contains the files needed to give the OpenAI Codex CLI access
to your Quelch MCP server.

## What's here

| File | Purpose |
|---|---|
| `AGENTS.md` | Agent instructions for Codex CLI |
| `codex-mcp.toml` | MCP server configuration |
| `prompts.md` | Example prompts to test the skill |

## Quick start

1. Copy `codex-mcp.toml` to your project root (or merge into an existing file).
2. Copy `AGENTS.md` to your project root.
   - **If your project already has an `AGENTS.md`**, rename this file to
     `AGENTS.quelch.md`, then append its contents to your existing `AGENTS.md`.
3. Set the API key:
   ```sh
   export QUELCH_API_KEY="<your-api-key>"
   ```
{}
4. Run Codex: `codex "What issues are in the current sprint?"`

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

fn agents_md(bundle: &Bundle) -> String {
    format!(
        r#"# Agent instructions — Quelch enterprise knowledge

{}

{}

{}

{}
"#,
        bundle.trigger_description, bundle.tool_reference, bundle.schema_cheatsheet, bundle.howtos,
    )
}

fn codex_mcp_toml(bundle: &Bundle) -> String {
    format!(
        r#"[mcp_servers.quelch]
type = "streamable-http"
url = "{}"

[mcp_servers.quelch.headers]
Authorization = "Bearer ${{QUELCH_API_KEY}}"
"#,
        bundle.connection.url,
    )
}

fn prompts_md(bundle: &Bundle) -> String {
    format!(
        "# Example prompts for Codex CLI\n\nUse with `codex \"<prompt>\"`.\n\n{}",
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
        assert!(dir.path().join("AGENTS.md").exists());
        assert!(dir.path().join("codex-mcp.toml").exists());
        assert!(dir.path().join("prompts.md").exists());
    }

    #[test]
    fn codex_mcp_toml_uses_env_var_not_literal_key() {
        let bundle = sample_bundle();
        let toml = codex_mcp_toml(&bundle);
        assert!(toml.contains("${QUELCH_API_KEY}"));
        assert!(!toml.contains("my-secret-key"));
    }

    #[test]
    fn codex_mcp_toml_contains_url() {
        let bundle = sample_bundle();
        let toml = codex_mcp_toml(&bundle);
        assert!(toml.contains(&bundle.connection.url));
    }

    #[test]
    fn codex_mcp_toml_is_parseable_toml() {
        let bundle = sample_bundle();
        let toml_str = codex_mcp_toml(&bundle);
        // Replace env var syntax before parsing (TOML doesn't support ${...})
        let normalized = toml_str.replace("${QUELCH_API_KEY}", "PLACEHOLDER");
        toml::from_str::<toml::Value>(&normalized).expect("codex-mcp.toml must be valid TOML");
    }

    #[test]
    fn agents_md_contains_tool_reference() {
        let bundle = sample_bundle();
        let md = agents_md(&bundle);
        assert!(md.contains("list_sources"));
        assert!(md.contains("query"));
    }

    #[test]
    fn readme_mentions_agents_quelch_fallback() {
        let bundle = sample_bundle();
        let md = readme(&bundle);
        assert!(md.contains("AGENTS.quelch.md"));
    }
}
