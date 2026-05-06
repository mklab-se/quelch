//! Top-level orchestrator for `quelch azure plan`.
//!
//! Computes the union of two diffs:
//!   - Cosmos DB control-plane (containers): [`crate::azure::cosmos_config::plan`].
//!   - Azure AI Search resources: [`crate::azure::rigg::plan`] driven by
//!     [`crate::azure::rigg::generate`].
//!
//! Renders both halves into a single human-readable string. Phase 5 of the
//! no-deploy pivot.

use anyhow::Result;

use crate::azure::cosmos_config::{self, ContainerDiff};
use crate::azure::rigg::{self, RiggApiAdapter};
use crate::config::schema::Config;

/// The combined Azure plan: Cosmos container diff + AI Search resource diff.
pub struct AzurePlan {
    /// Per-container actions Quelch will take on the next `azure apply`.
    pub cosmos: cosmos_config::CosmosPlan,
    /// Per-resource actions for AI Search (indexes, indexers, KS, KB, …).
    pub rigg: rigg::RiggDiff,
}

/// Compute an [`AzurePlan`] by diffing both halves against the live Azure
/// services.
///
/// The Cosmos client targets ARM REST; the rigg client targets the AI Search
/// service. Both can require live network calls — this function does no I/O
/// other than what those clients perform.
pub async fn compute<R: RiggApiAdapter>(
    cfg: &Config,
    cosmos_client: &cosmos_config::ArmCosmosClient,
    rigg_client: &R,
) -> Result<AzurePlan> {
    let cosmos_plan = cosmos_config::plan(cosmos_client, &cfg.azure.cosmos).await?;
    let desired = rigg::generate(cfg)?;
    let rigg_diff = rigg::plan(&desired, rigg_client).await?;
    Ok(AzurePlan {
        cosmos: cosmos_plan,
        rigg: rigg_diff,
    })
}

/// Render an [`AzurePlan`] as a human-readable multi-line string suitable for
/// printing to stdout.
///
/// Format:
/// ```text
/// Cosmos DB:
///   + jira-issues (partition key /id)
///   = quelch-meta  (no change)
///
/// AI Search:
///   + indexes/jira-issues  (create)
///   ~ indexers/jira-issues  (update)
///       fields.0.searchable: false → true
/// ```
pub fn render(plan: &AzurePlan) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    writeln!(&mut out, "Cosmos DB:").ok();
    if plan.cosmos.diffs.is_empty() {
        writeln!(&mut out, "  (no managed containers)").ok();
    } else {
        for d in &plan.cosmos.diffs {
            match d {
                ContainerDiff::Match { name } => {
                    writeln!(&mut out, "  = {name}  (no change)").ok();
                }
                ContainerDiff::Create(spec) => {
                    writeln!(
                        &mut out,
                        "  + {} (partition key {})",
                        spec.name, spec.partition_key
                    )
                    .ok();
                }
                ContainerDiff::PartitionKeyMismatch { name, want, have } => {
                    writeln!(
                        &mut out,
                        "  ! {name}  (have pk={have}, want pk={want}) — manual fix required"
                    )
                    .ok();
                }
            }
        }
    }
    writeln!(&mut out).ok();
    writeln!(&mut out, "AI Search:").ok();
    write!(&mut out, "{}", plan.rigg.render()).ok();
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::azure::cosmos_config::{ContainerSpec, CosmosPlan};
    use crate::azure::rigg::{FieldChange, ResourceChange, ResourceRef, RiggDiff};
    use rigg_core::resources::ResourceKind;
    use serde_json::Value as JsonValue;

    #[test]
    fn renders_cosmos_create_match_and_mismatch() {
        let plan = AzurePlan {
            cosmos: CosmosPlan {
                diffs: vec![
                    ContainerDiff::Create(ContainerSpec {
                        name: "jira-issues".into(),
                        partition_key: "/id".into(),
                    }),
                    ContainerDiff::Match {
                        name: "quelch-meta".into(),
                    },
                    ContainerDiff::PartitionKeyMismatch {
                        name: "old-container".into(),
                        want: "/id".into(),
                        have: "/wrong".into(),
                    },
                ],
            },
            rigg: RiggDiff::default(),
        };
        let out = render(&plan);
        assert!(out.contains("+ jira-issues"), "{out}");
        assert!(out.contains("(partition key /id)"), "{out}");
        assert!(out.contains("= quelch-meta"), "{out}");
        assert!(out.contains("! old-container"), "{out}");
        assert!(out.contains("manual fix required"), "{out}");
        assert!(out.contains("Cosmos DB:"), "{out}");
        assert!(out.contains("AI Search:"), "{out}");
    }

    #[test]
    fn renders_rigg_create_match_and_update() {
        let mut rigg_diff = RiggDiff::default();
        rigg_diff.changes.push(ResourceChange::Create(ResourceRef {
            kind: ResourceKind::Index,
            name: "jira-issues".into(),
        }));
        rigg_diff.changes.push(ResourceChange::Match(ResourceRef {
            kind: ResourceKind::DataSource,
            name: "jira-issues".into(),
        }));
        rigg_diff.changes.push(ResourceChange::Update {
            rref: ResourceRef {
                kind: ResourceKind::Indexer,
                name: "jira-issues".into(),
            },
            changes: vec![FieldChange {
                path: "fields.0.searchable".into(),
                from: JsonValue::Bool(false),
                to: JsonValue::Bool(true),
            }],
        });

        let plan = AzurePlan {
            cosmos: CosmosPlan { diffs: vec![] },
            rigg: rigg_diff,
        };
        let out = render(&plan);
        assert!(out.contains("(no managed containers)"), "{out}");
        assert!(out.contains("+ indexes/jira-issues"), "{out}");
        assert!(out.contains("= data_sources/jira-issues"), "{out}");
        assert!(out.contains("~ indexers/jira-issues"), "{out}");
        assert!(out.contains("fields.0.searchable: false → true"), "{out}");
    }

    #[test]
    fn renders_clean_plan_with_only_matches() {
        let plan = AzurePlan {
            cosmos: CosmosPlan {
                diffs: vec![ContainerDiff::Match {
                    name: "quelch-meta".into(),
                }],
            },
            rigg: RiggDiff::default(),
        };
        let out = render(&plan);
        assert!(out.contains("= quelch-meta"), "{out}");
        // No '+' or '~' or '!' markers should appear in a no-op plan.
        assert!(!out.contains("  + "), "{out}");
        assert!(!out.contains("  ~ "), "{out}");
        assert!(!out.contains("  ! "), "{out}");
    }
}
