/// Generate rigg-format YAML files from a [`Config`].
///
/// The entry point is [`all`], which returns a [`GeneratedRiggFiles`] struct
/// containing the YAML content for every AI Search resource implied by the
/// config. Each entry maps a resource name (e.g. `"jira-issues"`) to its YAML
/// string.
///
/// The generated YAML is built by constructing the `rigg-core` native structs
/// and serialising them with `serde_yaml`. This avoids template drift: if
/// rigg-core's schema changes, the Rust compiler tells us.
use std::collections::HashMap;

use serde_json::json;
use thiserror::Error;

use rigg_core::resources::{
    datasource::{DataSource, DataSourceContainer, DataSourceCredentials},
    index::{Field, Index},
    indexer::{FieldMapping, Indexer, IndexerSchedule},
    knowledge_base::KnowledgeBase,
    knowledge_source::KnowledgeSource,
    skillset::{Skill, SkillInput, SkillOutput, Skillset},
};

use crate::config::{Config, OutputMode, ReasoningEffort, data_sources};

/// Errors that can occur during rigg file generation.
#[derive(Debug, Error)]
pub enum GenerateError {
    #[error("YAML serialisation failed for resource '{name}': {source}")]
    Serialise {
        name: String,
        source: serde_yaml::Error,
    },
}

/// In-memory generated rigg files, grouped by resource type.
///
/// Each map entry is `resource_name → YAML string`. The YAML is
/// parseable by `rigg-core`'s native structs, enabling round-trip tests.
#[derive(Debug, Default)]
pub struct GeneratedRiggFiles {
    /// AI Search index definitions. Key = container/index name.
    pub indexes: HashMap<String, String>,
    /// Skillset definitions. Key = container name (e.g. `"jira-issues-skillset"`).
    pub skillsets: HashMap<String, String>,
    /// Indexer definitions. Key = container name (e.g. `"jira-issues-indexer"`).
    pub indexers: HashMap<String, String>,
    /// Data source definitions pointing at Cosmos containers.
    pub datasources: HashMap<String, String>,
    /// Knowledge source definitions wrapping indexes for Agentic Retrieval.
    pub knowledge_sources: HashMap<String, String>,
    /// Knowledge base definitions grouping knowledge sources per MCP deployment.
    pub knowledge_bases: HashMap<String, String>,
}

/// Generate all rigg resource files from a config.
///
/// # Errors
///
/// Returns [`GenerateError`] if any resource fails to serialise to YAML.
pub fn all(config: &Config) -> Result<GeneratedRiggFiles, GenerateError> {
    let mut files = GeneratedRiggFiles::default();

    // Resolve the effective data sources (either explicit or auto-derived).
    let data_sources = data_sources::resolve(config);

    // Build one set of resources per physical container.
    // Each data source may have multiple backing containers.
    let mut ks_by_deployment: HashMap<String, Vec<String>> = HashMap::new();

    for (ds_name, ds) in &data_sources {
        for backed_by in &ds.backed_by {
            let container = &backed_by.container;

            // Generate index.
            let index = build_index(container, &ds.kind, config)?;
            let yaml = to_yaml(&index, container)?;
            files.indexes.insert(container.clone(), yaml);

            // Generate datasource.
            let datasource = build_datasource(container)?;
            let ds_name_str = format!("{container}-ds");
            let yaml = to_yaml(&datasource, &ds_name_str)?;
            files.datasources.insert(ds_name_str.clone(), yaml);

            // Generate skillset.
            let skillset = build_skillset(container, &ds.kind, config)?;
            let skillset_name = format!("{container}-skillset");
            let yaml = to_yaml(&skillset, &skillset_name)?;
            files.skillsets.insert(skillset_name.clone(), yaml);

            // Generate indexer.
            let indexer = build_indexer(container, &ds_name_str, &skillset_name, config)?;
            let indexer_name = format!("{container}-indexer");
            let yaml = to_yaml(&indexer, &indexer_name)?;
            files.indexers.insert(indexer_name, yaml);

            // Generate knowledge source.
            let ks = build_knowledge_source(container)?;
            let ks_name = format!("{container}-ks");
            let yaml = to_yaml(&ks, &ks_name)?;
            files.knowledge_sources.insert(ks_name.clone(), yaml);

            // Track which knowledge sources belong to which MCP deployment.
            // We'll use the logical data source name to match against expose lists.
            for deployment in &config.deployments {
                if let Some(expose) = &deployment.expose
                    && expose.contains(ds_name)
                {
                    ks_by_deployment
                        .entry(deployment.name.clone())
                        .or_default()
                        .push(ks_name.clone());
                }
            }
        }
    }

    // Generate one knowledge base per MCP deployment.
    for (deployment_name, ks_names) in &ks_by_deployment {
        let kb = build_knowledge_base(deployment_name, ks_names, config)?;
        let kb_name = format!("{deployment_name}-kb");
        let yaml = to_yaml_kb(&kb, &kb_name)?;
        files.knowledge_bases.insert(kb_name, yaml);
    }

    Ok(files)
}

// ---------------------------------------------------------------------------
// Index
// ---------------------------------------------------------------------------

/// Build an `Index` for the given container and source kind.
fn build_index(container: &str, kind: &str, config: &Config) -> Result<Index, GenerateError> {
    let dimensions = config.ai.embedding.dimensions as i32;
    let vector_profile = "default-vector-profile";

    let mut fields = build_common_fields();
    fields.extend(kind_specific_fields(kind));
    fields.push(build_vector_field(dimensions, vector_profile));

    let index = Index {
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
        semantic: Some(rigg_core::resources::index::SemanticConfiguration {
            default_configuration: Some("default-semantic".to_string()),
            configurations: Some(vec![json!({
                "name": "default-semantic",
                "prioritizedFields": {
                    "contentFields": kind_content_fields(kind),
                    "keywordsFields": kind_keywords_fields(kind)
                }
            })]),
        }),
        vector_search: Some(rigg_core::resources::index::VectorSearch {
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
                    "resourceUri": config.ai.endpoint,
                    "deploymentId": config.ai.embedding.deployment,
                    "modelName": config.ai.embedding.deployment
                }
            })]),
            compressions: None,
        }),
        extra: Default::default(),
    };

    Ok(index)
}

/// Fields present on every document regardless of source type.
fn build_common_fields() -> Vec<Field> {
    vec![
        Field {
            name: "id".to_string(),
            field_type: "Edm.String".to_string(),
            key: Some(true),
            searchable: Some(false),
            filterable: Some(true),
            sortable: Some(true),
            facetable: None,
            retrievable: Some(true),
            stored: None,
            analyzer: None,
            search_analyzer: None,
            index_analyzer: None,
            synonym_maps: None,
            fields: None,
            dimensions: None,
            vector_search_profile: None,
            extra: Default::default(),
        },
        simple_string("source_name", false, true, true, true),
        simple_string("source_link", false, true, false, true),
        simple_string("_partition_key", false, true, true, false),
        // Soft-delete fields used by the AI Search indexer deletion detection policy.
        Field {
            name: "_deleted".to_string(),
            field_type: "Edm.Boolean".to_string(),
            key: None,
            searchable: None,
            filterable: Some(true),
            sortable: None,
            facetable: None,
            retrievable: Some(true),
            stored: None,
            analyzer: None,
            search_analyzer: None,
            index_analyzer: None,
            synonym_maps: None,
            fields: None,
            dimensions: None,
            vector_search_profile: None,
            extra: Default::default(),
        },
        Field {
            name: "_deleted_at".to_string(),
            field_type: "Edm.DateTimeOffset".to_string(),
            key: None,
            searchable: None,
            filterable: Some(true),
            sortable: Some(true),
            facetable: None,
            retrievable: Some(true),
            stored: None,
            analyzer: None,
            search_analyzer: None,
            index_analyzer: None,
            synonym_maps: None,
            fields: None,
            dimensions: None,
            vector_search_profile: None,
            extra: Default::default(),
        },
        datetime_field("created"),
        datetime_field("updated"),
    ]
}

/// Kind-specific fields for the given source type.
fn kind_specific_fields(kind: &str) -> Vec<Field> {
    match kind {
        "jira_issue" => jira_issue_fields(),
        "jira_sprint" => jira_sprint_fields(),
        "jira_fix_version" => jira_fix_version_fields(),
        "jira_project" => jira_project_fields(),
        "confluence_page" => confluence_page_fields(),
        "confluence_space" => confluence_space_fields(),
        _ => vec![],
    }
}

fn jira_issue_fields() -> Vec<Field> {
    vec![
        simple_string("key", false, true, true, true),
        simple_string("project_key", false, true, true, true),
        simple_string("type", false, true, true, true),
        simple_string("status", false, true, true, true),
        simple_string("status_category", false, true, true, true),
        simple_string("priority", false, true, true, true),
        simple_string("resolution", false, true, true, false),
        datetime_field("resolved"),
        datetime_field("due_date"),
        searchable_string("summary"),
        searchable_string("description"),
        // Assignee and reporter as complex objects.
        complex_field(
            "assignee",
            vec![
                simple_string("id", false, true, false, false),
                simple_string("name", true, false, false, true),
                simple_string("email", false, true, false, false),
            ],
        ),
        complex_field(
            "reporter",
            vec![
                simple_string("id", false, true, false, false),
                simple_string("name", true, false, false, true),
                simple_string("email", false, true, false, false),
            ],
        ),
        string_collection("labels"),
        string_collection("components"),
        // Fix versions and affects versions as complex collections.
        Field {
            name: "fix_versions".to_string(),
            field_type: "Collection(Edm.ComplexType)".to_string(),
            key: None,
            searchable: None,
            filterable: None,
            sortable: None,
            facetable: None,
            retrievable: Some(true),
            stored: None,
            analyzer: None,
            search_analyzer: None,
            index_analyzer: None,
            synonym_maps: None,
            fields: Some(vec![
                simple_string("id", false, true, false, false),
                simple_string("name", false, true, false, true),
            ]),
            dimensions: None,
            vector_search_profile: None,
            extra: Default::default(),
        },
        Field {
            name: "affects_versions".to_string(),
            field_type: "Collection(Edm.ComplexType)".to_string(),
            key: None,
            searchable: None,
            filterable: None,
            sortable: None,
            facetable: None,
            retrievable: Some(true),
            stored: None,
            analyzer: None,
            search_analyzer: None,
            index_analyzer: None,
            synonym_maps: None,
            fields: Some(vec![
                simple_string("id", false, true, false, false),
                simple_string("name", false, true, false, true),
            ]),
            dimensions: None,
            vector_search_profile: None,
            extra: Default::default(),
        },
        // Sprint.
        complex_field(
            "sprint",
            vec![
                simple_string("id", false, true, false, false),
                simple_string("name", true, false, false, true),
                simple_string("state", false, true, true, true),
                datetime_field("start_date"),
                datetime_field("end_date"),
                searchable_string("goal"),
            ],
        ),
        // Parent.
        complex_field(
            "parent",
            vec![
                simple_string("id", false, true, false, false),
                simple_string("key", false, true, false, true),
                simple_string("type", false, true, false, true),
            ],
        ),
        simple_string("epic_link", false, true, false, true),
        // Issue links.
        Field {
            name: "issuelinks".to_string(),
            field_type: "Collection(Edm.ComplexType)".to_string(),
            key: None,
            searchable: None,
            filterable: None,
            sortable: None,
            facetable: None,
            retrievable: Some(true),
            stored: None,
            analyzer: None,
            search_analyzer: None,
            index_analyzer: None,
            synonym_maps: None,
            fields: Some(vec![
                simple_string("type", false, true, false, true),
                simple_string("direction", false, true, false, true),
                simple_string("target_key", false, true, false, true),
                searchable_string("target_summary"),
            ]),
            dimensions: None,
            vector_search_profile: None,
            extra: Default::default(),
        },
        // Comments.
        Field {
            name: "comments".to_string(),
            field_type: "Collection(Edm.ComplexType)".to_string(),
            key: None,
            searchable: None,
            filterable: None,
            sortable: None,
            facetable: None,
            retrievable: Some(true),
            stored: None,
            analyzer: None,
            search_analyzer: None,
            index_analyzer: None,
            synonym_maps: None,
            fields: Some(vec![
                simple_string("id", false, true, false, false),
                complex_field(
                    "author",
                    vec![
                        simple_string("id", false, true, false, false),
                        simple_string("name", true, false, false, true),
                        simple_string("email", false, true, false, false),
                    ],
                ),
                searchable_string("body"),
                datetime_field("created"),
                datetime_field("updated"),
            ]),
            dimensions: None,
            vector_search_profile: None,
            extra: Default::default(),
        },
        // Optional custom fields.
        Field {
            name: "story_points".to_string(),
            field_type: "Edm.Int64".to_string(),
            key: None,
            searchable: None,
            filterable: Some(true),
            sortable: Some(true),
            facetable: Some(true),
            retrievable: Some(true),
            stored: None,
            analyzer: None,
            search_analyzer: None,
            index_analyzer: None,
            synonym_maps: None,
            fields: None,
            dimensions: None,
            vector_search_profile: None,
            extra: Default::default(),
        },
    ]
}

fn jira_sprint_fields() -> Vec<Field> {
    vec![
        simple_string("key", false, true, true, true),
        simple_string("name", true, false, false, true),
        simple_string("state", false, true, true, true),
        datetime_field("start_date"),
        datetime_field("end_date"),
        datetime_field("complete_date"),
        searchable_string("goal"),
        string_collection("project_keys"),
        simple_string("board_id", false, true, false, true),
    ]
}

fn jira_fix_version_fields() -> Vec<Field> {
    vec![
        simple_string("name", true, true, true, true),
        searchable_string("description"),
        Field {
            name: "released".to_string(),
            field_type: "Edm.Boolean".to_string(),
            key: None,
            searchable: None,
            filterable: Some(true),
            sortable: None,
            facetable: Some(true),
            retrievable: Some(true),
            stored: None,
            analyzer: None,
            search_analyzer: None,
            index_analyzer: None,
            synonym_maps: None,
            fields: None,
            dimensions: None,
            vector_search_profile: None,
            extra: Default::default(),
        },
        datetime_field("release_date"),
        Field {
            name: "archived".to_string(),
            field_type: "Edm.Boolean".to_string(),
            key: None,
            searchable: None,
            filterable: Some(true),
            sortable: None,
            facetable: Some(true),
            retrievable: Some(true),
            stored: None,
            analyzer: None,
            search_analyzer: None,
            index_analyzer: None,
            synonym_maps: None,
            fields: None,
            dimensions: None,
            vector_search_profile: None,
            extra: Default::default(),
        },
        simple_string("project_key", false, true, true, true),
    ]
}

fn jira_project_fields() -> Vec<Field> {
    vec![
        simple_string("key", false, true, true, true),
        simple_string("name", true, true, true, true),
        searchable_string("description"),
        complex_field(
            "lead",
            vec![
                simple_string("id", false, true, false, false),
                simple_string("name", true, false, false, true),
                simple_string("email", false, true, false, false),
            ],
        ),
        simple_string("project_type_key", false, true, true, true),
        complex_field(
            "category",
            vec![
                simple_string("id", false, true, false, false),
                simple_string("name", false, true, false, true),
            ],
        ),
    ]
}

fn confluence_page_fields() -> Vec<Field> {
    vec![
        simple_string("space_key", false, true, true, true),
        simple_string("page_id", false, true, false, true),
        searchable_string("title"),
        searchable_string("body"),
        complex_field(
            "version",
            vec![
                Field {
                    name: "number".to_string(),
                    field_type: "Edm.Int64".to_string(),
                    key: None,
                    searchable: None,
                    filterable: Some(true),
                    sortable: Some(true),
                    facetable: None,
                    retrievable: Some(true),
                    stored: None,
                    analyzer: None,
                    search_analyzer: None,
                    index_analyzer: None,
                    synonym_maps: None,
                    fields: None,
                    dimensions: None,
                    vector_search_profile: None,
                    extra: Default::default(),
                },
                datetime_field("when"),
                complex_field(
                    "by",
                    vec![
                        simple_string("id", false, true, false, false),
                        simple_string("name", true, false, false, true),
                        simple_string("email", false, true, false, false),
                    ],
                ),
            ],
        ),
        Field {
            name: "ancestors".to_string(),
            field_type: "Collection(Edm.ComplexType)".to_string(),
            key: None,
            searchable: None,
            filterable: None,
            sortable: None,
            facetable: None,
            retrievable: Some(true),
            stored: None,
            analyzer: None,
            search_analyzer: None,
            index_analyzer: None,
            synonym_maps: None,
            fields: Some(vec![
                simple_string("id", false, true, false, false),
                searchable_string("title"),
            ]),
            dimensions: None,
            vector_search_profile: None,
            extra: Default::default(),
        },
        complex_field(
            "created_by",
            vec![
                simple_string("id", false, true, false, false),
                simple_string("name", true, false, false, true),
                simple_string("email", false, true, false, false),
            ],
        ),
        datetime_field("created_at"),
        complex_field(
            "updated_by",
            vec![
                simple_string("id", false, true, false, false),
                simple_string("name", true, false, false, true),
                simple_string("email", false, true, false, false),
            ],
        ),
        datetime_field("updated_at"),
        string_collection("labels"),
    ]
}

fn confluence_space_fields() -> Vec<Field> {
    vec![
        simple_string("key", false, true, true, true),
        simple_string("name", true, true, true, true),
        searchable_string("description"),
        simple_string("type", false, true, true, true),
        simple_string("homepage_id", false, true, false, true),
    ]
}

/// Content fields used in semantic configuration for the given kind.
fn kind_content_fields(kind: &str) -> Vec<serde_json::Value> {
    match kind {
        "jira_issue" => vec![json!({"name": "summary"}), json!({"name": "description"})],
        "jira_sprint" => vec![json!({"name": "goal"})],
        "confluence_page" => vec![json!({"name": "title"}), json!({"name": "body"})],
        "jira_project" => vec![json!({"name": "description"})],
        "jira_fix_version" => vec![json!({"name": "description"})],
        "confluence_space" => vec![json!({"name": "description"})],
        _ => vec![],
    }
}

/// Keywords fields used in semantic configuration for the given kind.
fn kind_keywords_fields(kind: &str) -> Vec<serde_json::Value> {
    match kind {
        "jira_issue" => vec![
            json!({"name": "key"}),
            json!({"name": "status"}),
            json!({"name": "priority"}),
        ],
        "jira_sprint" => vec![json!({"name": "name"}), json!({"name": "state"})],
        "confluence_page" => vec![json!({"name": "space_key"})],
        "jira_project" => vec![json!({"name": "key"}), json!({"name": "name"})],
        "jira_fix_version" | "confluence_space" => vec![json!({"name": "name"})],
        _ => vec![],
    }
}

/// Build the `content_vector` field for integrated vectorisation.
fn build_vector_field(dimensions: i32, profile: &str) -> Field {
    Field {
        name: "content_vector".to_string(),
        field_type: "Collection(Edm.Single)".to_string(),
        key: None,
        searchable: Some(true),
        filterable: None,
        sortable: None,
        facetable: None,
        retrievable: None,
        stored: Some(true),
        analyzer: None,
        search_analyzer: None,
        index_analyzer: None,
        synonym_maps: None,
        fields: None,
        dimensions: Some(dimensions),
        vector_search_profile: Some(profile.to_string()),
        extra: Default::default(),
    }
}

// ---------------------------------------------------------------------------
// Datasource
// ---------------------------------------------------------------------------

/// Build a `DataSource` pointing at a Cosmos DB container.
///
/// The connection string is a Key Vault placeholder; the actual value is
/// populated by the Bicep generator (Phase 5) at deploy time.
fn build_datasource(container: &str) -> Result<DataSource, GenerateError> {
    let ds = DataSource {
        name: format!("{container}-ds"),
        datasource_type: "cosmosdb".to_string(),
        credentials: DataSourceCredentials {
            // Key Vault secret reference — Bicep fills in the real connection string.
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
    };

    Ok(ds)
}

// ---------------------------------------------------------------------------
// Skillset
// ---------------------------------------------------------------------------

/// Build a skillset that vectorises the primary text fields of a container.
fn build_skillset(container: &str, kind: &str, config: &Config) -> Result<Skillset, GenerateError> {
    // Merge primary text fields into a merged_text field.
    let source_fields = kind_text_source_fields(kind);
    let inputs: Vec<SkillInput> = source_fields
        .iter()
        .enumerate()
        .map(|(i, field)| SkillInput {
            name: format!("text{i}"),
            source: format!("/document/{field}"),
            source_context: None,
            inputs: None,
        })
        .collect();

    // Text merge skill: combine primary text fields into one.
    let merge_skill = Skill {
        odata_type: "#Microsoft.Skills.Text.MergeSkill".to_string(),
        name: "merge-text".to_string(),
        description: Some("Merge primary text fields for vectorisation".to_string()),
        context: Some("/document".to_string()),
        inputs: {
            let mut merge_inputs = vec![SkillInput {
                name: "itemsToInsert".to_string(),
                source: "/document/content".to_string(),
                source_context: None,
                inputs: None,
            }];
            merge_inputs.extend(inputs);
            merge_inputs
        },
        outputs: vec![SkillOutput {
            name: "mergedText".to_string(),
            target_name: Some("merged_text_for_embedding".to_string()),
        }],
        extra: Default::default(),
    };

    // Azure OpenAI embedding skill.
    let embedding_skill = Skill {
        odata_type: "#Microsoft.Skills.Text.AzureOpenAIEmbeddingSkill".to_string(),
        name: "azure-openai-embedding".to_string(),
        description: Some("Compute embeddings via Azure OpenAI".to_string()),
        context: Some("/document".to_string()),
        inputs: vec![SkillInput {
            name: "text".to_string(),
            source: "/document/merged_text_for_embedding".to_string(),
            source_context: None,
            inputs: None,
        }],
        outputs: vec![SkillOutput {
            name: "embedding".to_string(),
            target_name: Some("content_vector".to_string()),
        }],
        extra: {
            let mut m = std::collections::HashMap::new();
            m.insert("resourceUri".to_string(), json!(config.ai.endpoint));
            m.insert(
                "deploymentId".to_string(),
                json!(config.ai.embedding.deployment),
            );
            m.insert(
                "modelName".to_string(),
                json!(config.ai.embedding.deployment),
            );
            m
        },
    };

    let skillset = Skillset {
        name: format!("{container}-skillset"),
        description: Some(format!(
            "Integrated vectorisation skillset for the '{container}' index"
        )),
        skills: vec![merge_skill, embedding_skill],
        cognitive_services: None,
        knowledge_store: None,
        index_projections: None,
        encryption_key: None,
        extra: Default::default(),
    };

    Ok(skillset)
}

/// Primary text fields for a given source kind, used as embedding input.
fn kind_text_source_fields(kind: &str) -> Vec<&'static str> {
    match kind {
        "jira_issue" => vec!["summary", "description"],
        "jira_sprint" => vec!["name", "goal"],
        "jira_fix_version" => vec!["name", "description"],
        "jira_project" => vec!["name", "description"],
        "confluence_page" => vec!["title", "body"],
        "confluence_space" => vec!["name", "description"],
        _ => vec![],
    }
}

// ---------------------------------------------------------------------------
// Indexer
// ---------------------------------------------------------------------------

/// Build an `Indexer` that pulls from a Cosmos datasource into an index.
fn build_indexer(
    container: &str,
    datasource_name: &str,
    skillset_name: &str,
    config: &Config,
) -> Result<rigg_core::resources::indexer::Indexer, GenerateError> {
    let interval = config.search.indexer.schedule.interval.clone();

    // Output field mapping: write the skillset's content_vector output to the index field.
    let output_field_mappings = vec![FieldMapping {
        source_field_name: "/document/content_vector".to_string(),
        target_field_name: Some("content_vector".to_string()),
        mapping_function: None,
    }];

    let indexer = Indexer {
        name: format!("{container}-indexer"),
        data_source_name: datasource_name.to_string(),
        target_index_name: container.to_string(),
        skillset_name: Some(skillset_name.to_string()),
        description: Some(format!(
            "Indexer pulling '{container}' from Cosmos DB into AI Search"
        )),
        schedule: Some(IndexerSchedule {
            interval,
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
        output_field_mappings: Some(output_field_mappings),
        disabled: None,
        cache: None,
        encryption_key: None,
        extra: Default::default(),
    };

    Ok(indexer)
}

// ---------------------------------------------------------------------------
// Knowledge Source
// ---------------------------------------------------------------------------

/// Build a `KnowledgeSource` that wraps an index for Agentic Retrieval.
fn build_knowledge_source(container: &str) -> Result<KnowledgeSource, GenerateError> {
    let ks = KnowledgeSource {
        name: format!("{container}-ks"),
        index_name: container.to_string(),
        description: Some(format!(
            "Knowledge source wrapping the '{container}' index for Agentic Retrieval"
        )),
        knowledge_base_name: None, // Set separately by the KB generator if needed.
        query_type: Some("semantic".to_string()),
        semantic_configuration: Some("default-semantic".to_string()),
        top: Some(5),
        filter: None,
        select_fields: None,
        extra: Default::default(),
    };

    Ok(ks)
}

// ---------------------------------------------------------------------------
// Knowledge Base
// ---------------------------------------------------------------------------

/// Build a `KnowledgeBase` grouping knowledge sources for an MCP deployment.
///
/// The KB is wired up with:
/// - `knowledgeSources`: one reference per data source exposed by the MCP.
/// - `models[]`: a single `azureOpenAI`-kind model pointing at the chat
///   deployment configured in `ai.chat`.  AI Search uses this for query
///   planning and (when `output_mode: answerSynthesis`) answer formulation.
/// - `retrievalReasoningEffort`: minimal/low/medium per the Knowledge Base
///   preview API.
/// - `outputMode`: answerSynthesis or extractedData.
///
/// The model JSON shape is the same for both Azure OpenAI accounts and
/// Microsoft Foundry projects — only the `resourceUri` (= `ai.endpoint`)
/// differs.  Authentication is expected to flow through the search service's
/// managed identity (no `apiKey` is emitted).
fn build_knowledge_base(
    deployment_name: &str,
    ks_names: &[String],
    config: &Config,
) -> Result<KnowledgeBase, GenerateError> {
    let mut extra = std::collections::HashMap::new();
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
                "resourceUri": config.ai.endpoint,
                "deploymentId": config.ai.chat.deployment,
                "modelName": config.ai.chat.model_name,
            }
        }]),
    );

    let effort = match config.ai.chat.retrieval_reasoning_effort {
        ReasoningEffort::Minimal => "minimal",
        ReasoningEffort::Low => "low",
        ReasoningEffort::Medium => "medium",
    };
    extra.insert(
        "retrievalReasoningEffort".to_string(),
        json!({ "kind": effort }),
    );

    let output_mode = match config.ai.chat.output_mode {
        OutputMode::AnswerSynthesis => "answerSynthesis",
        OutputMode::ExtractedData => "extractedData",
    };
    extra.insert("outputMode".to_string(), json!(output_mode));

    let kb = KnowledgeBase {
        name: format!("{deployment_name}-kb"),
        description: Some(format!(
            "Knowledge base for '{deployment_name}' MCP deployment"
        )),
        storage_connection_string_secret: None,
        storage_container: None,
        identity: None,
        extra,
    };

    Ok(kb)
}

// ---------------------------------------------------------------------------
// Field helpers
// ---------------------------------------------------------------------------

/// A simple string field with no text analysis.
fn simple_string(
    name: &str,
    searchable: bool,
    filterable: bool,
    facetable: bool,
    retrievable: bool,
) -> Field {
    Field {
        name: name.to_string(),
        field_type: "Edm.String".to_string(),
        key: None,
        searchable: if searchable { Some(true) } else { None },
        filterable: if filterable { Some(true) } else { None },
        sortable: if filterable { Some(true) } else { None },
        facetable: if facetable { Some(true) } else { None },
        retrievable: if retrievable { Some(true) } else { None },
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

/// A searchable string field with the standard Lucene analyser.
fn searchable_string(name: &str) -> Field {
    Field {
        name: name.to_string(),
        field_type: "Edm.String".to_string(),
        key: None,
        searchable: Some(true),
        filterable: None,
        sortable: None,
        facetable: None,
        retrievable: Some(true),
        stored: None,
        analyzer: Some("standard.lucene".to_string()),
        search_analyzer: None,
        index_analyzer: None,
        synonym_maps: None,
        fields: None,
        dimensions: None,
        vector_search_profile: None,
        extra: Default::default(),
    }
}

/// A `Collection(Edm.String)` field for arrays of strings.
fn string_collection(name: &str) -> Field {
    Field {
        name: name.to_string(),
        field_type: "Collection(Edm.String)".to_string(),
        key: None,
        searchable: None,
        filterable: Some(true),
        sortable: None,
        facetable: Some(true),
        retrievable: Some(true),
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

/// A `Edm.DateTimeOffset` field.
fn datetime_field(name: &str) -> Field {
    Field {
        name: name.to_string(),
        field_type: "Edm.DateTimeOffset".to_string(),
        key: None,
        searchable: None,
        filterable: Some(true),
        sortable: Some(true),
        facetable: None,
        retrievable: Some(true),
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

/// A `Edm.ComplexType` field with sub-fields.
fn complex_field(name: &str, subfields: Vec<Field>) -> Field {
    Field {
        name: name.to_string(),
        field_type: "Edm.ComplexType".to_string(),
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
        fields: Some(subfields),
        dimensions: None,
        vector_search_profile: None,
        extra: Default::default(),
    }
}

// ---------------------------------------------------------------------------
// Serialisation helpers
// ---------------------------------------------------------------------------

/// Serialise a resource to YAML.
fn to_yaml<T: serde::Serialize>(value: &T, name: &str) -> Result<String, GenerateError> {
    serde_yaml::to_string(value).map_err(|e| GenerateError::Serialise {
        name: name.to_string(),
        source: e,
    })
}

/// Serialise a `KnowledgeBase` to YAML (same as `to_yaml` but explicit for clarity).
fn to_yaml_kb(value: &KnowledgeBase, name: &str) -> Result<String, GenerateError> {
    to_yaml(value, name)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    // ---------------------------------------------------------------------------
    // Test helpers
    // ---------------------------------------------------------------------------

    fn parse_config(yaml: &str) -> Config {
        serde_yaml::from_str(yaml).expect("yaml must parse")
    }

    const BASE_CONFIG: &str = r#"
azure:
  subscription_id: "sub-test"
  resource_group: "rg-test"
  region: "swedencentral"
cosmos:
  database: "quelch"
ai:
  provider: azure_openai
  endpoint: "https://test.openai.azure.com"
  embedding:
    deployment: "text-embedding-3-large"
    dimensions: 3072
  chat:
    deployment: "gpt-5-mini"
    model_name: "gpt-5-mini"
"#;

    fn config_with_jira_cloud() -> Config {
        let yaml = format!(
            r#"{BASE_CONFIG}
sources:
  - type: jira
    name: jira-cloud
    url: "https://example.atlassian.net"
    auth:
      email: "u@example.com"
      api_token: "tok"
    projects: ["DO"]
deployments:
  - name: mcp
    role: mcp
    target: azure
    expose:
      - jira_issues
      - jira_sprints
      - jira_fix_versions
      - jira_projects
    auth:
      mode: "api_key"
"#
        );
        parse_config(&yaml)
    }

    fn config_with_confluence() -> Config {
        let yaml = format!(
            r#"{BASE_CONFIG}
sources:
  - type: confluence
    name: confluence-cloud
    url: "https://example.atlassian.net/wiki"
    auth:
      email: "u@example.com"
      api_token: "tok"
    spaces: ["ENG"]
deployments:
  - name: mcp
    role: mcp
    target: azure
    expose:
      - confluence_pages
      - confluence_spaces
    auth:
      mode: "api_key"
"#
        );
        parse_config(&yaml)
    }

    fn config_with_jira_and_confluence() -> Config {
        let yaml = format!(
            r#"{BASE_CONFIG}
sources:
  - type: jira
    name: jira-cloud
    url: "https://example.atlassian.net"
    auth:
      email: "u@example.com"
      api_token: "tok"
    projects: ["DO"]
  - type: confluence
    name: confluence-cloud
    url: "https://example.atlassian.net/wiki"
    auth:
      email: "u@example.com"
      api_token: "tok"
    spaces: ["ENG"]
deployments:
  - name: mcp
    role: mcp
    target: azure
    expose:
      - jira_issues
      - confluence_pages
    auth:
      mode: "api_key"
"#
        );
        parse_config(&yaml)
    }

    // ---------------------------------------------------------------------------
    // Index tests
    // ---------------------------------------------------------------------------

    #[test]
    fn generates_jira_issues_index_with_canonical_fields() {
        let cfg = config_with_jira_cloud();
        let files = all(&cfg).unwrap();

        let idx_yaml = files.indexes.get("jira-issues").unwrap();

        // Round-trip via rigg-core.
        let idx: Index = serde_yaml::from_str(idx_yaml).unwrap();

        let field_names: Vec<&str> = idx.fields.iter().map(|f| f.name.as_str()).collect();

        assert!(field_names.contains(&"id"), "missing 'id' field");
        assert!(field_names.contains(&"key"), "missing 'key' field");
        assert!(
            field_names.contains(&"status_category"),
            "missing 'status_category' field"
        );
        assert!(
            field_names.contains(&"issuelinks"),
            "missing 'issuelinks' field"
        );
        assert!(field_names.contains(&"summary"), "missing 'summary' field");
        assert!(
            field_names.contains(&"description"),
            "missing 'description' field"
        );
        assert!(
            field_names.contains(&"content_vector"),
            "missing 'content_vector' vector field"
        );
        assert!(field_names.contains(&"sprint"), "missing 'sprint' field");
        assert!(
            field_names.contains(&"issuelinks"),
            "missing 'issuelinks' field"
        );
        assert!(
            field_names.contains(&"comments"),
            "missing 'comments' field"
        );

        // Check the key field has key=true.
        let id_field = idx.fields.iter().find(|f| f.name == "id").unwrap();
        assert_eq!(id_field.key, Some(true));

        // Check the vector field dimensions.
        let vec_field = idx
            .fields
            .iter()
            .find(|f| f.name == "content_vector")
            .unwrap();
        assert_eq!(vec_field.dimensions, Some(3072));
    }

    #[test]
    fn generates_confluence_pages_index_with_canonical_fields() {
        let cfg = config_with_confluence();
        let files = all(&cfg).unwrap();

        let idx_yaml = files.indexes.get("confluence-pages").unwrap();
        let idx: Index = serde_yaml::from_str(idx_yaml).unwrap();

        let field_names: Vec<&str> = idx.fields.iter().map(|f| f.name.as_str()).collect();

        assert!(field_names.contains(&"title"), "missing 'title'");
        assert!(field_names.contains(&"body"), "missing 'body'");
        assert!(field_names.contains(&"space_key"), "missing 'space_key'");
        assert!(field_names.contains(&"page_id"), "missing 'page_id'");
        assert!(
            field_names.contains(&"content_vector"),
            "missing 'content_vector'"
        );
        assert!(field_names.contains(&"ancestors"), "missing 'ancestors'");

        // title should be searchable with Lucene analyser.
        let title_field = idx.fields.iter().find(|f| f.name == "title").unwrap();
        assert_eq!(
            title_field.analyzer.as_deref(),
            Some("standard.lucene"),
            "title field should use standard.lucene analyser"
        );
    }

    #[test]
    fn index_has_semantic_and_vector_search_config() {
        let cfg = config_with_jira_cloud();
        let files = all(&cfg).unwrap();
        let idx: Index = serde_yaml::from_str(files.indexes.get("jira-issues").unwrap()).unwrap();

        assert!(
            idx.semantic.is_some(),
            "index must have semantic configuration"
        );
        assert!(
            idx.vector_search.is_some(),
            "index must have vector_search configuration"
        );
    }

    // ---------------------------------------------------------------------------
    // Skillset tests
    // ---------------------------------------------------------------------------

    #[test]
    fn generates_skillset_referencing_openai_endpoint() {
        let cfg = config_with_jira_cloud();
        let files = all(&cfg).unwrap();

        let ss_yaml = files.skillsets.get("jira-issues-skillset").unwrap();
        let ss: Skillset = serde_yaml::from_str(ss_yaml).unwrap();

        assert_eq!(ss.name, "jira-issues-skillset");
        assert!(
            !ss.skills.is_empty(),
            "skillset must have at least one skill"
        );

        // Find the embedding skill.
        let embedding = ss
            .skills
            .iter()
            .find(|s| s.odata_type.contains("AzureOpenAIEmbeddingSkill"))
            .expect("must have an AzureOpenAIEmbeddingSkill");

        let resource_uri = embedding
            .extra
            .get("resourceUri")
            .and_then(|v| v.as_str())
            .expect("embedding skill must have resourceUri");

        assert_eq!(
            resource_uri, "https://test.openai.azure.com",
            "skillset must reference the configured OpenAI endpoint"
        );

        let deployment = embedding
            .extra
            .get("deploymentId")
            .and_then(|v| v.as_str())
            .expect("embedding skill must have deploymentId");

        assert_eq!(
            deployment, "text-embedding-3-large",
            "skillset must reference the configured embedding deployment"
        );
    }

    // ---------------------------------------------------------------------------
    // Indexer tests
    // ---------------------------------------------------------------------------

    #[test]
    fn generates_indexer_with_soft_delete_column_policy() {
        let cfg = config_with_jira_cloud();
        let files = all(&cfg).unwrap();

        let ds_yaml = files.datasources.get("jira-issues-ds").unwrap();
        let ds: DataSource = serde_yaml::from_str(ds_yaml).unwrap();

        // The soft-delete policy is on the datasource.
        let deletion_policy = ds
            .data_deletion_detection_policy
            .as_ref()
            .expect("datasource must have data_deletion_detection_policy");

        let col_name = deletion_policy
            .get("softDeleteColumnName")
            .and_then(|v| v.as_str())
            .expect("must have softDeleteColumnName");

        assert_eq!(
            col_name, "_deleted",
            "soft-delete column must be '_deleted'"
        );

        let marker_value = deletion_policy
            .get("softDeleteMarkerValue")
            .and_then(|v| v.as_str())
            .expect("must have softDeleteMarkerValue");

        assert_eq!(
            marker_value, "true",
            "soft-delete marker value must be 'true'"
        );
    }

    #[test]
    fn generates_indexer_pointing_at_datasource_and_index() {
        let cfg = config_with_jira_cloud();
        let files = all(&cfg).unwrap();

        let idx_yaml = files.indexers.get("jira-issues-indexer").unwrap();
        let indexer: rigg_core::resources::indexer::Indexer =
            serde_yaml::from_str(idx_yaml).unwrap();

        assert_eq!(indexer.data_source_name, "jira-issues-ds");
        assert_eq!(indexer.target_index_name, "jira-issues");
        assert_eq!(
            indexer.skillset_name.as_deref(),
            Some("jira-issues-skillset")
        );

        let schedule = indexer
            .schedule
            .as_ref()
            .expect("indexer must have schedule");
        assert_eq!(schedule.interval, "PT15M");
    }

    // ---------------------------------------------------------------------------
    // Datasource tests
    // ---------------------------------------------------------------------------

    #[test]
    fn generates_datasource_pointing_at_cosmos_container() {
        let cfg = config_with_jira_cloud();
        let files = all(&cfg).unwrap();

        let ds_yaml = files.datasources.get("jira-issues-ds").unwrap();
        let ds: DataSource = serde_yaml::from_str(ds_yaml).unwrap();

        assert_eq!(ds.datasource_type, "cosmosdb");
        assert_eq!(ds.container.name, "jira-issues");

        let query = ds
            .container
            .query
            .as_deref()
            .expect("container must have query");
        assert!(
            query.contains("@HighWaterMark"),
            "query must use @HighWaterMark for incremental pull"
        );

        // Connection string is a Key Vault placeholder.
        let conn_str = ds
            .credentials
            .connection_string
            .as_deref()
            .expect("must have connection_string");
        assert!(
            conn_str.contains("KeyVault"),
            "connection string must be a Key Vault reference placeholder"
        );
    }

    // ---------------------------------------------------------------------------
    // Knowledge Source tests
    // ---------------------------------------------------------------------------

    #[test]
    fn generates_knowledge_source_wrapping_index() {
        let cfg = config_with_jira_cloud();
        let files = all(&cfg).unwrap();

        let ks_yaml = files.knowledge_sources.get("jira-issues-ks").unwrap();
        let ks: KnowledgeSource = serde_yaml::from_str(ks_yaml).unwrap();

        assert_eq!(ks.name, "jira-issues-ks");
        assert_eq!(ks.index_name, "jira-issues");
        assert_eq!(ks.query_type.as_deref(), Some("semantic"));
    }

    // ---------------------------------------------------------------------------
    // Knowledge Base tests
    // ---------------------------------------------------------------------------

    #[test]
    fn generates_knowledge_base_grouping_exposed_data_sources() {
        let cfg = config_with_jira_and_confluence();
        let files = all(&cfg).unwrap();

        // The 'mcp' deployment exposes jira_issues and confluence_pages.
        let kb_yaml = files.knowledge_bases.get("mcp-kb").unwrap();
        let kb: KnowledgeBase = serde_yaml::from_str(kb_yaml).unwrap();

        assert_eq!(kb.name, "mcp-kb");

        let ks_array = kb
            .extra
            .get("knowledgeSources")
            .expect("knowledge base must have knowledgeSources")
            .as_array()
            .expect("knowledgeSources must be an array");

        assert!(
            !ks_array.is_empty(),
            "knowledge base must reference at least one knowledge source"
        );

        let ks_names: Vec<&str> = ks_array
            .iter()
            .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
            .collect();

        assert!(
            ks_names.contains(&"jira-issues-ks"),
            "knowledge base must include jira-issues-ks, got: {ks_names:?}"
        );
        assert!(
            ks_names.contains(&"confluence-pages-ks"),
            "knowledge base must include confluence-pages-ks, got: {ks_names:?}"
        );
    }

    #[test]
    fn all_resource_types_are_generated_for_full_config() {
        let cfg = config_with_jira_and_confluence();
        let files = all(&cfg).unwrap();

        // jira-issues and confluence-pages should both be present.
        assert!(!files.indexes.is_empty(), "no indexes generated");
        assert!(!files.datasources.is_empty(), "no datasources generated");
        assert!(!files.skillsets.is_empty(), "no skillsets generated");
        assert!(!files.indexers.is_empty(), "no indexers generated");
        assert!(
            !files.knowledge_sources.is_empty(),
            "no knowledge sources generated"
        );
        assert!(
            !files.knowledge_bases.is_empty(),
            "no knowledge bases generated"
        );
    }
}
