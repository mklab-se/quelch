pub mod state;

use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;
use tracing::{error, info};

use crate::azure::SearchClient;
use crate::azure::schema::{IndexSchema, confluence_index_schema, jira_index_schema};
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

/// Get the appropriate schema for a source config.
fn schema_for_source(source_config: &SourceConfig) -> IndexSchema {
    match source_config {
        SourceConfig::Jira(j) => jira_index_schema(&j.index),
        SourceConfig::Confluence(c) => confluence_index_schema(&c.index),
    }
}

/// Check and optionally create all indexes required by the config.
/// Returns the list of indexes that were created.
pub async fn setup_indexes(config: &Config, mode: IndexMode) -> Result<Vec<String>> {
    let azure = SearchClient::new(&config.azure.endpoint, &config.azure.api_key);
    let mut created = Vec::new();

    // Collect unique indexes with their schemas
    let mut seen = std::collections::HashSet::new();
    let mut schemas = Vec::new();
    for source in &config.sources {
        let schema = schema_for_source(source);
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
pub async fn run_sync(config: &Config, state_path: &Path, index_mode: IndexMode) -> Result<()> {
    // Ensure all indexes exist before syncing
    setup_indexes(config, index_mode).await?;

    let azure = SearchClient::new(&config.azure.endpoint, &config.azure.api_key);
    let mut state = SyncState::load(state_path)?;

    for source_config in &config.sources {
        if let Err(e) = sync_source(&azure, source_config, config, &mut state, state_path).await {
            error!(source = source_config.name(), error = %e, "Sync failed for source");
        }
    }

    Ok(())
}

async fn sync_source(
    azure: &SearchClient,
    source_config: &SourceConfig,
    config: &Config,
    state: &mut SyncState,
    state_path: &Path,
) -> Result<()> {
    match source_config {
        SourceConfig::Jira(jira_config) => {
            let connector = JiraConnector::new(jira_config);
            sync_with_connector(azure, &connector, config, state, state_path).await
        }
        SourceConfig::Confluence(conf_config) => {
            let connector = ConfluenceConnector::new(conf_config);
            sync_with_connector(azure, &connector, config, state, state_path).await
        }
    }
}

async fn sync_with_connector<C: SourceConnector>(
    azure: &SearchClient,
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

        // Convert SourceDocuments to JSON values for Azure
        let azure_docs: Vec<serde_json::Value> = result
            .documents
            .iter()
            .map(|doc| {
                serde_json::Value::Object(
                    doc.fields
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                )
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
            "Pushed batch to Azure AI Search"
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
