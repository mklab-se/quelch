//! Azure integration: Cosmos control-plane, AI Search (rigg), and the
//! top-level `quelch azure plan` / `quelch azure apply` orchestrators.

pub mod apply;
pub mod cosmos_config;
pub mod indexer;
pub mod plan;
pub mod rigg;

use std::sync::Arc;

use anyhow::{Context, Result};
use azure_identity::DefaultAzureCredentialBuilder;

use crate::config::schema::Config;

use cosmos_config::ArmCosmosClient;
use rigg::RiggClientAdapter;

/// AI Search REST API version Quelch sends to `rigg-client` for control-plane
/// calls. Tracks the preview version that ships rigg-core's resource shapes.
const SEARCH_PREVIEW_API_VERSION: &str = "2025-11-01-preview";

/// Build an [`ArmCosmosClient`] from the `azure.cosmos` block of `cfg`.
///
/// Returns an error if any of `subscription_id`, `resource_group`, or
/// `account` is missing — those are required for ARM control-plane calls and
/// do not have safe defaults.
///
/// Authentication uses [`DefaultAzureCredentialBuilder`], which honours
/// environment variables, managed identity, and `az login` in that order.
pub fn build_cosmos_client(cfg: &Config) -> Result<ArmCosmosClient> {
    let cosmos = &cfg.azure.cosmos;
    let subscription_id = cosmos.subscription_id.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "control-plane fields missing in `azure.cosmos` — `subscription_id` is required \
             for `azure plan` / `azure apply`"
        )
    })?;
    let resource_group = cosmos.resource_group.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "control-plane fields missing in `azure.cosmos` — `resource_group` is required \
             for `azure plan` / `azure apply`"
        )
    })?;
    let account = cosmos.account.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "control-plane fields missing in `azure.cosmos` — `account` is required \
             for `azure plan` / `azure apply`"
        )
    })?;

    let credential = DefaultAzureCredentialBuilder::default()
        .build()
        .context("build DefaultAzureCredential for ARM access")?;

    Ok(ArmCosmosClient {
        credential: Arc::new(credential),
        http: reqwest::Client::new(),
        subscription_id,
        resource_group,
        account,
    })
}

/// Build a [`RiggClientAdapter`] (production rigg client) from the
/// `azure.search` block of `cfg`.
///
/// Returns an error if `azure.search` is unset — the AI Search endpoint has
/// no safe default. The adapter uses `rigg-client`'s built-in auth provider
/// (`DefaultAzureCredential`-equivalent under the hood).
pub fn build_rigg_client(cfg: &Config) -> Result<RiggClientAdapter> {
    let search = cfg.azure.search.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "`azure.search.endpoint` missing — required for AI Search configuration \
             (`azure plan` / `azure apply`)"
        )
    })?;
    RiggClientAdapter::new(
        search.endpoint.clone(),
        SEARCH_PREVIEW_API_VERSION.to_string(),
    )
    .context("build rigg AI Search client")
}
