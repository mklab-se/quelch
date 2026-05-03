//! MCP `list_sources` tool — describe all data sources exposed by this deployment.
//!
//! Agents should call `list_sources` first to discover what data sources exist,
//! what fields they have, and what example queries they can use.

use serde::{Deserialize, Serialize};

use crate::mcp::error::McpError;
use crate::mcp::expose::ExposeResolver;
use crate::mcp::schema::{ExampleCall, FieldInfo, SchemaCatalog};

/// Request parameters for `list_sources` (currently empty).
#[derive(Debug, Default, Deserialize)]
pub struct ListSourcesRequest {}

/// Response from `list_sources`.
#[derive(Debug, Serialize)]
pub struct ListSourcesResponse {
    pub data_sources: Vec<DataSourceInfo>,
}

/// Information about a single exposed data source.
#[derive(Debug, Serialize)]
pub struct DataSourceInfo {
    /// Logical data-source name (as passed to other tool calls).
    pub name: String,
    /// Entity kind (e.g. `"jira_issue"`, `"confluence_page"`).
    pub kind: String,
    /// Human-readable description.
    pub description: String,
    /// Whether this source supports the `search` tool.
    pub searchable: bool,
    /// Names of the underlying Cosmos containers.
    pub containers: Vec<String>,
    /// Field schema.
    pub schema: Vec<FieldInfo>,
    /// Example tool calls for agents.
    pub examples: Vec<ExampleCall>,
}

/// Execute the `list_sources` tool.
pub async fn run(
    expose: &ExposeResolver,
    schema: &SchemaCatalog,
) -> Result<ListSourcesResponse, McpError> {
    let mut out = Vec::new();

    for (name, resolved) in expose.list_all() {
        let kind_info = schema.lookup(&resolved.kind).ok_or_else(|| {
            McpError::Internal(format!("schema for kind '{}' not found", resolved.kind))
        })?;

        out.push(DataSourceInfo {
            name: name.clone(),
            kind: resolved.kind.clone(),
            description: kind_info.description.clone(),
            searchable: kind_info.searchable,
            containers: resolved
                .backed_by
                .iter()
                .map(|b| b.container.clone())
                .collect(),
            schema: kind_info.fields.clone(),
            examples: kind_info.examples.clone(),
        });
    }

    // Sort by name for deterministic output
    out.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(ListSourcesResponse { data_sources: out })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::schema::SchemaCatalog;
    use crate::mcp::tools::test_helpers::{build_expose, build_expose_jira_issues};

    #[tokio::test]
    async fn list_sources_returns_only_exposed_data_sources() {
        let expose = build_expose_jira_issues(); // only jira_issues
        let schema = SchemaCatalog::new();
        let resp = run(&expose, &schema).await.unwrap();
        assert_eq!(resp.data_sources.len(), 1);
        assert_eq!(resp.data_sources[0].name, "jira_issues");
    }

    #[tokio::test]
    async fn list_sources_includes_kind_and_schema() {
        let expose = build_expose_jira_issues();
        let schema = SchemaCatalog::new();
        let resp = run(&expose, &schema).await.unwrap();
        let ds = &resp.data_sources[0];
        assert_eq!(ds.kind, "jira_issue");
        assert!(!ds.schema.is_empty(), "schema should have fields");
        // key field should be present
        assert!(ds.schema.iter().any(|f| f.field == "key"));
    }

    #[tokio::test]
    async fn list_sources_marks_searchable() {
        let expose = build_expose_jira_issues();
        let schema = SchemaCatalog::new();
        let resp = run(&expose, &schema).await.unwrap();
        let ds = &resp.data_sources[0];
        // jira_issue is searchable
        assert!(ds.searchable);
    }

    #[tokio::test]
    async fn list_sources_marks_non_searchable() {
        let expose = build_expose(&[("jira_sprints", "jira_sprint", "jira-sprints")]);
        let schema = SchemaCatalog::new();
        let resp = run(&expose, &schema).await.unwrap();
        let ds = &resp.data_sources[0];
        assert!(!ds.searchable);
    }

    #[tokio::test]
    async fn list_sources_sorted_by_name() {
        let expose = build_expose(&[
            ("jira_sprints", "jira_sprint", "jira-sprints"),
            ("jira_issues", "jira_issue", "jira-issues"),
            ("confluence_pages", "confluence_page", "confluence-pages"),
        ]);
        let schema = SchemaCatalog::new();
        let resp = run(&expose, &schema).await.unwrap();
        let names: Vec<&str> = resp.data_sources.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["confluence_pages", "jira_issues", "jira_sprints"]
        );
    }

    #[tokio::test]
    async fn list_sources_includes_containers() {
        let expose = build_expose_jira_issues();
        let schema = SchemaCatalog::new();
        let resp = run(&expose, &schema).await.unwrap();
        let ds = &resp.data_sources[0];
        assert_eq!(ds.containers, vec!["jira-issues"]);
    }

    #[tokio::test]
    async fn list_sources_includes_examples() {
        let expose = build_expose_jira_issues();
        let schema = SchemaCatalog::new();
        let resp = run(&expose, &schema).await.unwrap();
        let ds = &resp.data_sources[0];
        assert!(!ds.examples.is_empty(), "jira_issue should have examples");
    }
}
