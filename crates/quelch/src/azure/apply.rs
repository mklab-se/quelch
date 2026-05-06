//! Top-level orchestrator for `quelch azure apply`.
//!
//! Applies both halves of the Azure plan in dependency order:
//!   1. Cosmos DB control-plane via [`crate::azure::cosmos_config::apply`] —
//!      ensures the database exists and creates any missing containers. This
//!      runs first because AI Search indexers reference the Cosmos containers.
//!   2. AI Search via [`crate::azure::rigg::generate`] +
//!      [`crate::azure::rigg::apply`] — pushes the desired-state resources
//!      (indexes, indexers, knowledge sources, knowledge bases, …).
//!
//! Phase 5 of the no-deploy pivot.

use anyhow::Result;

use crate::azure::cosmos_config::{self, ArmCosmosClient};
use crate::azure::rigg::{self, RiggApiAdapter};
use crate::config::schema::Config;

/// Apply both the Cosmos and AI Search halves of `cfg` to live Azure.
///
/// Idempotent. Cosmos containers are created if missing (Quelch never deletes
/// or repartitions). AI Search resources are upserted via `PUT`, replacing
/// any existing resource of the same name.
pub async fn apply<R: RiggApiAdapter>(
    cfg: &Config,
    cosmos_client: &ArmCosmosClient,
    rigg_client: &R,
) -> Result<()> {
    cosmos_config::apply(cosmos_client, &cfg.azure.cosmos).await?;
    let desired = rigg::generate(cfg)?;
    rigg::apply(&desired, rigg_client).await?;
    Ok(())
}
