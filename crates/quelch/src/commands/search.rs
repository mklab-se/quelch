//! `quelch search` — semantic / hybrid search via Azure AI Search.
//!
//! CLI wrapper around [`crate::mcp::tools::search::run`].  Requires Azure
//! credentials for the production `AzureSearchAdapter`; in tests use the
//! `MockSearchApi` via the tool function directly.

use serde_json::Value;

use crate::config::{Config, DeploymentRole};
use crate::cosmos::factory::build_cosmos_backend;
use crate::mcp::expose::ExposeResolver;
use crate::mcp::schema::SchemaCatalog;
use crate::mcp::tools::search::{self, IncludeContent, SearchRequest, SearchToolConfig};
use crate::mcp::tools::search_api::AzureSearchAdapter;

/// CLI-friendly variant of [`IncludeContent`] (maps 1-to-1 via clap `ValueEnum`).
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
    /// Free-text search query.
    pub query: String,
    /// Comma-separated logical data-source names (optional).
    pub data_sources: Option<Vec<String>>,
    /// Optional structured filter.
    pub where_: Option<Value>,
    /// Maximum hits per page.
    pub top: usize,
    /// Pagination cursor from a prior response.
    pub cursor: Option<String>,
    /// Content detail level.
    pub include_content: IncludeContentArg,
    /// Include soft-deleted documents.
    pub include_deleted: bool,
    /// Emit machine-readable JSON instead of formatted output.
    pub json: bool,
}

/// Run `quelch search`.
///
/// Requires Azure credentials to construct the `AzureSearchAdapter`.  Will
/// error out with a clear message if credentials are unavailable.
pub async fn run(config: &Config, options: SearchOptions) -> anyhow::Result<()> {
    let cosmos = build_cosmos_backend(config).await?;

    // Find the first MCP deployment.
    let deployment_name = config
        .deployments
        .iter()
        .find(|d| d.role == DeploymentRole::Mcp)
        .map(|d| d.name.clone())
        .ok_or_else(|| {
            anyhow::anyhow!("no MCP deployment in config; `quelch search` requires one")
        })?;

    let sliced = crate::config::slice::for_deployment(config, &deployment_name)?;
    let expose = ExposeResolver::from_sliced(&sliced, &deployment_name)
        .map_err(|e| anyhow::anyhow!("expose resolver: {e}"))?;

    let schema = SchemaCatalog::default();
    let search_config = SearchToolConfig {
        disable_agentic: sliced
            .mcp
            .search
            .as_ref()
            .map(|s| s.disable_agentic)
            .unwrap_or(false),
        knowledge_base_name: sliced
            .mcp
            .search
            .as_ref()
            .and_then(|s| s.knowledge_base.clone())
            .unwrap_or_else(|| "quelch-prod-kb".into()),
        default_top: sliced.mcp.default_top as usize,
        max_top: sliced.mcp.max_top as usize,
    };

    // Build the Azure search adapter.
    // TODO: if Azure credentials are unavailable (e.g. dev machine without `az login`),
    // this will fail with an auth error — which is expected. Operators should
    // run `az login` or set the appropriate credential env vars before using `quelch search`.
    let search_service = sliced
        .search
        .service
        .as_deref()
        .unwrap_or("quelch-prod-search");
    let search_endpoint = format!("https://{search_service}.search.windows.net");
    let api_version = "2025-11-01-preview".to_string();
    let search_api = AzureSearchAdapter::new(search_endpoint, api_version)
        .map_err(|e| anyhow::anyhow!("search adapter init failed: {e}\nHint: run `az login` or set AZURE_CLIENT_ID / AZURE_CLIENT_SECRET env vars"))?;

    let req = SearchRequest {
        query: options.query,
        data_sources: options.data_sources,
        r#where: options.where_,
        top: options.top,
        cursor: options.cursor,
        include_deleted: options.include_deleted,
        include_content: options.include_content.into(),
    };

    let resp = search::run(&search_api, &expose, &schema, &search_config, req)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if options.json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    } else {
        println!("Results: {} (estimated)", resp.total_estimate);
        if let Some(answer) = &resp.answer {
            println!();
            println!("Answer: {answer}");
        }
        if !resp.items.is_empty() {
            println!();
            for item in &resp.items {
                println!(
                    "• [{:.2}] {} — {}",
                    item.score,
                    item.source_link,
                    item.snippet.as_deref().unwrap_or("—")
                );
            }
        }
        if let Some(cursor) = &resp.next_cursor {
            println!();
            println!("More results available. Continue with --cursor {cursor}");
        }
    }

    // cosmos is built but not used directly for search — it's kept to satisfy the
    // borrow checker lifetime. Drop it here.
    drop(cosmos);

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
//
// We do NOT test the full CLI path end-to-end because `AzureSearchAdapter`
// requires live Azure credentials. Instead we test the underlying tool
// function directly with `MockSearchApi`, which covers the business logic.
// See `crate::mcp::tools::search::tests` for comprehensive tool tests.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::schema::SchemaCatalog;
    use crate::mcp::tools::search::SearchToolConfig;
    use crate::mcp::tools::search_api::mock::MockSearchApi;
    use crate::mcp::tools::test_helpers::build_expose;

    #[tokio::test]
    async fn search_dispatches_to_search_tool_with_mock() {
        // Exercises search::run through a mock adapter — no Azure required.
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

        // MockSearchApi returns a default response with 1 hit.
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
