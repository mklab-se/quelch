pub mod state;

use anyhow::{Context, Result};
use chrono::Timelike;
use std::io::Write;
use std::path::Path;
use tracing::{debug, error, info, warn};

use crate::azure::SearchClient;
use crate::azure::schema::{EmbeddingConfig, confluence_index_schema, jira_index_schema};
use crate::config::{Config, SourceConfig};
use crate::sources::confluence::ConfluenceConnector;
use crate::sources::jira::JiraConnector;
use crate::sources::{SourceConnector, SyncCursor};
use crate::text::truncate_for_display;

use self::state::SyncState;

fn format_error_chain(error: &anyhow::Error) -> String {
    error
        .chain()
        .map(|cause| cause.to_string())
        .collect::<Vec<_>>()
        .join(": ")
}

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
/// If `max_docs` is Some, stop after syncing that many documents per source.
pub async fn run_sync(
    config: &Config,
    state_path: &Path,
    embedding: &EmbeddingConfig,
    index_mode: IndexMode,
    embed_client: Option<&ailloy::Client>,
    max_docs: Option<u64>,
) -> Result<()> {
    // Ensure all indexes exist before syncing
    setup_indexes(config, embedding, index_mode).await?;

    let azure = SearchClient::new(&config.azure.endpoint, &config.azure.api_key);
    let mut state = SyncState::load(state_path)?;
    let mut failures = Vec::new();

    for source_config in &config.sources {
        if let Err(e) = sync_source(
            &azure,
            embed_client,
            source_config,
            config,
            &mut state,
            state_path,
            max_docs,
        )
        .await
        {
            let error_chain = format_error_chain(&e);
            error!(
                source = source_config.name(),
                error = %error_chain,
                "Sync failed for source"
            );
            failures.push(format!("{}: {}", source_config.name(), error_chain));
        }
    }

    if !failures.is_empty() {
        anyhow::bail!(
            "sync failed for {} source(s): {}",
            failures.len(),
            failures.join(" | ")
        );
    }

    Ok(())
}

/// Purge orphaned documents from all configured indexes.
/// Compares source IDs with indexed IDs and removes any that no longer exist in the source.
pub async fn run_purge(config: &Config) -> Result<()> {
    let azure = SearchClient::new(&config.azure.endpoint, &config.azure.api_key);

    for source_config in &config.sources {
        if let Err(e) = purge_source(&azure, source_config).await {
            error!(source = source_config.name(), error = %e, "Purge failed for source");
        }
    }

    Ok(())
}

async fn purge_source(azure: &SearchClient, source_config: &SourceConfig) -> Result<()> {
    match source_config {
        SourceConfig::Jira(jira_config) => {
            let connector = JiraConnector::new(jira_config);
            purge_with_connector(azure, &connector).await
        }
        SourceConfig::Confluence(conf_config) => {
            let connector = ConfluenceConnector::new(conf_config);
            purge_with_connector(azure, &connector).await
        }
    }
}

async fn purge_with_connector<C: SourceConnector>(
    azure: &SearchClient,
    connector: &C,
) -> Result<()> {
    let source_name = connector.source_name();
    let index_name = connector.index_name();

    info!(source = source_name, "Starting orphan detection");

    // Fetch all IDs from the source
    let source_ids: std::collections::HashSet<String> = connector
        .fetch_all_ids()
        .await
        .context("failed to fetch IDs from source")?
        .into_iter()
        .collect();

    // Fetch all IDs from the Azure index
    let index_ids = azure
        .fetch_all_ids(index_name)
        .await
        .context("failed to fetch IDs from Azure index")?;

    // Find orphans: IDs in index but not in source
    let orphans: Vec<String> = index_ids
        .into_iter()
        .filter(|id| !source_ids.contains(id))
        .collect();

    if orphans.is_empty() {
        info!(source = source_name, "No orphaned documents found");
        return Ok(());
    }

    info!(
        source = source_name,
        orphans = orphans.len(),
        "Removing orphaned documents"
    );

    // Delete in batches of 1000 (Azure limit)
    for chunk in orphans.chunks(1000) {
        azure
            .delete_documents(index_name, chunk)
            .await
            .context("failed to delete orphaned documents")?;
    }

    info!(
        source = source_name,
        removed = orphans.len(),
        "Purge complete"
    );

    Ok(())
}

async fn sync_source(
    azure: &SearchClient,
    embed_client: Option<&ailloy::Client>,
    source_config: &SourceConfig,
    config: &Config,
    state: &mut SyncState,
    state_path: &Path,
    max_docs: Option<u64>,
) -> Result<()> {
    match source_config {
        SourceConfig::Jira(jira_config) => {
            let connector = JiraConnector::new(jira_config);
            sync_with_connector(
                azure,
                embed_client,
                &connector,
                config,
                state,
                state_path,
                max_docs,
            )
            .await
        }
        SourceConfig::Confluence(conf_config) => {
            let connector = ConfluenceConnector::new(conf_config);
            sync_with_connector(
                azure,
                embed_client,
                &connector,
                config,
                state,
                state_path,
                max_docs,
            )
            .await
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
    max_docs: Option<u64>,
) -> Result<()> {
    let index_name = connector.index_name();
    let source_name = connector.source_name();

    // Get cursor from persisted state
    let source_state = state.get_source(source_name);
    let mut cursor = source_state
        .last_cursor
        .map(|ts| SyncCursor { last_updated: ts });

    if let Some(ref c) = cursor {
        info!(
            source = source_name,
            last_cursor = %c.last_updated,
            "Resuming sync from saved cursor"
        );
    } else {
        info!(
            source = source_name,
            "Starting full sync (no previous cursor)"
        );
    }

    let mut total_synced: u64 = 0;
    let mut batch_num: u64 = 0;
    let mut soft_limit_reached = false;

    loop {
        // --max-docs is a soft limit: once we've synced ≥ N docs, we stop
        // fetching new batches but we never truncate mid-batch so we always
        // finish the current minute and avoid leaving a half-synced gap.
        if soft_limit_reached {
            info!(
                source = source_name,
                total = total_synced,
                limit = max_docs.unwrap_or(0),
                "Reached --max-docs soft limit, stopping sync"
            );
            break;
        }

        batch_num += 1;
        let result = connector
            .fetch_changes(cursor.as_ref(), config.sync.batch_size)
            .await
            .context("failed to fetch changes from source")?;

        // Destructure result before filtering (documents is consumed by into_iter)
        let result_cursor = result.cursor;
        let result_has_more = result.has_more;

        // Filter out documents from before the cursor's minute. JQL uses minute
        // precision ("updated >= 2026-01-12 13:56"), so the filter must also use
        // minute precision. We truncate the cursor to its minute start and keep
        // all docs after that. This re-syncs a few docs from the cursor's minute,
        // which is fine since Azure AI Search push is an upsert.
        let new_docs: Vec<_> = if let Some(ref c) = cursor {
            let cursor_minute = c
                .last_updated
                .with_second(0)
                .and_then(|t| t.with_nanosecond(0))
                .unwrap_or(c.last_updated);
            result
                .documents
                .into_iter()
                .filter(|doc| doc.updated_at > cursor_minute)
                .collect()
        } else {
            result.documents
        };

        let doc_count = new_docs.len() as u64;
        if doc_count == 0 {
            if batch_num == 1 {
                info!(source = source_name, "No changes since last sync");
            } else {
                info!(
                    source = source_name,
                    batches = batch_num,
                    total = total_synced,
                    "No more changes to sync"
                );
            }
            break;
        }

        // Log each document at info level for visibility
        for doc in &new_docs {
            let issue_key = doc
                .fields
                .get("issue_key")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let summary = doc
                .fields
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let updated = doc
                .fields
                .get("updated_at")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let summary_preview = truncate_for_display(summary, 80);
            info!(
                source = source_name,
                key = issue_key,
                updated = updated,
                "[{}/{}] {} — {}",
                total_synced + 1,
                max_docs.map_or("∞".to_string(), |l| l.to_string()),
                issue_key,
                summary_preview
            );
        }

        // Log document content at debug level (-v)
        for doc in &new_docs {
            let id = doc.fields.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            let content = doc
                .fields
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let preview = truncate_for_display(content, 200);
            debug!(
                source = source_name,
                id = id,
                content_len = content.len(),
                "Content: {}",
                preview
            );
        }

        // Generate embeddings if client is available.
        // If the content exceeds the model's token limit, we progressively truncate
        // and retry per-document rather than failing the whole batch.
        let embeddings: Option<Vec<Vec<f32>>> = if let Some(client) = embed_client {
            debug!(
                source = source_name,
                count = new_docs.len(),
                "Generating embeddings"
            );
            let mut vecs = Vec::with_capacity(new_docs.len());
            for doc in &new_docs {
                let content = doc
                    .fields
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let id = doc.fields.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let embedding = embed_with_retry(client, id, content, source_name)
                    .await
                    .context("failed to generate embedding")?;
                vecs.push(embedding);
            }
            Some(vecs)
        } else {
            None
        };

        // Convert SourceDocuments to JSON values, with embeddings if available
        let azure_docs: Vec<serde_json::Value> = new_docs
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
        state.update_source(source_name, result_cursor.last_updated, doc_count);
        state
            .save(state_path)
            .context("failed to save sync state")?;

        info!(
            source = source_name,
            batch = batch_num,
            batch_docs = doc_count,
            total = total_synced,
            cursor = %result_cursor.last_updated,
            "Batch pushed to Azure AI Search"
        );

        // Advance cursor for next iteration — THIS IS CRITICAL.
        // Without this, the next fetch_changes call would use the same JQL
        // timestamp and return the same results, causing an infinite loop.
        cursor = Some(result_cursor);

        // Check soft limit after processing and cursor update
        if let Some(limit) = max_docs
            && total_synced >= limit
        {
            soft_limit_reached = true;
        }

        if !result_has_more {
            break;
        }
    }

    if total_synced > 0 {
        info!(source = source_name, total = total_synced, "Sync complete");
    }

    Ok(())
}

/// Embed a single document's content, retrying with progressively truncated text
/// if the embedding model rejects it for exceeding the token limit.
///
/// Strategy: on a token-limit error, calculate the reduction ratio from the error
/// message if possible, otherwise halve the content. Retry up to 5 times.
async fn embed_with_retry(
    client: &ailloy::Client,
    doc_id: &str,
    content: &str,
    source_name: &str,
) -> Result<Vec<f32>> {
    const MAX_RETRIES: usize = 5;

    let mut text = content.to_string();

    for attempt in 0..=MAX_RETRIES {
        match client.embed_one(&text).await {
            Ok(embedding) => return Ok(embedding),
            Err(e) => {
                if attempt == MAX_RETRIES {
                    anyhow::bail!(
                        "document {} still exceeds token limit after {} truncations: {}",
                        doc_id,
                        MAX_RETRIES,
                        e
                    );
                }

                // Check if this is a token limit error we can retry.
                // Match on the error string since the ClientError may be wrapped
                // in context layers that prevent downcast_ref from finding it.
                let error_msg = format!("{}", e);
                let is_token_error = error_msg.contains("maximum context length");

                if !is_token_error {
                    return Err(e);
                }

                // Try to parse the actual/max tokens from the error to calculate
                // the optimal reduction. Error format:
                // "...maximum context length is 8192 tokens, however you requested 11371 tokens..."
                let shrink_ratio = parse_token_ratio(&error_msg).unwrap_or(0.5);

                let new_len = ((text.len() as f64) * shrink_ratio * 0.9) as usize; // 10% extra margin
                let new_len = new_len.max(100); // never go below 100 chars

                // Truncate on a char boundary
                let byte_end = text
                    .char_indices()
                    .take_while(|(i, _)| *i < new_len)
                    .last()
                    .map(|(i, c)| i + c.len_utf8())
                    .unwrap_or(new_len.min(text.len()));
                text.truncate(byte_end);

                warn!(
                    source = source_name,
                    id = doc_id,
                    attempt = attempt + 1,
                    new_chars = text.len(),
                    shrink_ratio = format!("{:.0}%", shrink_ratio * 100.0),
                    "Truncating content for embedding (token limit exceeded)"
                );
            }
        }
    }

    unreachable!()
}

/// Parse the max/requested token counts from an embedding error message and return
/// the ratio (max_tokens / requested_tokens) to scale the content down.
fn parse_token_ratio(error_msg: &str) -> Option<f64> {
    // "maximum context length is 8192 tokens, however you requested 11371 tokens"
    let max_pos = error_msg.find("maximum context length is ")?;
    let after_max = &error_msg[max_pos + 25..];
    let max_tokens: f64 = after_max
        .split_whitespace()
        .next()?
        .trim_end_matches(|c: char| !c.is_ascii_digit())
        .parse()
        .ok()?;

    let req_pos = error_msg.find("you requested ")?;
    let after_req = &error_msg[req_pos + 14..];
    let req_tokens: f64 = after_req
        .split_whitespace()
        .next()?
        .trim_end_matches(|c: char| !c.is_ascii_digit())
        .parse()
        .ok()?;

    if req_tokens > 0.0 {
        Some(max_tokens / req_tokens)
    } else {
        None
    }
}
