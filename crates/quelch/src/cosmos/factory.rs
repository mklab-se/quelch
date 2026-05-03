//! Shared factory for constructing a [`CosmosBackend`] from config.
//!
//! Used by multiple CLI commands (`quelch status`, `quelch reset`, etc.) so they
//! all use the same backend construction logic without duplicating it.

use crate::config::{Config, StateBackend};
use crate::cosmos::CosmosBackend;

/// Construct a boxed [`CosmosBackend`] from the top-level config.
///
/// For `state.backend = cosmos` this creates a real [`crate::cosmos::CosmosClient`]
/// authenticated via DefaultAzureCredential.
///
/// # Errors
///
/// Returns an error if:
/// - `cosmos.account` is not set and the backend is `cosmos`.
/// - The Azure credential / Cosmos endpoint cannot be reached.
pub async fn build_cosmos_backend(config: &Config) -> anyhow::Result<Box<dyn CosmosBackend>> {
    match &config.state.backend {
        StateBackend::Cosmos => {
            let account = config.cosmos.account.as_deref().ok_or_else(|| {
                anyhow::anyhow!("cosmos.account is required for state.backend=cosmos")
            })?;

            let endpoint = if account.starts_with("https://") {
                account.to_owned()
            } else {
                format!("https://{account}.documents.azure.com:443/")
            };

            let client =
                crate::cosmos::CosmosClient::new(&endpoint, &config.cosmos.database).await?;
            Ok(Box::new(client))
        }
        StateBackend::LocalFile => {
            anyhow::bail!("state.backend=local_file is not supported for this command; use cosmos")
        }
    }
}
