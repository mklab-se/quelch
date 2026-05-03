//! Shared bundle content for agent/skill generation.
//!
//! A [`Bundle`] holds all the content sections that get packaged differently
//! per target: tool reference, schema cheatsheet, how-tos, example prompts,
//! connection details, and the trigger description.

use std::collections::HashMap;

use crate::config::Config;
use crate::config::data_sources::{ResolvedDataSource, resolve as resolve_data_sources};
use crate::mcp::schema::SchemaCatalog;

use super::error::BundleError;

// ---------------------------------------------------------------------------
// Public structs
// ---------------------------------------------------------------------------

/// All generated material for one MCP deployment.
#[derive(Debug)]
pub struct Bundle {
    /// Connection details for the deployed MCP service.
    pub connection: BundleConnection,
    /// Tool reference (markdown).
    pub tool_reference: String,
    /// Schema cheatsheet — only the data sources this deployment exposes.
    pub schema_cheatsheet: String,
    /// Domain how-tos (markdown).
    pub howtos: String,
    /// Example user prompts.
    pub example_prompts: String,
    /// Skill / agent triggering description (used as YAML frontmatter `description`).
    pub trigger_description: &'static str,
}

/// Connection details for the deployed MCP service.
#[derive(Debug, Clone)]
pub struct BundleConnection {
    /// Public URL of the deployed MCP server.
    pub url: String,
    /// Authentication mode used by this deployment.
    pub auth_mode: ConnectionAuthMode,
    /// Key Vault secret URI for the API key, if known (for human operators to fetch from).
    pub api_key_secret_uri: Option<String>,
}

/// Authentication mode for the MCP server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionAuthMode {
    /// Bearer token / API key in `Authorization: Bearer` header.
    ApiKey,
    /// Microsoft Entra ID (OAuth2).
    EntraId,
}

// ---------------------------------------------------------------------------
// Static content
// ---------------------------------------------------------------------------

/// The "use this when…" trigger string embedded in skill manifests.
pub const TRIGGER_DESCRIPTION: &str = "Use when the user asks about Jira issues, Confluence pages, sprints, releases, blockers, \
     sprint planning, or any other enterprise knowledge. Connect to the configured Quelch MCP server.";

const HOWTOS_MD: &str = r#"## How-tos

### Finding issues in a sprint

Use the `query` tool with `data_source: jira_issues` and a filter on `sprint.state: active`.
To narrow to a specific sprint name, add `sprint.name: "Sprint 42"`.

### Searching for a Confluence page

Use the `search` tool with a free-text query. The server performs semantic + keyword hybrid
search across all exposed Confluence pages. To narrow by space, add a `where` filter:
`{"space_key": "ENG"}`.

### Counting issues by assignee

Use the `aggregate` tool:
```json
{
  "tool": "aggregate",
  "data_source": "jira_issues",
  "group_by": "assignee.display_name",
  "count": true,
  "top_groups": 10
}
```

### Finding blocked issues

Use the `query` tool with `where: {"status": "Blocked"}`.

### Sprint velocity / story-point totals

Use the `aggregate` tool with `group_by: sprint.name` and `sum: story_points`.

### Getting a single document by ID

Use the `get` tool with the Jira issue key (e.g. `DO-1234`) or Confluence page ID.

### Listing available data sources

Call the `list_sources` tool with no arguments to see all data sources this deployment exposes,
including their schema and example calls.
"#;

const EXAMPLE_PROMPTS_MD: &str = r#"## Example prompts

- "What Jira issues are in the current sprint for project DO?"
- "Show me all blocked bugs assigned to alice@example.com"
- "How many story points are in the backlog?"
- "List all open epics in the INT project"
- "Find Confluence pages about onboarding updated in the last 30 days"
- "What did we ship in the last release?"
- "Which issues are blocking the release?"
- "Show me all critical bugs not yet resolved"
- "Who has the most open issues right now?"
- "Summarise what the ENG team worked on last sprint"
- "Find all issues linked to epic DO-500"
- "What sprints are planned for Q3?"
- "List Confluence pages in the ARCH space"
- "Are there any high-priority issues with no assignee?"
- "What's the breakdown of issue types in the current sprint?"
- "Show me issues created in the last 7 days"
- "Find all sub-tasks under DO-1234"
"#;

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Build a [`Bundle`] for the named MCP deployment.
///
/// # Errors
/// Returns [`BundleError::DeploymentNotFound`] if the deployment name is not
/// in the config, or [`BundleError::NotMcpDeployment`] if it is not an MCP
/// deployment.
pub fn build(config: &Config, deployment_name: &str) -> Result<Bundle, BundleError> {
    let dep = config
        .deployments
        .iter()
        .find(|d| d.name == deployment_name)
        .ok_or_else(|| BundleError::DeploymentNotFound(deployment_name.to_string()))?;

    if dep.role != crate::config::DeploymentRole::Mcp {
        return Err(BundleError::NotMcpDeployment(deployment_name.to_string()));
    }

    let connection = build_connection(dep)?;
    let exposed = exposed_data_sources(config, dep);
    let schema_catalog = SchemaCatalog::default();

    let tool_reference = render_tool_reference(&exposed, &schema_catalog);
    let schema_cheatsheet = render_schema_cheatsheet(&exposed, &schema_catalog);
    let howtos = HOWTOS_MD.to_string();
    let example_prompts = EXAMPLE_PROMPTS_MD.to_string();

    Ok(Bundle {
        connection,
        tool_reference,
        schema_cheatsheet,
        howtos,
        example_prompts,
        trigger_description: TRIGGER_DESCRIPTION,
    })
}

/// Build a [`Bundle`] with an explicit URL override (useful when the URL isn't
/// stored in config, e.g. after a manual deployment).
pub fn build_with_url(
    config: &Config,
    deployment_name: &str,
    url: String,
) -> Result<Bundle, BundleError> {
    let mut bundle = build(config, deployment_name)?;
    bundle.connection.url = url;
    Ok(bundle)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn build_connection(
    dep: &crate::config::DeploymentConfig,
) -> Result<BundleConnection, BundleError> {
    // Derive a best-effort URL. For Azure container apps we can construct a
    // likely URL from naming conventions; callers can always override it.
    let url = format!("https://{}.azurecontainerapps.io", dep.name);

    let auth_mode = match dep.auth.as_ref().map(|a| &a.mode) {
        Some(crate::config::McpAuthMode::Entra) => ConnectionAuthMode::EntraId,
        _ => ConnectionAuthMode::ApiKey,
    };

    Ok(BundleConnection {
        url,
        auth_mode,
        api_key_secret_uri: None,
    })
}

fn exposed_data_sources(
    config: &Config,
    dep: &crate::config::DeploymentConfig,
) -> HashMap<String, ResolvedDataSource> {
    let expose: std::collections::HashSet<&str> =
        dep.expose.iter().flatten().map(String::as_str).collect();

    let all = resolve_data_sources(config);
    if expose.is_empty() {
        // No explicit expose list — include everything.
        return all;
    }
    all.into_iter()
        .filter(|(name, _)| expose.contains(name.as_str()))
        .collect()
}

// ---------------------------------------------------------------------------
// Renderers
// ---------------------------------------------------------------------------

/// Render the tool reference section, filtered to the tools relevant for the
/// exposed data sources.
fn render_tool_reference(
    exposed: &HashMap<String, ResolvedDataSource>,
    catalog: &SchemaCatalog,
) -> String {
    let has_searchable = exposed.values().any(|ds| {
        catalog
            .lookup(&ds.kind)
            .map(|k| k.searchable)
            .unwrap_or(false)
    });

    let mut md = String::from("## Tool reference\n\n");

    // list_sources — always present
    md.push_str(
        "### `list_sources`\n\n\
         **When to use:** Discover which data sources this deployment exposes and what \
         fields each one has. Call this first if you are unsure what data is available.\n\n\
         ```yaml\n\
         tool: list_sources\n\
         ```\n\n",
    );

    // search — only when at least one searchable source is exposed
    if has_searchable {
        md.push_str(
            "### `search`\n\n\
             **When to use:** Free-text or semantic search across Jira issues and/or \
             Confluence pages. Use for open-ended discovery where you don't know exact \
             field values.\n\n\
             ```yaml\n\
             tool: search\n\
             query: \"<free text>\"\n\
             data_sources: [\"jira_issues\"]   # optional; omit to search all\n\
             where: {}                         # optional structured filter\n\
             top: 25\n\
             ```\n\n",
        );
    }

    // query — always present
    md.push_str(
        "### `query`\n\n\
         **When to use:** Structured, filter-based retrieval when you know exact field \
         values (e.g. `status: \"In Progress\"`, `assignee.email: \"alice@example.com\"`, \
         `sprint.state: \"active\"`). More precise than `search`.\n\n\
         ```yaml\n\
         tool: query\n\
         data_source: jira_issues\n\
         where:\n\
           and:\n\
             - status: [\"To Do\", \"In Progress\"]\n\
             - assignee.email: alice@example.com\n\
         order_by: [{field: updated, dir: desc}]\n\
         top: 50\n\
         ```\n\n",
    );

    // aggregate — always present
    md.push_str(
        "### `aggregate`\n\n\
         **When to use:** Counting, grouping, or summing over a data source. Use for \
         questions like \"how many issues per assignee\" or \"total story points in sprint\".\n\n\
         ```yaml\n\
         tool: aggregate\n\
         data_source: jira_issues\n\
         group_by: status\n\
         count: true\n\
         top_groups: 20\n\
         ```\n\n",
    );

    // get — always present
    md.push_str(
        "### `get`\n\n\
         **When to use:** Fetch a single document by its ID (Jira key or Confluence page ID) \
         to get full detail including all fields.\n\n\
         ```yaml\n\
         tool: get\n\
         id: DO-1234\n\
         data_source: jira_issues\n\
         ```\n\n",
    );

    md
}

/// Render the schema cheatsheet — one section per exposed data source.
fn render_schema_cheatsheet(
    exposed: &HashMap<String, ResolvedDataSource>,
    catalog: &SchemaCatalog,
) -> String {
    let mut md = String::from("## Schema cheatsheet\n\n");

    // Sort by data source name for deterministic output.
    let mut names: Vec<&String> = exposed.keys().collect();
    names.sort();

    for name in names {
        let ds = &exposed[name];
        let Some(kind_info) = catalog.lookup(&ds.kind) else {
            continue;
        };

        md.push_str(&format!("### `{name}` — {}\n\n", kind_info.description));

        // Container backing info
        let containers: Vec<&str> = ds.backed_by.iter().map(|b| b.container.as_str()).collect();
        if containers.len() == 1 {
            md.push_str(&format!(
                "Backed by Cosmos container: `{}`.\n\n",
                containers[0]
            ));
        } else {
            md.push_str("Backed by Cosmos containers: ");
            let listed: Vec<String> = containers.iter().map(|c| format!("`{c}`")).collect();
            md.push_str(&listed.join(", "));
            md.push_str(". The MCP server unifies them; queries return matches across all.\n\n");
        }

        // Fields table
        md.push_str("**Fields:**\n\n");
        md.push_str("| Field | Type | Notes |\n");
        md.push_str("|---|---|---|\n");
        for field in &kind_info.fields {
            let notes = match (&field.r#enum, &field.description) {
                (Some(vals), _) => format!("One of: {}", vals.join(", ")),
                (None, Some(desc)) => desc.clone(),
                (None, None) => String::new(),
            };
            md.push_str(&format!(
                "| `{}` | {} | {} |\n",
                field.field, field.r#type, notes
            ));
        }
        md.push('\n');

        // Examples
        if !kind_info.examples.is_empty() {
            md.push_str("**Example calls:**\n\n");
            for ex in &kind_info.examples {
                md.push_str(&format!("- {}: `{}`\n", ex.description, ex.call));
            }
            md.push('\n');
        }
    }

    md
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Build a sample [`Bundle`] for use in unit tests.
#[cfg(test)]
pub fn sample_bundle() -> Bundle {
    Bundle {
        connection: BundleConnection {
            url: "https://quelch-mcp.example.azurecontainerapps.io".to_string(),
            auth_mode: ConnectionAuthMode::ApiKey,
            api_key_secret_uri: Some(
                "https://my-vault.vault.azure.net/secrets/quelch-api-key".to_string(),
            ),
        },
        tool_reference: render_tool_reference(&sample_exposed(), &SchemaCatalog::default()),
        schema_cheatsheet: render_schema_cheatsheet(&sample_exposed(), &SchemaCatalog::default()),
        howtos: HOWTOS_MD.to_string(),
        example_prompts: EXAMPLE_PROMPTS_MD.to_string(),
        trigger_description: TRIGGER_DESCRIPTION,
    }
}

#[cfg(test)]
fn sample_exposed() -> HashMap<String, ResolvedDataSource> {
    use crate::config::BackedBy;

    let mut map = HashMap::new();
    map.insert(
        "jira_issues".to_string(),
        ResolvedDataSource {
            kind: "jira_issue".to_string(),
            backed_by: vec![BackedBy {
                container: "jira-issues".to_string(),
            }],
        },
    );
    map.insert(
        "confluence_pages".to_string(),
        ResolvedDataSource {
            kind: "confluence_page".to_string(),
            backed_by: vec![BackedBy {
                container: "confluence-pages".to_string(),
            }],
        },
    );
    map
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    const CONFIG_YAML: &str = r#"
azure:
  subscription_id: "sub-test"
  resource_group: "rg-test"
  region: "swedencentral"
cosmos:
  database: "quelch"
openai:
  endpoint: "https://test.openai.azure.com"
  embedding_deployment: "text-embedding-3-large"
  embedding_dimensions: 3072
sources:
  - type: jira
    name: jira-cloud
    url: "https://example.atlassian.net"
    auth:
      email: "u@example.com"
      api_token: "tok"
    projects: ["DO"]
  - type: confluence
    name: confluence-cloud
    url: "https://example.atlassian.net/wiki"
    auth:
      email: "u@example.com"
      api_token: "tok"
    spaces: ["ENG"]
deployments:
  - name: mcp
    role: mcp
    target: azure
    expose:
      - jira_issues
      - confluence_pages
    auth:
      mode: "api_key"
mcp:
  data_sources:
    jira_issues:
      kind: jira_issue
      backed_by:
        - container: jira-issues
    confluence_pages:
      kind: confluence_page
      backed_by:
        - container: confluence-pages
"#;

    fn parse_config() -> Config {
        serde_yaml::from_str(CONFIG_YAML).expect("test config must parse")
    }

    #[test]
    fn build_returns_bundle_for_mcp_deployment() {
        let config = parse_config();
        let bundle = build(&config, "mcp").expect("build must succeed");
        assert!(!bundle.connection.url.is_empty());
        assert!(!bundle.tool_reference.is_empty());
        assert!(!bundle.schema_cheatsheet.is_empty());
    }

    #[test]
    fn build_errors_for_missing_deployment() {
        let config = parse_config();
        let err = build(&config, "nonexistent").unwrap_err();
        assert!(matches!(err, BundleError::DeploymentNotFound(_)));
    }

    #[test]
    fn build_errors_for_non_mcp_role() {
        let yaml = r#"
azure:
  subscription_id: "sub"
  resource_group: "rg"
  region: "swedencentral"
cosmos:
  database: "quelch"
openai:
  endpoint: "https://x.openai.azure.com"
  embedding_deployment: "te"
  embedding_dimensions: 1536
sources: []
deployments:
  - name: ingest
    role: ingest
    target: azure
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let err = build(&config, "ingest").unwrap_err();
        assert!(matches!(err, BundleError::NotMcpDeployment(_)));
    }

    #[test]
    fn tool_reference_includes_all_five_tools() {
        let config = parse_config();
        let bundle = build(&config, "mcp").unwrap();
        assert!(bundle.tool_reference.contains("list_sources"));
        assert!(bundle.tool_reference.contains("search"));
        assert!(bundle.tool_reference.contains("query"));
        assert!(bundle.tool_reference.contains("aggregate"));
        assert!(bundle.tool_reference.contains("get"));
    }

    #[test]
    fn schema_cheatsheet_includes_exposed_sources() {
        let config = parse_config();
        let bundle = build(&config, "mcp").unwrap();
        assert!(bundle.schema_cheatsheet.contains("jira_issues"));
        assert!(bundle.schema_cheatsheet.contains("confluence_pages"));
    }

    #[test]
    fn schema_cheatsheet_has_field_table() {
        let config = parse_config();
        let bundle = build(&config, "mcp").unwrap();
        assert!(
            bundle
                .schema_cheatsheet
                .contains("| Field | Type | Notes |")
        );
        assert!(bundle.schema_cheatsheet.contains("`key`"));
    }

    #[test]
    fn trigger_description_is_non_empty() {
        let config = parse_config();
        let bundle = build(&config, "mcp").unwrap();
        assert!(!bundle.trigger_description.is_empty());
        assert!(bundle.trigger_description.contains("Jira"));
    }

    #[test]
    fn connection_auth_mode_api_key_for_api_key_config() {
        let config = parse_config();
        let bundle = build(&config, "mcp").unwrap();
        assert_eq!(bundle.connection.auth_mode, ConnectionAuthMode::ApiKey);
    }

    #[test]
    fn sample_bundle_is_valid() {
        let bundle = sample_bundle();
        assert!(!bundle.tool_reference.is_empty());
        assert!(!bundle.schema_cheatsheet.is_empty());
    }
}
