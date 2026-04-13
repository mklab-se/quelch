pub mod state;

use anyhow::{Context, Result};
use std::path::Path;
use tracing::{error, info};

use crate::azure::SearchClient;
use crate::azure::schema::jira_index_schema;
use crate::config::{Config, SourceConfig};
use crate::sources::jira::JiraConnector;
use crate::sources::{SourceConnector, SyncCursor};

use self::state::SyncState;

/// Run a one-shot sync of all configured sources.
pub async fn run_sync(config: &Config, state_path: &Path) -> Result<()> {
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
            sync_with_connector(azure, &connector, source_config, config, state, state_path).await
        }
        SourceConfig::Confluence(_) => {
            anyhow::bail!("Confluence connector not yet implemented");
        }
    }
}

async fn sync_with_connector<C: SourceConnector>(
    azure: &SearchClient,
    connector: &C,
    source_config: &SourceConfig,
    config: &Config,
    state: &mut SyncState,
    state_path: &Path,
) -> Result<()> {
    let index_name = connector.index_name();
    let source_name = connector.source_name();

    // Ensure index exists with correct schema
    let schema = match source_config {
        SourceConfig::Jira(_) => jira_index_schema(index_name),
        SourceConfig::Confluence(_) => unreachable!(),
    };
    azure
        .ensure_index(&schema)
        .await
        .context("failed to ensure index exists")?;

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
