//! Cosmos DB control-plane via ARM REST.
//!
//! Replaces the old Bicep-based provisioning with direct PUTs against
//! `management.azure.com`. The high-level entry points are [`plan`] (compute
//! the diff) and [`apply`] (execute it).

pub mod arm_client;
pub mod diff;

use anyhow::Result;

use crate::config::schema::CosmosConfig;

pub use arm_client::ArmCosmosClient;
pub use diff::{ContainerDiff, ContainerSpec, desired_containers, diff_containers};

/// The set of per-container actions Quelch will take on the next [`apply`].
pub struct CosmosPlan {
    /// One entry per desired container.
    pub diffs: Vec<ContainerDiff>,
}

/// Compute a [`CosmosPlan`] by diffing the desired containers against the
/// live ones.
///
/// If the database doesn't exist yet, [`ArmCosmosClient::list_containers`]
/// returns an empty list, so every desired container surfaces as `Create`.
pub async fn plan(client: &ArmCosmosClient, cosmos: &CosmosConfig) -> Result<CosmosPlan> {
    let want = desired_containers(cosmos);
    let have = client.list_containers(&cosmos.database).await?;
    Ok(CosmosPlan {
        diffs: diff_containers(&want, &have),
    })
}

/// Execute the plan: ensure the database exists, then create each missing
/// container.
///
/// Errors hard on [`ContainerDiff::PartitionKeyMismatch`] — Quelch will not
/// recreate a populated container behind the operator's back.
pub async fn apply(client: &ArmCosmosClient, cosmos: &CosmosConfig) -> Result<()> {
    client.ensure_database(&cosmos.database).await?;
    let plan = plan(client, cosmos).await?;
    for d in plan.diffs {
        match d {
            ContainerDiff::Match { .. } => continue,
            ContainerDiff::Create(spec) => {
                client.create_container(&cosmos.database, &spec).await?;
            }
            ContainerDiff::PartitionKeyMismatch { name, want, have } => {
                anyhow::bail!(
                    "container '{name}' exists with partition key '{have}', want '{want}'. \
                     Quelch will not auto-recreate (would lose data). Resolve manually then retry."
                );
            }
        }
    }
    Ok(())
}
