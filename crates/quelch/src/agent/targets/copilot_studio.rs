//! Copilot Studio agent bundle target.
//!
//! Writes an agent-form bundle suitable for import into Microsoft Copilot Studio.
//! The output is an agent (not a skill): it gets its own instructions and topics
//! that orchestrate tool calls against the Quelch MCP server.

use std::fs;
use std::path::Path;

use crate::agent::bundle::Bundle;
use crate::agent::error::TargetError;

/// Write a Copilot Studio agent bundle to `output_dir`.
///
/// Output structure:
/// ```text
/// agent-bundle/
/// ├── README.md
/// ├── agent-instructions.md
/// ├── topics/
/// │   ├── search-jira.yaml
/// │   ├── search-confluence.yaml
/// │   ├── get-issue.yaml
/// │   ├── sprint-summary.yaml
/// │   └── blockers.yaml
/// ├── connection.md
/// └── prompts.md
/// ```
pub fn write(bundle: &Bundle, output_dir: &Path) -> Result<(), TargetError> {
    fs::create_dir_all(output_dir)?;
    fs::create_dir_all(output_dir.join("topics"))?;

    fs::write(output_dir.join("README.md"), readme(bundle))?;
    fs::write(
        output_dir.join("agent-instructions.md"),
        agent_instructions(bundle),
    )?;
    fs::write(output_dir.join("connection.md"), connection_md(bundle))?;
    fs::write(output_dir.join("prompts.md"), prompts_md(bundle))?;

    // Topics
    for (filename, content) in topics(bundle) {
        fs::write(output_dir.join("topics").join(&filename), content)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Content generators
// ---------------------------------------------------------------------------

fn readme(bundle: &Bundle) -> String {
    format!(
        r#"# Quelch — Copilot Studio Agent Bundle

This directory contains the materials needed to set up a Copilot Studio agent that
queries your Quelch MCP server.

## What's here

| File | Purpose |
|---|---|
| `agent-instructions.md` | System-prompt instructions for the Copilot Studio agent |
| `topics/` | Conversational topics for common query patterns |
| `connection.md` | How to configure the MCP connection in Copilot Studio |
| `prompts.md` | Example user prompts to test the agent |

## Quick start

1. Create a new Copilot Studio agent (or open an existing one).
2. Paste the contents of `agent-instructions.md` into the agent's **Instructions** field.
3. Import each `.yaml` file in `topics/` as a new topic.
4. Follow `connection.md` to configure the MCP connection action.
5. Test the agent using the example prompts in `prompts.md`.

## MCP server

URL: `{}`

See `connection.md` for authentication setup.
"#,
        bundle.connection.url
    )
}

fn agent_instructions(bundle: &Bundle) -> String {
    format!(
        r#"# Agent instructions

You are an enterprise knowledge assistant powered by Quelch.
You help users query Jira issues, Confluence pages, sprints, releases, and other
enterprise data indexed by the Quelch sync engine.

## Your capabilities

You have access to a Quelch MCP server. Use these tools to answer questions:

{}

## How to answer

1. If the user asks about issues, sprints, or project status, use the `query` or `search` tool.
2. If the user asks for counts or summaries, use the `aggregate` tool.
3. If the user asks for a specific issue by key (e.g. "DO-1234"), use the `get` tool.
4. If you are unsure what data is available, call `list_sources` first.
5. Always show the Jira issue key or Confluence page title in your answer.
6. For Jira, link to issues: `[<key>](https://your-jira.atlassian.net/browse/<key>)`.

## Tone

- Be concise and factual.
- Use bullet points for lists of issues.
- If no results are found, say so clearly and suggest a broader query.

{}

{}
"#,
        bundle.tool_reference, bundle.schema_cheatsheet, bundle.howtos,
    )
}

fn connection_md(bundle: &Bundle) -> String {
    let auth_note = match bundle.connection.auth_mode {
        crate::agent::bundle::ConnectionAuthMode::ApiKey => {
            let kv_note = bundle
                .connection
                .api_key_secret_uri
                .as_deref()
                .map(|uri| format!("\nFetch the API key from Key Vault: `{uri}`"))
                .unwrap_or_default();

            format!(
                r#"## Authentication

This deployment uses API key authentication.

Set the `Authorization` header to `Bearer ${{QUELCH_API_KEY}}` on all MCP requests.{kv_note}

In Copilot Studio, configure a **Connection** of type *HTTP with API key* and provide
the key as a header value.  **Never hard-code the key in topic YAML or instructions.**"#
            )
        }
        crate::agent::bundle::ConnectionAuthMode::EntraId => r#"## Authentication

This deployment uses Microsoft Entra ID (OAuth2).

Configure a **Connection** of type *OAuth 2.0 — Azure Active Directory* in Copilot Studio.
The agent will use the signed-in user's identity to authenticate against the MCP server."#
            .to_string(),
    };

    format!(
        r#"# Quelch MCP — Connection guide (Copilot Studio)

## Server URL

```
{}
```

{}

## Testing the connection

Call the `list_sources` action with no parameters.  If the connection is working you
will receive a JSON response listing the available data sources.
"#,
        bundle.connection.url, auth_note,
    )
}

fn prompts_md(bundle: &Bundle) -> String {
    format!(
        "# Example prompts\n\nUse these to test your Copilot Studio agent.\n\n{}",
        bundle.example_prompts
    )
}

/// Generate Copilot Studio topic YAML files.
///
/// Each topic covers a common query pattern and uses MCP tool calls (via
/// Copilot Studio's HTTP action or Power Automate connector) to fulfil the
/// user's intent.
fn topics(bundle: &Bundle) -> Vec<(String, String)> {
    let url = &bundle.connection.url;
    vec![
        ("search-jira.yaml".to_string(), search_jira_topic(url)),
        (
            "search-confluence.yaml".to_string(),
            search_confluence_topic(url),
        ),
        ("get-issue.yaml".to_string(), get_issue_topic(url)),
        ("sprint-summary.yaml".to_string(), sprint_summary_topic(url)),
        ("blockers.yaml".to_string(), blockers_topic(url)),
    ]
}

fn search_jira_topic(url: &str) -> String {
    format!(
        r#"kind: AdaptiveDialog
beginDialog:
  kind: OnRecognizedIntent
  id: main
  intent:
    displayName: Search Jira issues
    triggerQueries:
      - search jira
      - find issues
      - look up issues
      - show me issues about
      - jira issues for
  actions:
    - kind: Question
      id: askQuery
      prompt: What would you like to search for in Jira?
      variable: Topic.SearchQuery

    - kind: SendActivity
      id: callMcp
      activity:
        type: invokeResponse
        value:
          type: http
          method: POST
          url: {url}/mcp
          headers:
            Authorization: Bearer ${{system.mcpApiKey}}
          body: |
            {{
              "jsonrpc": "2.0",
              "method": "tools/call",
              "params": {{
                "name": "search",
                "arguments": {{
                  "query": "${{Topic.SearchQuery}}",
                  "data_sources": ["jira_issues"],
                  "top": 10
                }}
              }},
              "id": 1
            }}
      resultVariable: Topic.SearchResult

    - kind: SendActivity
      id: showResults
      activity: ${{Topic.SearchResult}}
"#,
    )
}

fn search_confluence_topic(url: &str) -> String {
    format!(
        r#"kind: AdaptiveDialog
beginDialog:
  kind: OnRecognizedIntent
  id: main
  intent:
    displayName: Search Confluence pages
    triggerQueries:
      - search confluence
      - find pages
      - look up documentation
      - show me docs about
      - confluence pages for
  actions:
    - kind: Question
      id: askQuery
      prompt: What would you like to search for in Confluence?
      variable: Topic.SearchQuery

    - kind: SendActivity
      id: callMcp
      activity:
        type: invokeResponse
        value:
          type: http
          method: POST
          url: {url}/mcp
          headers:
            Authorization: Bearer ${{system.mcpApiKey}}
          body: |
            {{
              "jsonrpc": "2.0",
              "method": "tools/call",
              "params": {{
                "name": "search",
                "arguments": {{
                  "query": "${{Topic.SearchQuery}}",
                  "data_sources": ["confluence_pages"],
                  "top": 10
                }}
              }},
              "id": 1
            }}
      resultVariable: Topic.SearchResult

    - kind: SendActivity
      id: showResults
      activity: ${{Topic.SearchResult}}
"#,
    )
}

fn get_issue_topic(url: &str) -> String {
    format!(
        r#"kind: AdaptiveDialog
beginDialog:
  kind: OnRecognizedIntent
  id: main
  intent:
    displayName: Get Jira issue
    triggerQueries:
      - get issue
      - show issue
      - tell me about issue
      - details for
      - what is
  actions:
    - kind: Question
      id: askId
      prompt: What is the issue key? (e.g. DO-1234)
      variable: Topic.IssueKey

    - kind: SendActivity
      id: callMcp
      activity:
        type: invokeResponse
        value:
          type: http
          method: POST
          url: {url}/mcp
          headers:
            Authorization: Bearer ${{system.mcpApiKey}}
          body: |
            {{
              "jsonrpc": "2.0",
              "method": "tools/call",
              "params": {{
                "name": "get",
                "arguments": {{
                  "id": "${{Topic.IssueKey}}",
                  "data_source": "jira_issues"
                }}
              }},
              "id": 1
            }}
      resultVariable: Topic.Issue

    - kind: SendActivity
      id: showIssue
      activity: ${{Topic.Issue}}
"#,
    )
}

fn sprint_summary_topic(url: &str) -> String {
    format!(
        r#"kind: AdaptiveDialog
beginDialog:
  kind: OnRecognizedIntent
  id: main
  intent:
    displayName: Sprint summary
    triggerQueries:
      - sprint summary
      - current sprint
      - what's in the sprint
      - sprint status
      - what are we working on
  actions:
    - kind: SendActivity
      id: callMcp
      activity:
        type: invokeResponse
        value:
          type: http
          method: POST
          url: {url}/mcp
          headers:
            Authorization: Bearer ${{system.mcpApiKey}}
          body: |
            {{
              "jsonrpc": "2.0",
              "method": "tools/call",
              "params": {{
                "name": "query",
                "arguments": {{
                  "data_source": "jira_issues",
                  "where": {{"sprint.state": "active"}},
                  "order_by": [{{"field": "status", "dir": "asc"}}],
                  "top": 50
                }}
              }},
              "id": 1
            }}
      resultVariable: Topic.SprintIssues

    - kind: SendActivity
      id: showSprint
      activity: ${{Topic.SprintIssues}}
"#,
    )
}

fn blockers_topic(url: &str) -> String {
    format!(
        r#"kind: AdaptiveDialog
beginDialog:
  kind: OnRecognizedIntent
  id: main
  intent:
    displayName: Find blockers
    triggerQueries:
      - blockers
      - blocked issues
      - what is blocked
      - show blockers
      - which issues are blocked
  actions:
    - kind: SendActivity
      id: callMcp
      activity:
        type: invokeResponse
        value:
          type: http
          method: POST
          url: {url}/mcp
          headers:
            Authorization: Bearer ${{system.mcpApiKey}}
          body: |
            {{
              "jsonrpc": "2.0",
              "method": "tools/call",
              "params": {{
                "name": "query",
                "arguments": {{
                  "data_source": "jira_issues",
                  "where": {{"status": "Blocked"}},
                  "order_by": [{{"field": "priority", "dir": "asc"}}],
                  "top": 50
                }}
              }},
              "id": 1
            }}
      resultVariable: Topic.Blockers

    - kind: SendActivity
      id: showBlockers
      activity: ${{Topic.Blockers}}
"#,
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
        assert!(dir.path().join("agent-instructions.md").exists());
        assert!(dir.path().join("connection.md").exists());
        assert!(dir.path().join("prompts.md").exists());
        assert!(dir.path().join("topics/search-jira.yaml").exists());
        assert!(dir.path().join("topics/search-confluence.yaml").exists());
        assert!(dir.path().join("topics/get-issue.yaml").exists());
        assert!(dir.path().join("topics/sprint-summary.yaml").exists());
        assert!(dir.path().join("topics/blockers.yaml").exists());
    }

    #[test]
    fn agent_instructions_contains_key_sections() {
        let bundle = sample_bundle();
        let instr = agent_instructions(&bundle);
        assert!(instr.contains("list_sources"));
        assert!(instr.contains("query"));
        assert!(instr.contains("aggregate"));
    }

    #[test]
    fn topic_yaml_starts_with_kind() {
        let bundle = sample_bundle();
        let url = &bundle.connection.url;
        let yaml = search_jira_topic(url);
        assert!(yaml.starts_with("kind: AdaptiveDialog"));
    }

    #[test]
    fn topic_yaml_contains_mcp_url() {
        let bundle = sample_bundle();
        let url = &bundle.connection.url;
        let yaml = search_jira_topic(url);
        assert!(yaml.contains(url.as_str()));
    }

    #[test]
    fn connection_md_does_not_contain_literal_key() {
        let bundle = sample_bundle();
        let md = connection_md(&bundle);
        // The actual key must never appear — only the env var reference.
        assert!(!md.contains("my-secret-key"));
        assert!(md.contains("QUELCH_API_KEY") || md.contains("Bearer"));
    }
}
