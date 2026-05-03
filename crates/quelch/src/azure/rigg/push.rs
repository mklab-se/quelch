/// Apply a [`PlanReport`] to Azure AI Search by creating, updating, and
/// deleting resources in the correct dependency order.
///
/// Dependency ordering on **create / update**:
/// 1. Data sources (indexers and skillsets reference them)
/// 2. Skillsets
/// 3. Indexes
/// 4. Indexers (reference all of the above)
/// 5. Knowledge sources (reference indexes)
/// 6. Knowledge bases (reference knowledge sources)
///
/// On **delete** the order is reversed so dependants are removed first.
use std::path::Path;

use rigg_core::resources::ResourceKind;

use crate::azure::rigg::plan::{PlanReport, ResourceRef, RiggApiAdapter};

/// The outcome of a push run.
#[derive(Debug, Default)]
pub struct PushOutcome {
    /// Resources successfully created.
    pub created: Vec<ResourceRef>,
    /// Resources successfully updated.
    pub updated: Vec<ResourceRef>,
    /// Resources successfully deleted.
    pub deleted: Vec<ResourceRef>,
}

/// Errors that can occur during a push.
#[derive(Debug, thiserror::Error)]
pub enum PushError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("rigg: {0}")]
    Rigg(String),
}

/// Resource kind ordering for create / update operations.
const CREATE_ORDER: &[ResourceKind] = &[
    ResourceKind::DataSource,
    ResourceKind::Skillset,
    ResourceKind::Index,
    ResourceKind::Indexer,
    ResourceKind::KnowledgeSource,
    ResourceKind::KnowledgeBase,
];

/// Apply the given plan, reading local files for body content.
///
/// `rigg_dir` must be the root of the quelch rigg directory (the directory
/// containing `indexes/`, `datasources/`, etc. subdirectories).
///
/// `api` is the rigg adapter — use [`RiggClientAdapter`] in production or a
/// mock in tests.
pub async fn run<A: RiggApiAdapter>(
    plan: PlanReport,
    rigg_dir: &Path,
    api: &A,
) -> Result<PushOutcome, PushError> {
    let mut outcome = PushOutcome::default();

    // ---- Creates & updates (dependency order) ----
    // Collect creates and updates into per-kind buckets first, then flush in
    // CREATE_ORDER so we respect dependency ordering.
    let mut creates_by_kind: std::collections::HashMap<ResourceKind, Vec<ResourceRef>> =
        std::collections::HashMap::new();
    let mut updates_by_kind: std::collections::HashMap<ResourceKind, Vec<ResourceRef>> =
        std::collections::HashMap::new();

    for rref in plan.creates {
        creates_by_kind.entry(rref.kind).or_default().push(rref);
    }
    for (rref, _diff) in plan.updates {
        updates_by_kind.entry(rref.kind).or_default().push(rref);
    }

    for kind in CREATE_ORDER {
        // Creates first.
        if let Some(refs) = creates_by_kind.remove(kind) {
            for rref in refs {
                let body = read_resource_body(rigg_dir, &rref)?;
                api.upsert_resource(rref.kind, &rref.name, &body)
                    .await
                    .map_err(|e| PushError::Rigg(e.to_string()))?;
                outcome.created.push(rref);
            }
        }
        // Updates second.
        if let Some(refs) = updates_by_kind.remove(kind) {
            for rref in refs {
                let body = read_resource_body(rigg_dir, &rref)?;
                api.upsert_resource(rref.kind, &rref.name, &body)
                    .await
                    .map_err(|e| PushError::Rigg(e.to_string()))?;
                outcome.updated.push(rref);
            }
        }
    }

    // ---- Deletes (reverse dependency order) ----
    // Collect into a per-kind map, then flush in reverse of CREATE_ORDER.
    let mut deletes_by_kind: std::collections::HashMap<ResourceKind, Vec<ResourceRef>> =
        std::collections::HashMap::new();
    for rref in plan.deletes {
        deletes_by_kind.entry(rref.kind).or_default().push(rref);
    }

    for kind in CREATE_ORDER.iter().rev() {
        if let Some(refs) = deletes_by_kind.remove(kind) {
            for rref in refs {
                api.delete_resource(rref.kind, &rref.name)
                    .await
                    .map_err(|e| PushError::Rigg(e.to_string()))?;
                outcome.deleted.push(rref);
            }
        }
    }

    Ok(outcome)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Read the local YAML file for a resource and parse it into a JSON value
/// suitable for the Azure REST API.
fn read_resource_body(rigg_dir: &Path, rref: &ResourceRef) -> Result<serde_json::Value, PushError> {
    let subdir = crate::azure::rigg::plan::subdir_for_kind(rref.kind);
    let path = rigg_dir.join(subdir).join(format!("{}.yaml", rref.name));

    let yaml_text = std::fs::read_to_string(&path)?;
    let yaml_val: serde_yaml::Value = serde_yaml::from_str(&yaml_text)?;
    // Round-trip to JSON for the REST API.
    let json_str = serde_json::to_string(&yaml_val).map_err(|e| PushError::Rigg(e.to_string()))?;
    let json_val: serde_json::Value =
        serde_json::from_str(&json_str).map_err(|e| PushError::Rigg(e.to_string()))?;
    Ok(json_val)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::azure::rigg::plan::subdir_for_kind;
    use crate::azure::rigg::plan::tests::MockRiggApi;
    use std::path::Path;

    /// Write a minimal YAML file for the given kind/name under rigg_dir.
    fn write_resource(dir: &Path, kind: ResourceKind, name: &str) {
        let subdir = dir.join(subdir_for_kind(kind));
        std::fs::create_dir_all(&subdir).unwrap();
        let content = format!("name: {name}\n");
        std::fs::write(subdir.join(format!("{name}.yaml")), content).unwrap();
    }

    /// Build a minimal PlanReport with exactly the creates/updates/deletes
    /// given.
    fn make_plan(
        creates: Vec<(ResourceKind, &str)>,
        updates: Vec<(ResourceKind, &str)>,
        deletes: Vec<(ResourceKind, &str)>,
    ) -> PlanReport {
        PlanReport {
            creates: creates
                .into_iter()
                .map(|(k, n)| ResourceRef {
                    kind: k,
                    name: n.to_string(),
                })
                .collect(),
            updates: updates
                .into_iter()
                .map(|(k, n)| {
                    (
                        ResourceRef {
                            kind: k,
                            name: n.to_string(),
                        },
                        crate::azure::rigg::plan::ResourceDiff {
                            field_changes: vec![],
                        },
                    )
                })
                .collect(),
            deletes: deletes
                .into_iter()
                .map(|(k, n)| ResourceRef {
                    kind: k,
                    name: n.to_string(),
                })
                .collect(),
            unchanged: vec![],
        }
    }

    #[tokio::test]
    async fn push_creates_in_dependency_order() {
        let dir = tempfile::tempdir().unwrap();
        // Pre-create files for all resources.
        write_resource(dir.path(), ResourceKind::Index, "my-index");
        write_resource(dir.path(), ResourceKind::DataSource, "my-ds");
        write_resource(dir.path(), ResourceKind::Skillset, "my-skillset");
        write_resource(dir.path(), ResourceKind::Indexer, "my-indexer");
        write_resource(dir.path(), ResourceKind::KnowledgeSource, "my-ks");
        write_resource(dir.path(), ResourceKind::KnowledgeBase, "my-kb");

        let plan = make_plan(
            vec![
                (ResourceKind::KnowledgeBase, "my-kb"),
                (ResourceKind::Index, "my-index"),
                (ResourceKind::Indexer, "my-indexer"),
                (ResourceKind::DataSource, "my-ds"),
                (ResourceKind::Skillset, "my-skillset"),
                (ResourceKind::KnowledgeSource, "my-ks"),
            ],
            vec![],
            vec![],
        );

        let api = MockRiggApi::default();
        let outcome = run(plan, dir.path(), &api).await.unwrap();

        let upserted = api.upserted.lock().unwrap();
        let kinds: Vec<ResourceKind> = upserted.iter().map(|(k, _)| *k).collect();

        // DataSource must come before Skillset, which must come before Index,
        // which must come before Indexer, which must come before KS, which
        // must come before KB.
        let pos = |target: ResourceKind| kinds.iter().position(|k| *k == target).unwrap();
        assert!(pos(ResourceKind::DataSource) < pos(ResourceKind::Skillset));
        assert!(pos(ResourceKind::Skillset) < pos(ResourceKind::Index));
        assert!(pos(ResourceKind::Index) < pos(ResourceKind::Indexer));
        assert!(pos(ResourceKind::Indexer) < pos(ResourceKind::KnowledgeSource));
        assert!(pos(ResourceKind::KnowledgeSource) < pos(ResourceKind::KnowledgeBase));

        assert_eq!(outcome.created.len(), 6);
        assert!(outcome.updated.is_empty());
        assert!(outcome.deleted.is_empty());
    }

    #[tokio::test]
    async fn push_deletes_in_reverse_order() {
        let dir = tempfile::tempdir().unwrap();

        let plan = make_plan(
            vec![],
            vec![],
            vec![
                (ResourceKind::DataSource, "ds"),
                (ResourceKind::Index, "idx"),
                (ResourceKind::KnowledgeBase, "kb"),
            ],
        );

        let api = MockRiggApi::default();
        let outcome = run(plan, dir.path(), &api).await.unwrap();

        let deleted = api.deleted.lock().unwrap();
        let kinds: Vec<ResourceKind> = deleted.iter().map(|(k, _)| *k).collect();

        // KB before Index before DataSource on delete.
        let pos = |target: ResourceKind| kinds.iter().position(|k| *k == target).unwrap();
        assert!(pos(ResourceKind::KnowledgeBase) < pos(ResourceKind::Index));
        assert!(pos(ResourceKind::Index) < pos(ResourceKind::DataSource));

        assert_eq!(outcome.deleted.len(), 3);
        assert!(outcome.created.is_empty());
        assert!(outcome.updated.is_empty());
    }

    #[tokio::test]
    async fn push_updates_in_dependency_order() {
        let dir = tempfile::tempdir().unwrap();
        write_resource(dir.path(), ResourceKind::DataSource, "ds");
        write_resource(dir.path(), ResourceKind::Indexer, "idx");

        let plan = make_plan(
            vec![],
            vec![
                (ResourceKind::Indexer, "idx"),
                (ResourceKind::DataSource, "ds"),
            ],
            vec![],
        );

        let api = MockRiggApi::default();
        let outcome = run(plan, dir.path(), &api).await.unwrap();

        let upserted = api.upserted.lock().unwrap();
        let kinds: Vec<ResourceKind> = upserted.iter().map(|(k, _)| *k).collect();
        let pos = |target: ResourceKind| kinds.iter().position(|k| *k == target).unwrap();
        assert!(pos(ResourceKind::DataSource) < pos(ResourceKind::Indexer));

        assert_eq!(outcome.updated.len(), 2);
    }
}
