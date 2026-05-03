//! Generic markdown bundle target.
//!
//! Writes a target-agnostic bundle that works for both agent and skill use
//! cases. All content is split into separate markdown files so users can
//! pick what they need.

use std::fs;
use std::path::Path;

use crate::agent::bundle::Bundle;
use crate::agent::error::TargetError;

/// Write a generic markdown bundle to `output_dir`.
///
/// Output structure:
/// ```text
/// agent-bundle/
/// ├── README.md
/// ├── connection.md
/// ├── tools.md
/// ├── schema.md
/// ├── howtos.md
/// ├── agent-prompt.md
/// ├── skill.md
/// └── prompts.md
/// ```
pub fn write(bundle: &Bundle, output_dir: &Path) -> Result<(), TargetError> {
    fs::create_dir_all(output_dir)?;

    fs::write(output_dir.join("README.md"), readme(bundle))?;
    fs::write(output_dir.join("connection.md"), connection_md(bundle))?;
    fs::write(output_dir.join("tools.md"), tools_md(bundle))?;
    fs::write(output_dir.join("schema.md"), schema_md(bundle))?;
    fs::write(output_dir.join("howtos.md"), howtos_md(bundle))?;
    fs::write(output_dir.join("agent-prompt.md"), agent_prompt_md(bundle))?;
    fs::write(output_dir.join("skill.md"), skill_md(bundle))?;
    fs::write(output_dir.join("prompts.md"), prompts_md(bundle))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Content generators
// ---------------------------------------------------------------------------

fn readme(bundle: &Bundle) -> String {
    format!(
        r#"# Quelch — generic markdown bundle

This directory contains all the generated material for integrating with your
Quelch MCP server. Choose the files that suit your agent platform.

## What's here

| File | Purpose |
|---|---|
| `connection.md` | MCP server URL and authentication setup |
| `tools.md` | Reference for all 5 MCP tools |
| `schema.md` | Schema cheatsheet for all exposed data sources |
| `howtos.md` | Domain-specific how-to guides |
| `agent-prompt.md` | System prompt for an agent that uses Quelch |
| `skill.md` | Skill definition with frontmatter (for skill-based platforms) |
| `prompts.md` | Example user prompts |

## MCP server

URL: `{}`

See `connection.md` for authentication details.
"#,
        bundle.connection.url,
    )
}

fn connection_md(bundle: &Bundle) -> String {
    let auth_section = match bundle.connection.auth_mode {
        crate::agent::bundle::ConnectionAuthMode::ApiKey => {
            let kv_note = bundle
                .connection
                .api_key_secret_uri
                .as_deref()
                .map(|uri| format!("\nFetch the API key from Key Vault: `{uri}`"))
                .unwrap_or_default();

            format!(
                r#"## Authentication

Mode: **API key** (Bearer token)

Set the `Authorization` header to `Bearer <your-api-key>` on all requests.{kv_note}

**Never commit the API key to version control.** Store it as an environment variable
(`QUELCH_API_KEY`) or in your platform's secret store."#
            )
        }
        crate::agent::bundle::ConnectionAuthMode::EntraId => r#"## Authentication

Mode: **Microsoft Entra ID** (OAuth2)

Obtain a token from Entra ID and set the `Authorization` header to `Bearer <token>`."#
            .to_string(),
    };

    format!(
        r#"# Connection details

## Server URL

```
{}
```

{}

## Protocol

Quelch implements the [Model Context Protocol](https://modelcontextprotocol.io) over
Streamable HTTP (`POST /mcp`).
"#,
        bundle.connection.url, auth_section,
    )
}

fn tools_md(bundle: &Bundle) -> String {
    format!("# Tool reference\n\n{}", bundle.tool_reference)
}

fn schema_md(bundle: &Bundle) -> String {
    format!("# Schema reference\n\n{}", bundle.schema_cheatsheet)
}

fn howtos_md(bundle: &Bundle) -> String {
    bundle.howtos.clone()
}

fn agent_prompt_md(bundle: &Bundle) -> String {
    format!(
        r#"# Agent system prompt

Paste this into your agent's system-prompt / instructions field.

---

You are an enterprise knowledge assistant powered by Quelch.
You help users query Jira issues, Confluence pages, sprints, releases, and other
enterprise data indexed by the Quelch sync engine.

{}

{}

{}

---
"#,
        bundle.tool_reference, bundle.schema_cheatsheet, bundle.howtos,
    )
}

fn skill_md(bundle: &Bundle) -> String {
    format!(
        r#"---
name: quelch
description: |
  {}
---

# Quelch — enterprise knowledge

{}

{}

{}
"#,
        bundle.trigger_description, bundle.tool_reference, bundle.schema_cheatsheet, bundle.howtos,
    )
}

fn prompts_md(bundle: &Bundle) -> String {
    format!("# Example prompts\n\n{}", bundle.example_prompts,)
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
        assert!(dir.path().join("connection.md").exists());
        assert!(dir.path().join("tools.md").exists());
        assert!(dir.path().join("schema.md").exists());
        assert!(dir.path().join("howtos.md").exists());
        assert!(dir.path().join("agent-prompt.md").exists());
        assert!(dir.path().join("skill.md").exists());
        assert!(dir.path().join("prompts.md").exists());
    }

    #[test]
    fn skill_md_has_frontmatter() {
        let bundle = sample_bundle();
        let md = skill_md(&bundle);
        assert!(md.starts_with("---\n"));
        assert!(md.contains("description:"));
    }

    #[test]
    fn tools_md_contains_all_tools() {
        let bundle = sample_bundle();
        let md = tools_md(&bundle);
        assert!(md.contains("list_sources"));
        assert!(md.contains("search"));
        assert!(md.contains("query"));
        assert!(md.contains("aggregate"));
        assert!(md.contains("get"));
    }

    #[test]
    fn connection_md_does_not_expose_literal_key() {
        let bundle = sample_bundle();
        let md = connection_md(&bundle);
        // The README must reference the env var, not a literal key.
        assert!(md.contains("QUELCH_API_KEY") || md.contains("secret store"));
    }
}
