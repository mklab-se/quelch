//! MCP (Model Context Protocol) server module.
//!
//! This module houses the MCP server implementation, including:
//! - Filter grammar parser and translators (Tasks 6.1–6.3)
//! - Tool definitions (Tasks 6.4–6.5)
//! - HTTP transport, auth, and CLI integration (Tasks 6.7–6.9)

pub mod auth;
pub mod error;
pub mod expose;
pub mod filter;
pub mod handlers;
pub mod schema;
pub mod server;
pub mod tools;

// ---------------------------------------------------------------------------
// Top-level entry point used by `quelch mcp`
// ---------------------------------------------------------------------------

/// Build the `ServerState` from config and start the HTTP server.
///
/// This is the entry point wired up by the `quelch mcp` CLI command.
/// It constructs the Cosmos backend, search adapter, and expose resolver from
/// the sliced (per-deployment) config, then calls [`server::serve`].
///
/// # Startup behaviour for the search adapter
///
/// `AzureSearchAdapter::new` acquires an Azure auth token at construction time.
/// If Azure credentials are not available (e.g. in CI without Azure access),
/// this function returns an error.  Use [`server::router`] with an injected
/// mock `ServerState` for integration tests that don't require Azure.
pub async fn run_server(
    config: &crate::config::Config,
    deployment_name: &str,
    bind_addr: &str,
) -> anyhow::Result<()> {
    use std::sync::Arc;

    use crate::config::DeploymentRole;

    let sliced = crate::config::slice::for_deployment(config, deployment_name)?;

    // Validate the deployment is an MCP role.
    let dep = sliced.deployments.first().ok_or_else(|| {
        anyhow::anyhow!("no deployment found after slicing for '{deployment_name}'")
    })?;

    if dep.role != DeploymentRole::Mcp {
        anyhow::bail!(
            "quelch mcp requires a deployment with role=mcp, got '{:?}' for '{deployment_name}'",
            dep.role
        );
    }

    // Build Cosmos backend.
    let cosmos: Arc<dyn crate::cosmos::CosmosBackend> = build_cosmos(&sliced).await?;

    // Build the Azure AI Search adapter.
    // Auth tokens are acquired here; if credentials are missing this errors out early.
    let search_service = sliced
        .search
        .service
        .as_deref()
        .unwrap_or("quelch-prod-search");
    let search_endpoint = format!("https://{search_service}.search.windows.net");
    let api_version = "2025-11-01-preview".to_string();
    let search: Arc<dyn tools::search_api::SearchApiAdapter> = Arc::new(
        tools::search_api::AzureSearchAdapter::new(search_endpoint, api_version)
            .map_err(|e| anyhow::anyhow!("search adapter: {e}"))?,
    );

    // Build the expose resolver (enforces deployment's `expose:` list).
    let expose = Arc::new(
        expose::ExposeResolver::from_sliced(&sliced, deployment_name)
            .map_err(|e| anyhow::anyhow!("expose resolver: {e}"))?,
    );

    // Static schema catalog.
    let schema = Arc::new(schema::SchemaCatalog::default());

    // Search tool config derived from `mcp:` section.
    let search_config = Arc::new(tools::search::SearchToolConfig {
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
    });

    let state = server::ServerState {
        cosmos,
        search,
        expose,
        schema,
        search_config,
    };

    server::serve(state, bind_addr).await
}

/// Construct a [`CosmosBackend`][crate::cosmos::CosmosBackend] from the sliced config.
async fn build_cosmos(
    config: &crate::config::Config,
) -> anyhow::Result<std::sync::Arc<dyn crate::cosmos::CosmosBackend>> {
    use crate::config::StateBackend;

    match &config.state.backend {
        StateBackend::Cosmos => {
            let account = config.cosmos.account.as_deref().ok_or_else(|| {
                anyhow::anyhow!("cosmos.account is required when state.backend=cosmos")
            })?;
            let endpoint = if account.starts_with("https://") {
                account.to_owned()
            } else {
                format!("https://{account}.documents.azure.com:443/")
            };
            let client =
                crate::cosmos::CosmosClient::new(&endpoint, &config.cosmos.database).await?;
            Ok(std::sync::Arc::new(client))
        }
        StateBackend::LocalFile => {
            anyhow::bail!(
                "state.backend=local_file is not supported for the MCP server; use cosmos"
            )
        }
    }
}
