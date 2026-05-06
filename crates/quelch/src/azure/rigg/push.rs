//! Apply a [`RiggDesiredState`] to the live Azure AI Search service.
//!
//! Phase 4 of the no-deploy pivot: this module operates purely on in-memory
//! state. There are no on-disk YAML files, no `rigg/` directory.
//!
//! Resources are upserted in dependency order:
//! 1. Data sources (referenced by indexers)
//! 2. Skillsets (referenced by indexers)
//! 3. Indexes (referenced by indexers and knowledge sources)
//! 4. Indexers
//! 5. Knowledge sources (referenced by knowledge bases)
//! 6. Knowledge bases
//!
//! Quelch never schedules deletes — by spec, Quelch only adds. Users who
//! want a resource gone use the AI Search portal or the standalone `rigg`
//! tool.

use rigg_core::resources::ResourceKind;
use serde_json::Value as JsonValue;

use super::RiggDesiredState;
use super::plan::RiggApiAdapter;

/// Errors that can occur during apply.
#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    /// The rigg API call to upsert a resource failed.
    #[error("rigg api: {0}")]
    Api(String),
    /// Failed to serialise a desired-state resource to JSON.
    #[error("serialise: {0}")]
    Serialise(#[from] serde_json::Error),
}

/// Push every resource in `desired` to the live AI Search service.
///
/// Idempotent: each resource is sent through `create_or_update`, which
/// rigg-client maps to a `PUT` (Azure semantics: create-if-absent,
/// replace-if-present).
pub async fn apply<A: RiggApiAdapter>(
    desired: &RiggDesiredState,
    api: &A,
) -> Result<(), ApplyError> {
    // Collect (kind, name, body) tuples in dependency order, then push.
    let mut payloads: Vec<(ResourceKind, String, JsonValue)> = Vec::new();

    for r in &desired.data_sources {
        payloads.push((
            ResourceKind::DataSource,
            r.name.clone(),
            serde_json::to_value(r)?,
        ));
    }
    for r in &desired.skillsets {
        payloads.push((
            ResourceKind::Skillset,
            r.name.clone(),
            serde_json::to_value(r)?,
        ));
    }
    for r in &desired.indexes {
        payloads.push((
            ResourceKind::Index,
            r.name.clone(),
            serde_json::to_value(r)?,
        ));
    }
    for r in &desired.indexers {
        payloads.push((
            ResourceKind::Indexer,
            r.name.clone(),
            serde_json::to_value(r)?,
        ));
    }
    for r in &desired.knowledge_sources {
        payloads.push((
            ResourceKind::KnowledgeSource,
            r.name.clone(),
            serde_json::to_value(r)?,
        ));
    }
    for r in &desired.knowledge_bases {
        payloads.push((
            ResourceKind::KnowledgeBase,
            r.name.clone(),
            serde_json::to_value(r)?,
        ));
    }

    for (kind, name, body) in &payloads {
        api.upsert_resource(*kind, name, body)
            .await
            .map_err(|e| ApplyError::Api(e.to_string()))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::azure::rigg::plan::tests::MockRiggApi;
    use rigg_core::resources::{
        DataSource, Index, Indexer, KnowledgeBase, KnowledgeSource, Skillset,
        datasource::{DataSourceContainer, DataSourceCredentials},
    };

    fn full_state() -> RiggDesiredState {
        RiggDesiredState {
            indexes: vec![Index {
                name: "my-index".into(),
                fields: vec![],
                scoring_profiles: None,
                default_scoring_profile: None,
                cors_options: None,
                suggesters: None,
                analyzers: None,
                tokenizers: None,
                token_filters: None,
                char_filters: None,
                similarity: None,
                semantic: None,
                vector_search: None,
                extra: Default::default(),
            }],
            data_sources: vec![DataSource {
                name: "my-ds".into(),
                datasource_type: "cosmosdb".into(),
                credentials: DataSourceCredentials {
                    connection_string: Some("x".into()),
                },
                container: DataSourceContainer {
                    name: "c".into(),
                    query: None,
                },
                description: None,
                data_change_detection_policy: None,
                data_deletion_detection_policy: None,
                encryption_key: None,
                identity: None,
                extra: Default::default(),
            }],
            skillsets: vec![Skillset {
                name: "my-ss".into(),
                skills: vec![],
                description: None,
                cognitive_services: None,
                knowledge_store: None,
                index_projections: None,
                encryption_key: None,
                extra: Default::default(),
            }],
            indexers: vec![Indexer {
                name: "my-ix".into(),
                data_source_name: "my-ds".into(),
                target_index_name: "my-index".into(),
                skillset_name: Some("my-ss".into()),
                description: None,
                schedule: None,
                parameters: None,
                field_mappings: None,
                output_field_mappings: None,
                disabled: None,
                cache: None,
                encryption_key: None,
                extra: Default::default(),
            }],
            knowledge_sources: vec![KnowledgeSource {
                name: "my-ks".into(),
                index_name: "my-index".into(),
                description: None,
                knowledge_base_name: None,
                query_type: None,
                semantic_configuration: None,
                top: None,
                filter: None,
                select_fields: None,
                extra: Default::default(),
            }],
            knowledge_bases: vec![KnowledgeBase {
                name: "my-kb".into(),
                description: None,
                storage_connection_string_secret: None,
                storage_container: None,
                identity: None,
                extra: Default::default(),
            }],
        }
    }

    #[tokio::test]
    async fn apply_upserts_in_dependency_order() {
        let state = full_state();
        let api = MockRiggApi::default();
        apply(&state, &api).await.unwrap();

        let upserted = api.upserted.lock().unwrap();
        let kinds: Vec<ResourceKind> = upserted.iter().map(|(k, _, _)| *k).collect();
        assert_eq!(kinds.len(), 6);

        let pos = |target: ResourceKind| kinds.iter().position(|k| *k == target).unwrap();
        assert!(pos(ResourceKind::DataSource) < pos(ResourceKind::Indexer));
        assert!(pos(ResourceKind::Skillset) < pos(ResourceKind::Indexer));
        assert!(pos(ResourceKind::Index) < pos(ResourceKind::Indexer));
        assert!(pos(ResourceKind::Indexer) < pos(ResourceKind::KnowledgeSource));
        assert!(pos(ResourceKind::KnowledgeSource) < pos(ResourceKind::KnowledgeBase));
    }

    #[tokio::test]
    async fn apply_sends_resource_name_and_body() {
        let state = full_state();
        let api = MockRiggApi::default();
        apply(&state, &api).await.unwrap();
        let upserted = api.upserted.lock().unwrap();

        let (_, name, body) = upserted
            .iter()
            .find(|(k, _, _)| *k == ResourceKind::Index)
            .unwrap();
        assert_eq!(name, "my-index");
        assert_eq!(body.get("name").and_then(|v| v.as_str()), Some("my-index"));
    }

    #[tokio::test]
    async fn apply_on_empty_state_is_noop() {
        let state = RiggDesiredState::default();
        let api = MockRiggApi::default();
        apply(&state, &api).await.unwrap();
        assert!(api.upserted.lock().unwrap().is_empty());
    }
}
