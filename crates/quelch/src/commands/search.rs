//! `quelch search` — semantic / hybrid search via Azure AI Search.
//!
//! Operator command. Picks the first MCP instance in the config (so that
//! the same `expose:` rules apply that an agent would see), builds the real
//! Azure AI Search adapter, and calls the `search` tool implementation.

use serde_json::Value;

use crate::config::Config;
use crate::config::schema::{InstanceKind, InstanceSpec};
use crate::mcp::expose::ExposeResolver;
use crate::mcp::schema::SchemaCatalog;
use crate::mcp::tools::search::{
    IncludeContent, SearchRequest, SearchToolConfig, run as search_run,
};
use crate::mcp::tools::search_api::{AzureSearchAdapter, SearchApiAdapter};

/// CLI value-enum mirror of [`IncludeContent`].
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum IncludeContentArg {
    Snippet,
    Full,
    AgenticAnswer,
}

impl From<IncludeContentArg> for IncludeContent {
    fn from(a: IncludeContentArg) -> Self {
        match a {
            IncludeContentArg::Snippet => IncludeContent::Snippet,
            IncludeContentArg::Full => IncludeContent::Full,
            IncludeContentArg::AgenticAnswer => IncludeContent::AgenticAnswer,
        }
    }
}

/// Options for `quelch search`.
#[derive(Debug)]
pub struct SearchOptions {
    pub query: String,
    pub data_sources: Option<Vec<String>>,
    pub where_: Option<Value>,
    pub top: usize,
    pub cursor: Option<String>,
    pub include_content: IncludeContentArg,
    pub include_deleted: bool,
    pub json: bool,
}

/// Run `quelch search`.
pub async fn run(config: &Config, options: SearchOptions) -> anyhow::Result<()> {
    let mcp_instance_name = pick_mcp_instance_name(config)?;
    let sliced = crate::config::slice::slice_for_instance(config, &mcp_instance_name)?;

    let mcp = match &sliced
        .instances
        .first()
        .ok_or_else(|| anyhow::anyhow!("slice produced no instances"))?
        .spec
    {
        InstanceSpec::Mcp(m) => m.clone(),
        InstanceSpec::Ingest(_) => unreachable!("pick_mcp_instance_name guarantees mcp"),
    };

    let endpoint = sliced
        .azure
        .search
        .as_ref()
        .map(|s| s.endpoint.clone())
        .ok_or_else(|| anyhow::anyhow!("azure.search.endpoint is required for quelch search"))?;
    let api: Box<dyn SearchApiAdapter> = Box::new(
        AzureSearchAdapter::new(endpoint, "2025-11-01-preview".to_string())
            .map_err(|e| anyhow::anyhow!("search adapter: {e}"))?,
    );

    let expose = ExposeResolver::from_sliced(&sliced, &mcp_instance_name)
        .map_err(|e| anyhow::anyhow!("expose resolver: {e}"))?;
    let schema = SchemaCatalog::default();
    let cfg = SearchToolConfig {
        knowledge_base_name: mcp.knowledge_base.clone(),
        ..SearchToolConfig::default()
    };

    let req = SearchRequest {
        query: options.query,
        data_sources: options.data_sources,
        r#where: options.where_,
        top: options.top,
        cursor: options.cursor,
        include_deleted: options.include_deleted,
        include_content: options.include_content.into(),
    };

    let resp = search_run(api.as_ref(), &expose, &schema, &cfg, req)
        .await
        .map_err(|e| anyhow::anyhow!("search: {e}"))?;

    if options.json {
        let payload = serde_json::to_value(&resp)?;
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("{} hit(s)", resp.total_estimate);
        for hit in &resp.items {
            println!("- [{}] {}: {}", hit.data_source, hit.id, hit.source_link);
            if let Some(s) = &hit.snippet {
                println!("    {s}");
            }
        }
        if let Some(c) = &resp.next_cursor {
            println!("(next cursor: {c})");
        }
    }

    Ok(())
}

/// Pick the first MCP instance, or error if none exists.
fn pick_mcp_instance_name(config: &Config) -> anyhow::Result<String> {
    config
        .instances
        .iter()
        .find(|i| i.kind() == InstanceKind::Mcp)
        .map(|i| i.name.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "quelch search requires an MCP instance in the config (none found in instances:)"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::schema::SchemaCatalog;
    use crate::mcp::tools::search::{self, SearchRequest, SearchToolConfig};
    use crate::mcp::tools::search_api::mock::MockSearchApi;
    use crate::mcp::tools::test_helpers::build_expose;

    #[tokio::test]
    async fn search_dispatches_to_search_tool_with_mock() {
        let api = MockSearchApi::new();
        let expose = build_expose(&[("jira_issues", "jira_issue", "jira-issues")]);
        let schema = SchemaCatalog::default();
        let config = SearchToolConfig {
            disable_agentic: true,
            ..Default::default()
        };

        let req = SearchRequest {
            query: "open bugs".to_string(),
            data_sources: Some(vec!["jira_issues".to_string()]),
            r#where: None,
            top: 10,
            cursor: None,
            include_deleted: false,
            include_content: IncludeContent::Snippet,
        };

        let resp = search::run(&api, &expose, &schema, &config, req)
            .await
            .unwrap();

        assert_eq!(resp.total_estimate, 1);
    }

    #[test]
    fn include_content_arg_converts_correctly() {
        assert!(matches!(
            IncludeContent::from(IncludeContentArg::Snippet),
            IncludeContent::Snippet
        ));
        assert!(matches!(
            IncludeContent::from(IncludeContentArg::Full),
            IncludeContent::Full
        ));
        assert!(matches!(
            IncludeContent::from(IncludeContentArg::AgenticAnswer),
            IncludeContent::AgenticAnswer
        ));
    }
}
