//! Build the in-memory desired state for Azure AI Search from a [`Config`].
//!
//! Phase 4 of the no-deploy pivot: `quelch.yaml` → [`RiggDesiredState`].
//! The orchestrator (`azure plan`/`azure apply`, Phase 5) hands this to
//! [`super::plan`] / [`super::apply`].
//!
//! For each MCP instance (`InstanceSpec::Mcp`), and for each entry in its
//! `expose:` list (each entry names a Cosmos container kind such as
//! `jira_issues`), we produce one tuple of:
//!   - [`Index`] — destination of the indexer.
//!   - [`DataSource`] — pointer at the Cosmos container.
//!   - [`Skillset`] — vectorises the document text using the configured
//!     embedding deployment.
//!   - [`Indexer`] — pulls Cosmos → skillset → index.
//!   - [`KnowledgeSource`] — wraps the index for Agentic Retrieval.
//!
//! Plus exactly one [`KnowledgeBase`] per MCP instance, named per the
//! instance's `knowledge_base:` field, listing the knowledge sources its
//! `expose:` list produced and pointing at the configured chat deployment.

use std::collections::HashMap;

use serde_json::json;
use thiserror::Error;

use rigg_core::resources::{
    DataSource, Index, Indexer, KnowledgeBase, KnowledgeSource, Skillset,
    datasource::{DataSourceContainer, DataSourceCredentials},
    index::{Field, SemanticConfiguration, VectorSearch},
    indexer::IndexerSchedule,
    skillset::{Skill, SkillInput, SkillOutput},
};

use crate::config::schema::{AiConfig, Config, ContainerLayout, InstanceSpec, McpInstance};

use super::RiggDesiredState;

/// Errors that can occur while building [`RiggDesiredState`].
#[derive(Debug, Error)]
pub enum GenerateError {
    /// `azure.ai` is not configured but the config has at least one MCP
    /// instance. AI Search needs the embedding deployment for vectorisation
    /// and the chat deployment for the knowledge base.
    #[error(
        "azure.ai is required when at least one MCP instance is configured \
         (the AI Search skillset needs an embedding deployment, the knowledge \
         base needs a chat deployment)"
    )]
    AiBlockMissing,

    /// An MCP instance's `expose:` entry doesn't match a known container
    /// kind. The supported kinds are the keys of [`ContainerLayout`].
    #[error(
        "MCP instance '{instance}' exposes unknown container kind '{kind}' \
         (supported: jira_issues, jira_sprints, jira_fix_versions, \
         jira_projects, confluence_pages, confluence_spaces)"
    )]
    UnknownExposeKind {
        /// The MCP instance whose `expose` list contained the bad entry.
        instance: String,
        /// The bad `expose` value.
        kind: String,
    },
}

/// Build the desired AI Search resource state for `cfg`.
///
/// Returns a populated [`RiggDesiredState`] if any MCP instance is present,
/// or an empty state if there are no MCP instances. Returns an error if at
/// least one MCP instance exists without a configured `azure.ai` block.
pub fn generate(cfg: &Config) -> Result<RiggDesiredState, GenerateError> {
    let mut state = RiggDesiredState::default();

    let mcp_instances: Vec<&McpInstance> = cfg
        .instances
        .iter()
        .filter_map(|inst| match &inst.spec {
            InstanceSpec::Mcp(m) => Some(m),
            _ => None,
        })
        .collect();

    if mcp_instances.is_empty() {
        return Ok(state);
    }

    let ai = cfg.azure.ai.as_ref().ok_or(GenerateError::AiBlockMissing)?;

    // Avoid emitting duplicate index/indexer/etc. resources when multiple MCP
    // instances expose the same container. A KS is shared across MCPs that
    // expose the same container; only the KB → KS membership differs.
    let mut emitted_containers: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    for mcp in &mcp_instances {
        let mut ks_for_kb: Vec<String> = Vec::with_capacity(mcp.expose.len());

        for kind in &mcp.expose {
            let container = container_name_for_kind(&cfg.azure.cosmos.containers, kind)
                .ok_or_else(|| GenerateError::UnknownExposeKind {
                    instance: instance_name_for_mcp(cfg, mcp).to_string(),
                    kind: kind.clone(),
                })?;

            ks_for_kb.push(knowledge_source_name(&container));

            if emitted_containers.insert(container.clone()) {
                state.indexes.push(build_index(&container, ai));
                state.data_sources.push(build_datasource(&container));
                state.skillsets.push(build_skillset(&container, ai));
                state.indexers.push(build_indexer(&container));
                state
                    .knowledge_sources
                    .push(build_knowledge_source(&container));
            }
        }

        state
            .knowledge_bases
            .push(build_knowledge_base(mcp, &ks_for_kb, ai));
    }

    Ok(state)
}

// ---------------------------------------------------------------------------
// Resource builders
// ---------------------------------------------------------------------------

/// Build a minimal [`Index`] for `container`, wired with a vector field
/// pointing at the configured embedding deployment.
fn build_index(container: &str, ai: &AiConfig) -> Index {
    let dimensions = ai.embedding.dimensions as i32;
    let vector_profile = "default-vector-profile";

    let mut id_field = blank_field("id", "Edm.String");
    id_field.key = Some(true);
    id_field.searchable = Some(false);
    id_field.filterable = Some(true);
    id_field.sortable = Some(true);
    id_field.retrievable = Some(true);

    let mut content_field = blank_field("content", "Edm.String");
    content_field.searchable = Some(true);
    content_field.retrievable = Some(true);
    content_field.stored = Some(true);
    content_field.analyzer = Some("standard.lucene".to_string());

    let mut vector_field = blank_field("content_vector", "Collection(Edm.Single)");
    vector_field.searchable = Some(true);
    vector_field.stored = Some(true);
    vector_field.dimensions = Some(dimensions);
    vector_field.vector_search_profile = Some(vector_profile.to_string());

    let fields = vec![id_field, content_field, vector_field];

    Index {
        name: container.to_string(),
        fields,
        scoring_profiles: None,
        default_scoring_profile: None,
        cors_options: None,
        suggesters: None,
        analyzers: None,
        tokenizers: None,
        token_filters: None,
        char_filters: None,
        similarity: None,
        semantic: Some(SemanticConfiguration {
            default_configuration: Some("default-semantic".to_string()),
            configurations: Some(vec![json!({
                "name": "default-semantic",
                "prioritizedFields": {
                    "contentFields": [{"fieldName": "content"}]
                }
            })]),
        }),
        vector_search: Some(VectorSearch {
            algorithms: Some(vec![json!({
                "name": "default-hnsw",
                "kind": "hnsw",
                "hnswParameters": {
                    "metric": "cosine",
                    "m": 4,
                    "efConstruction": 400,
                    "efSearch": 500
                }
            })]),
            profiles: Some(vec![json!({
                "name": vector_profile,
                "algorithm": "default-hnsw",
                "vectorizer": "azure-openai-vectorizer"
            })]),
            vectorizers: Some(vec![json!({
                "name": "azure-openai-vectorizer",
                "kind": "azureOpenAI",
                "azureOpenAIParameters": {
                    "resourceUri": ai.endpoint,
                    "deploymentId": ai.embedding.deployment,
                    "modelName": ai.embedding.deployment,
                }
            })]),
            compressions: None,
        }),
        extra: Default::default(),
    }
}

/// Build a [`DataSource`] pointing at the Cosmos container.
///
/// The connection string is left as a placeholder reference so that the
/// orchestrator (Phase 5) can rewrite it to either a Key Vault reference or
/// a directly-fetched control-plane key — depending on the user's auth model.
fn build_datasource(container: &str) -> DataSource {
    DataSource {
        name: container.to_string(),
        datasource_type: "cosmosdb".to_string(),
        credentials: DataSourceCredentials {
            connection_string: Some(
                "@Microsoft.KeyVault(SecretUri=https://kv.vault.azure.net/secrets/cosmos-connection)"
                    .to_string(),
            ),
        },
        container: DataSourceContainer {
            name: container.to_string(),
            query: Some(
                "SELECT * FROM c WHERE c._ts >= @HighWaterMark ORDER BY c._ts".to_string(),
            ),
        },
        description: Some(format!("Cosmos DB container '{container}' for Quelch")),
        data_change_detection_policy: Some(json!({
            "@odata.type": "#Microsoft.Azure.Search.HighWaterMarkChangeDetectionPolicy",
            "highWaterMarkColumnName": "_ts"
        })),
        data_deletion_detection_policy: Some(json!({
            "@odata.type": "#Microsoft.Azure.Search.SoftDeleteColumnDeletionDetectionPolicy",
            "softDeleteColumnName": "_deleted",
            "softDeleteMarkerValue": "true"
        })),
        encryption_key: None,
        identity: None,
        extra: Default::default(),
    }
}

/// Build a [`Skillset`] that vectorises the index's `content` field via the
/// configured Azure OpenAI / Foundry embedding deployment.
fn build_skillset(container: &str, ai: &AiConfig) -> Skillset {
    let embedding_skill = Skill {
        odata_type: "#Microsoft.Skills.Text.AzureOpenAIEmbeddingSkill".to_string(),
        name: "azure-openai-embedding".to_string(),
        description: Some("Compute embeddings via Azure OpenAI / Foundry".to_string()),
        context: Some("/document".to_string()),
        inputs: vec![SkillInput {
            name: "text".to_string(),
            source: "/document/content".to_string(),
            source_context: None,
            inputs: None,
        }],
        outputs: vec![SkillOutput {
            name: "embedding".to_string(),
            target_name: Some("content_vector".to_string()),
        }],
        extra: {
            let mut m = HashMap::new();
            m.insert("resourceUri".to_string(), json!(ai.endpoint));
            m.insert("deploymentId".to_string(), json!(ai.embedding.deployment));
            m.insert("modelName".to_string(), json!(ai.embedding.deployment));
            m
        },
    };

    Skillset {
        name: skillset_name(container),
        description: Some(format!("Vectorisation skillset for '{container}'")),
        skills: vec![embedding_skill],
        cognitive_services: None,
        knowledge_store: None,
        index_projections: None,
        encryption_key: None,
        extra: Default::default(),
    }
}

/// Build an [`Indexer`] pulling the Cosmos data source through the skillset
/// into the index.
fn build_indexer(container: &str) -> Indexer {
    Indexer {
        name: container.to_string(),
        data_source_name: container.to_string(),
        target_index_name: container.to_string(),
        skillset_name: Some(skillset_name(container)),
        description: Some(format!(
            "Indexer pulling '{container}' from Cosmos DB into AI Search"
        )),
        schedule: Some(IndexerSchedule {
            interval: "PT5M".to_string(),
            start_time: None,
        }),
        parameters: Some(rigg_core::resources::indexer::IndexerParameters {
            batch_size: None,
            max_failed_items: Some(-1),
            max_failed_items_per_batch: Some(-1),
            configuration: Some(json!({
                "assumeOrderByHighWaterMarkColumn": true
            })),
        }),
        field_mappings: None,
        output_field_mappings: Some(vec![rigg_core::resources::indexer::FieldMapping {
            source_field_name: "/document/content_vector".to_string(),
            target_field_name: Some("content_vector".to_string()),
            mapping_function: None,
        }]),
        disabled: None,
        cache: None,
        encryption_key: None,
        extra: Default::default(),
    }
}

/// Build a [`KnowledgeSource`] wrapping the index for Agentic Retrieval.
fn build_knowledge_source(container: &str) -> KnowledgeSource {
    KnowledgeSource {
        name: knowledge_source_name(container),
        index_name: container.to_string(),
        description: Some(format!("Knowledge source wrapping the '{container}' index")),
        knowledge_base_name: None,
        query_type: Some("semantic".to_string()),
        semantic_configuration: Some("default-semantic".to_string()),
        top: Some(5),
        filter: None,
        select_fields: None,
        extra: Default::default(),
    }
}

/// Build a [`KnowledgeBase`] for an MCP instance, listing its KS members and
/// pointing at the configured chat deployment.
fn build_knowledge_base(mcp: &McpInstance, ks_names: &[String], ai: &AiConfig) -> KnowledgeBase {
    let mut extra = HashMap::new();

    extra.insert(
        "knowledgeSources".to_string(),
        json!(
            ks_names
                .iter()
                .map(|n| json!({"name": n}))
                .collect::<Vec<_>>()
        ),
    );

    extra.insert(
        "models".to_string(),
        json!([{
            "kind": "azureOpenAI",
            "azureOpenAIParameters": {
                "resourceUri": ai.endpoint,
                "deploymentId": ai.chat.deployment,
                "modelName": ai.chat.model_name,
            }
        }]),
    );

    KnowledgeBase {
        name: mcp.knowledge_base.clone(),
        description: Some(format!(
            "Knowledge base for MCP instance (knowledge_base='{}')",
            mcp.knowledge_base
        )),
        storage_connection_string_secret: None,
        storage_container: None,
        identity: None,
        extra,
    }
}

// ---------------------------------------------------------------------------
// Naming + lookup helpers
// ---------------------------------------------------------------------------

/// Derive the skillset resource name for a Cosmos container.
fn skillset_name(container: &str) -> String {
    format!("{container}-vectorise")
}

/// Derive the knowledge-source resource name for a Cosmos container.
fn knowledge_source_name(container: &str) -> String {
    format!("{container}-ks")
}

/// Resolve an MCP `expose:` entry (e.g. `jira_issues`) to the actual Cosmos
/// container name, falling back to a hyphenated default
/// (`jira_issues` → `jira-issues`) if the user hasn't specified one.
fn container_name_for_kind(layout: &ContainerLayout, kind: &str) -> Option<String> {
    let explicit = match kind {
        "jira_issues" => layout.jira_issues.clone(),
        "jira_sprints" => layout.jira_sprints.clone(),
        "jira_fix_versions" => layout.jira_fix_versions.clone(),
        "jira_projects" => layout.jira_projects.clone(),
        "confluence_pages" => layout.confluence_pages.clone(),
        "confluence_spaces" => layout.confluence_spaces.clone(),
        _ => return None,
    };
    Some(explicit.unwrap_or_else(|| kind.replace('_', "-")))
}

/// Find the instance name for a borrowed [`McpInstance`]. Used only for
/// error messages — most call sites already have the [`InstanceConfig`].
fn instance_name_for_mcp<'a>(cfg: &'a Config, mcp: &McpInstance) -> &'a str {
    cfg.instances
        .iter()
        .find(|i| matches!(&i.spec, InstanceSpec::Mcp(m) if std::ptr::eq(m, mcp)))
        .map(|i| i.name.as_str())
        .unwrap_or("<unknown>")
}

/// Build a [`Field`] with all attribute slots set to `None`. Callers fill
/// in the bits they care about; this trims a lot of boilerplate.
fn blank_field(name: &str, field_type: &str) -> Field {
    Field {
        name: name.to_string(),
        field_type: field_type.to_string(),
        key: None,
        searchable: None,
        filterable: None,
        sortable: None,
        facetable: None,
        retrievable: None,
        stored: None,
        analyzer: None,
        search_analyzer: None,
        index_analyzer: None,
        synonym_maps: None,
        fields: None,
        dimensions: None,
        vector_search_profile: None,
        extra: Default::default(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Config {
        serde_yaml::from_str(yaml).expect("yaml parses")
    }

    const MCP_FIXTURE: &str = r#"
azure:
  cosmos:
    endpoint: https://x
    database: quelch
    containers:
      jira_issues: jira-issues
      confluence_pages: confluence-pages
    meta_container: quelch-meta
  search:
    endpoint: https://srv.search.windows.net
  ai:
    provider: foundry
    endpoint: https://ai.example
    embedding: { deployment: text-embedding-3-large, dimensions: 3072 }
    chat: { deployment: gpt-5-mini, model_name: gpt-5-mini }
source_connections: []
instances:
  - name: mcp-prod
    kind: mcp
    expose: [jira_issues, confluence_pages]
    api_key: K
    knowledge_base: kb-prod
    listen: 0.0.0.0:8080
"#;

    #[test]
    fn one_mcp_with_two_exposes_emits_2x_per_resource_and_one_kb() {
        let cfg = parse(MCP_FIXTURE);
        let state = generate(&cfg).expect("generate");

        assert_eq!(state.indexes.len(), 2, "one index per exposed kind");
        assert_eq!(state.indexers.len(), 2, "one indexer per exposed kind");
        assert_eq!(state.skillsets.len(), 2, "one skillset per exposed kind");
        assert_eq!(
            state.data_sources.len(),
            2,
            "one data source per exposed kind"
        );
        assert_eq!(state.knowledge_sources.len(), 2, "one KS per exposed kind");
        assert_eq!(state.knowledge_bases.len(), 1, "one KB per MCP instance");
    }

    #[test]
    fn index_names_match_container_names() {
        let cfg = parse(MCP_FIXTURE);
        let state = generate(&cfg).unwrap();
        let names: Vec<&str> = state.indexes.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"jira-issues"));
        assert!(names.contains(&"confluence-pages"));
    }

    #[test]
    fn knowledge_base_name_comes_from_mcp_instance_field() {
        let cfg = parse(MCP_FIXTURE);
        let state = generate(&cfg).unwrap();
        assert_eq!(state.knowledge_bases[0].name, "kb-prod");
    }

    #[test]
    fn no_mcp_instances_yields_empty_state() {
        let yaml = r#"
azure:
  cosmos:
    endpoint: https://x
    database: quelch
    meta_container: quelch-meta
source_connections: []
instances: []
"#;
        let cfg = parse(yaml);
        let state = generate(&cfg).expect("generate");
        assert!(state.indexes.is_empty());
        assert!(state.indexers.is_empty());
        assert!(state.skillsets.is_empty());
        assert!(state.data_sources.is_empty());
        assert!(state.knowledge_sources.is_empty());
        assert!(state.knowledge_bases.is_empty());
    }

    #[test]
    fn missing_ai_block_with_mcp_instance_is_error() {
        let yaml = r#"
azure:
  cosmos:
    endpoint: https://x
    database: quelch
    meta_container: quelch-meta
source_connections: []
instances:
  - name: m
    kind: mcp
    expose: [jira_issues]
    api_key: K
    knowledge_base: kb
    listen: 0.0.0.0:8080
"#;
        let cfg = parse(yaml);
        let err = generate(&cfg).unwrap_err();
        assert!(matches!(err, GenerateError::AiBlockMissing));
    }

    #[test]
    fn unknown_expose_kind_is_error() {
        let yaml = r#"
azure:
  cosmos:
    endpoint: https://x
    database: quelch
    meta_container: quelch-meta
  ai:
    provider: foundry
    endpoint: https://ai
    embedding: { deployment: e, dimensions: 3072 }
    chat: { deployment: c, model_name: c }
source_connections: []
instances:
  - name: m
    kind: mcp
    expose: [bogus_thing]
    api_key: K
    knowledge_base: kb
    listen: 0.0.0.0:8080
"#;
        let cfg = parse(yaml);
        let err = generate(&cfg).unwrap_err();
        match err {
            GenerateError::UnknownExposeKind { instance, kind } => {
                assert_eq!(instance, "m");
                assert_eq!(kind, "bogus_thing");
            }
            other => panic!("expected UnknownExposeKind, got {other:?}"),
        }
    }

    #[test]
    fn skillset_wires_in_embedding_deployment() {
        let cfg = parse(MCP_FIXTURE);
        let state = generate(&cfg).unwrap();
        let ss = state
            .skillsets
            .iter()
            .find(|s| s.name == "jira-issues-vectorise")
            .expect("skillset present");
        let skill = &ss.skills[0];
        assert_eq!(
            skill.extra.get("deploymentId").and_then(|v| v.as_str()),
            Some("text-embedding-3-large")
        );
    }

    #[test]
    fn knowledge_base_wires_in_chat_deployment_and_lists_knowledge_sources() {
        let cfg = parse(MCP_FIXTURE);
        let state = generate(&cfg).unwrap();
        let kb = &state.knowledge_bases[0];

        let models = kb.extra.get("models").expect("models present");
        let model = &models.as_array().unwrap()[0];
        assert_eq!(model["kind"], "azureOpenAI");
        assert_eq!(model["azureOpenAIParameters"]["deploymentId"], "gpt-5-mini");

        let ks = kb
            .extra
            .get("knowledgeSources")
            .and_then(|v| v.as_array())
            .expect("knowledgeSources present");
        let names: Vec<&str> = ks.iter().filter_map(|v| v["name"].as_str()).collect();
        assert!(names.contains(&"jira-issues-ks"));
        assert!(names.contains(&"confluence-pages-ks"));
    }

    #[test]
    fn duplicate_exposes_across_mcps_dont_duplicate_resources() {
        let yaml = r#"
azure:
  cosmos:
    endpoint: https://x
    database: quelch
    containers: { jira_issues: jira-issues }
    meta_container: quelch-meta
  ai:
    provider: foundry
    endpoint: https://ai
    embedding: { deployment: e, dimensions: 3072 }
    chat: { deployment: c, model_name: c }
source_connections: []
instances:
  - { name: a, kind: mcp, expose: [jira_issues], api_key: K,
      knowledge_base: kb-a, listen: 0.0.0.0:8080 }
  - { name: b, kind: mcp, expose: [jira_issues], api_key: K,
      knowledge_base: kb-b, listen: 0.0.0.0:8081 }
"#;
        let cfg = parse(yaml);
        let state = generate(&cfg).unwrap();
        // Two MCP instances → two KBs, but only one of each other resource.
        assert_eq!(state.knowledge_bases.len(), 2);
        assert_eq!(state.indexes.len(), 1);
        assert_eq!(state.indexers.len(), 1);
        assert_eq!(state.skillsets.len(), 1);
        assert_eq!(state.data_sources.len(), 1);
        assert_eq!(state.knowledge_sources.len(), 1);
    }
}
