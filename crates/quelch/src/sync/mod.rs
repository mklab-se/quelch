pub mod state;

use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;
use tracing::{debug, error, info};

use crate::azure::SearchClient;
use crate::azure::schema::{EmbeddingConfig, confluence_index_schema, jira_index_schema};
use crate::config::{Config, SourceConfig};
use crate::sources::confluence::ConfluenceConnector;
use crate::sources::jira::JiraConnector;
use crate::sources::{SourceConnector, SyncCursor};

use self::state::SyncState;

/// Controls how missing indexes are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexMode {
    /// Prompt the user interactively before creating.
    Interactive,
    /// Auto-create missing indexes without prompting.
    AutoCreate,
    /// Fail if any index is missing (for CI/scripts).
    RequireExisting,
}

/// Load embedding configuration from ailloy.
/// Returns error if no embedding model is configured.
pub fn load_embedding_config() -> Result<EmbeddingConfig> {
    let config = ailloy::config::Config::load()
        .context("failed to load ailloy config — run 'quelch ai config' to set up AI")?;

    let (_id, node) = config
        .default_node_for("embedding")
        .context("no embedding model configured — run 'quelch ai config' to set one up")?;

    let metadata = node.embedding_metadata();

    let dimensions = metadata.dimensions.context(
        "embedding model has no dimensions configured — reconfigure with 'quelch ai config'",
    )?;

    let vectorizer_json = metadata
        .to_azure_search_vectorizer("quelch-vectorizer")
        .context("failed to generate vectorizer config — ensure you're using an Azure OpenAI embedding model")?;

    Ok(EmbeddingConfig {
        dimensions,
        vectorizer_json,
    })
}

/// Get the appropriate schema for a source config.
fn schema_for_source(
    source_config: &SourceConfig,
    embedding: &EmbeddingConfig,
) -> crate::azure::schema::IndexSchema {
    match source_config {
        SourceConfig::Jira(j) => jira_index_schema(&j.index, embedding),
        SourceConfig::Confluence(c) => confluence_index_schema(&c.index, embedding),
    }
}

/// Delete all indexes configured in the config, then clear sync state.
pub async fn reset_indexes(config: &Config, state_path: &Path) -> Result<Vec<String>> {
    let azure = SearchClient::new(&config.azure.endpoint, &config.azure.api_key);
    let mut deleted = Vec::new();

    // Collect unique index names
    let mut seen = std::collections::HashSet::new();
    for source in &config.sources {
        let index = source.index().to_string();
        if seen.insert(index.clone()) {
            let exists = azure
                .index_exists(&index)
                .await
                .with_context(|| format!("failed to check index '{}'", index))?;

            if exists {
                azure
                    .delete_index(&index)
                    .await
                    .with_context(|| format!("failed to delete index '{}'", index))?;
                println!("  [deleted] {}", index);
                deleted.push(index);
            } else {
                println!("  [absent]  {}", index);
            }
        }
    }

    // Clear sync state
    let mut state = SyncState::load(state_path)?;
    state.reset_all();
    state.save(state_path)?;
    println!("  [cleared] sync state");

    Ok(deleted)
}

/// Check and optionally create all indexes required by the config.
/// Returns the list of indexes that were created.
pub async fn setup_indexes(
    config: &Config,
    embedding: &EmbeddingConfig,
    mode: IndexMode,
) -> Result<Vec<String>> {
    let azure = SearchClient::new(&config.azure.endpoint, &config.azure.api_key);
    let mut created = Vec::new();

    // Collect unique indexes with their schemas
    let mut seen = std::collections::HashSet::new();
    let mut schemas = Vec::new();
    for source in &config.sources {
        let schema = schema_for_source(source, embedding);
        if seen.insert(schema.name.clone()) {
            schemas.push(schema);
        }
    }

    for schema in &schemas {
        let exists = azure
            .index_exists(&schema.name)
            .await
            .with_context(|| format!("failed to check index '{}'", schema.name))?;

        if exists {
            println!("  [exists]  {}", schema.name);
            continue;
        }

        let should_create = match mode {
            IndexMode::AutoCreate => true,
            IndexMode::RequireExisting => {
                anyhow::bail!(
                    "Index '{}' does not exist. Run 'quelch setup' to create it.",
                    schema.name
                );
            }
            IndexMode::Interactive => {
                print!("  [missing] {} — Create it? [y/N] ", schema.name);
                std::io::stdout().flush()?;
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                input.trim().eq_ignore_ascii_case("y")
            }
        };

        if should_create {
            azure
                .create_index(schema)
                .await
                .with_context(|| format!("failed to create index '{}'", schema.name))?;
            println!("  [created] {}", schema.name);
            created.push(schema.name.clone());
        } else {
            println!("  [skipped] {}", schema.name);
        }
    }

    Ok(created)
}

/// Run a one-shot sync of all configured sources.
/// If `embed_client` is None, documents are pushed without embeddings (for testing/mock mode).
pub async fn run_sync(
    config: &Config,
    state_path: &Path,
    embedding: &EmbeddingConfig,
    index_mode: IndexMode,
    embed_client: Option<&ailloy::Client>,
) -> Result<()> {
    // Ensure all indexes exist before syncing
    setup_indexes(config, embedding, index_mode).await?;

    let azure = SearchClient::new(&config.azure.endpoint, &config.azure.api_key);
    let mut state = SyncState::load(state_path)?;

    for source_config in &config.sources {
        if let Err(e) = sync_source(
            &azure,
            embed_client,
            source_config,
            config,
            &mut state,
            state_path,
        )
        .await
        {
            error!(source = source_config.name(), error = %e, "Sync failed for source");
        }
    }

    Ok(())
}

async fn sync_source(
    azure: &SearchClient,
    embed_client: Option<&ailloy::Client>,
    source_config: &SourceConfig,
    config: &Config,
    state: &mut SyncState,
    state_path: &Path,
) -> Result<()> {
    match source_config {
        SourceConfig::Jira(jira_config) => {
            let connector = JiraConnector::new(jira_config);
            sync_with_connector(azure, embed_client, &connector, config, state, state_path).await
        }
        SourceConfig::Confluence(conf_config) => {
            let connector = ConfluenceConnector::new(conf_config);
            sync_with_connector(azure, embed_client, &connector, config, state, state_path).await
        }
    }
}

async fn sync_with_connector<C: SourceConnector>(
    azure: &SearchClient,
    embed_client: Option<&ailloy::Client>,
    connector: &C,
    config: &Config,
    state: &mut SyncState,
    state_path: &Path,
) -> Result<()> {
    let index_name = connector.index_name();
    let source_name = connector.source_name();

    // Get cursor from persisted state
    let source_state = state.get_source(source_name);
    let cursor = source_state
        .last_cursor
        .map(|ts| SyncCursor { last_updated: ts });

    let mut total_synced: u64 = 0;

    loop {
        let result = connector
            .fetch_changes(cursor.as_ref(), config.sync.batch_size)
            .await
            .context("failed to fetch changes from source")?;

        let doc_count = result.documents.len() as u64;
        if doc_count == 0 {
            info!(source = source_name, "No changes since last sync");
            break;
        }

        // Generate embeddings if client is available
        let embeddings: Option<Vec<Vec<f32>>> = if let Some(client) = embed_client {
            let content_texts: Vec<&str> = result
                .documents
                .iter()
                .map(|doc| {
                    doc.fields
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                })
                .collect();

            debug!(
                source = source_name,
                count = content_texts.len(),
                "Generating embeddings"
            );
            let embed_response = client
                .embed(&content_texts)
                .await
                .context("failed to generate embeddings")?;
            Some(embed_response.embeddings)
        } else {
            None
        };

        // Convert SourceDocuments to JSON values, with embeddings if available
        let azure_docs: Vec<serde_json::Value> = result
            .documents
            .iter()
            .enumerate()
            .map(|(i, doc)| {
                let mut obj: serde_json::Map<String, serde_json::Value> = doc
                    .fields
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                if let Some(embedding) = embeddings.as_ref().and_then(|vecs| vecs.get(i)) {
                    obj.insert("content_vector".to_string(), serde_json::json!(embedding));
                }
                serde_json::Value::Object(obj)
            })
            .collect();

        // Push to Azure AI Search
        azure
            .push_documents(index_name, azure_docs)
            .await
            .context("failed to push documents to Azure AI Search")?;

        total_synced += doc_count;

        // Persist state immediately after each batch (crash safety)
        state.update_source(source_name, result.cursor.last_updated, doc_count);
        state
            .save(state_path)
            .context("failed to save sync state")?;

        info!(
            source = source_name,
            batch = doc_count,
            total = total_synced,
            "Pushed batch with embeddings to Azure AI Search"
        );

        if !result.has_more {
            break;
        }
    }

    if total_synced > 0 {
        info!(source = source_name, total = total_synced, "Sync complete");
    }

    Ok(())
}
