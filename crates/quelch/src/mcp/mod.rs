//! Q-MCP — the Quelch MCP server.
//!
//! This module hosts the Streamable HTTP server an agent connects to. Top
//! level entry points:
//!
//! - [`run_server`] is wired up by `quelch mcp --instance NAME`. It constructs
//!   a real Azure AI Search adapter and a real Cosmos client from the sliced
//!   config, then calls [`server::serve`].
//! - [`run_server_in_memory`] is used by `quelch dev`: it shares the caller's
//!   in-memory Cosmos backend and uses a no-op search adapter so no Azure
//!   credentials are required.

use std::sync::Arc;

use crate::config::Config;
use crate::config::schema::InstanceSpec;
use crate::cosmos::CosmosBackend;

pub mod auth;
pub mod error;
pub mod expose;
pub mod filter;
pub mod handlers;
pub mod schema;
pub mod server;
pub mod tools;

/// Resolve the named MCP instance from a config, returning its spec.
fn lookup_mcp_instance<'a>(
    config: &'a Config,
    instance_name: &str,
) -> anyhow::Result<&'a crate::config::schema::McpInstance> {
    let inst = config
        .instances
        .iter()
        .find(|i| i.name == instance_name)
        .ok_or_else(|| anyhow::anyhow!("instance '{instance_name}' not found in config"))?;
    match &inst.spec {
        InstanceSpec::Mcp(m) => Ok(m),
        InstanceSpec::Ingest(_) => Err(anyhow::anyhow!(
            "instance '{instance_name}' is an ingest instance; quelch mcp requires kind=mcp"
        )),
    }
}

/// Build a `SearchToolConfig` for the named MCP instance.
fn build_search_config(
    mcp: &crate::config::schema::McpInstance,
) -> tools::search::SearchToolConfig {
    tools::search::SearchToolConfig {
        knowledge_base_name: mcp.knowledge_base.clone(),
        ..tools::search::SearchToolConfig::default()
    }
}

/// Run the MCP HTTP server backed by a real Azure AI Search account and a
/// real Cosmos client.
///
/// Wired by `quelch mcp --instance NAME`. Errors out early if either client
/// cannot be constructed (e.g. missing Azure credentials).
pub async fn run_server(
    config: &Config,
    instance_name: &str,
    bind_addr: &str,
) -> anyhow::Result<()> {
    let sliced = crate::config::slice::slice_for_instance(config, instance_name)?;
    let mcp = lookup_mcp_instance(&sliced, instance_name)?;

    let cosmos: Arc<dyn CosmosBackend> =
        Arc::from(crate::cosmos::factory::build_cosmos_backend(&sliced).await?);

    let search_endpoint = sliced
        .azure
        .search
        .as_ref()
        .map(|s| s.endpoint.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "azure.search.endpoint is required for instance '{instance_name}' (kind=mcp)"
            )
        })?;
    let api_version = "2025-11-01-preview".to_string();
    let search: Arc<dyn tools::search_api::SearchApiAdapter> = Arc::new(
        tools::search_api::AzureSearchAdapter::new(search_endpoint, api_version)
            .map_err(|e| anyhow::anyhow!("search adapter: {e}"))?,
    );

    let expose = Arc::new(
        expose::ExposeResolver::from_sliced(&sliced, instance_name)
            .map_err(|e| anyhow::anyhow!("expose resolver: {e}"))?,
    );
    let schema = Arc::new(schema::SchemaCatalog::default());
    let search_config = Arc::new(build_search_config(mcp));

    let state = server::ServerState {
        cosmos,
        search,
        expose,
        schema,
        search_config,
    };

    server::serve(state, bind_addr).await
}

/// Run the MCP server with a caller-provided Cosmos backend and a no-op
/// search adapter.
///
/// Used by `quelch dev`, which spins up the MCP server next to an in-memory
/// Cosmos and an ingest worker — no Azure access required.
pub async fn run_server_in_memory(
    config: &Config,
    instance_name: &str,
    bind_addr: &str,
    cosmos: Arc<dyn CosmosBackend>,
) -> anyhow::Result<()> {
    let sliced = crate::config::slice::slice_for_instance(config, instance_name)?;
    let mcp = lookup_mcp_instance(&sliced, instance_name)?;

    let search: Arc<dyn tools::search_api::SearchApiAdapter> =
        Arc::new(tools::search_api::NoOpSearch);

    let expose = Arc::new(
        expose::ExposeResolver::from_sliced(&sliced, instance_name)
            .map_err(|e| anyhow::anyhow!("expose resolver: {e}"))?,
    );
    let schema = Arc::new(schema::SchemaCatalog::default());
    let search_config = Arc::new(build_search_config(mcp));

    let state = server::ServerState {
        cosmos,
        search,
        expose,
        schema,
        search_config,
    };

    server::serve(state, bind_addr).await
}
