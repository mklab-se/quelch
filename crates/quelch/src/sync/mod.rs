pub mod embedder;
pub mod state;

use anyhow::{Context, Result};
use chrono::Timelike;
use std::io::Write;
use std::path::Path;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::azure::SearchClient;
use crate::azure::schema::{EmbeddingConfig, confluence_index_schema, jira_index_schema};
use crate::config::{Config, SourceConfig};
use crate::sources::confluence::ConfluenceConnector;
use crate::sources::jira::JiraConnector;
use crate::sources::{SourceConnector, SyncCursor};

use self::state::SyncState;

/// Commands the TUI sends back to the engine. Plain-log runs get a
/// never-firing receiver so the same code path serves both modes.
#[derive(Debug, Clone)]
pub enum UiCommand {
    Pause,
    Resume,
    SyncNow,
    ResetCursor {
        source: String,
        subsource: Option<String>,
    },
    PurgeNow {
        source: String,
    },
    Shutdown,
}

/// Build a never-firing command channel for plain-log runs where no TUI
/// layer will push commands. The sender is dropped immediately by the
/// caller (or held but unused); the receiver is consumed by the engine.
pub fn never_command_channel() -> (mpsc::Sender<UiCommand>, mpsc::Receiver<UiCommand>) {
    mpsc::channel(1)
}

/// Outcome of a command-poll tick or one iteration of the engine loop.
#[derive(Debug)]
pub enum EngineOutcome {
    Continue,
    Shutdown,
    ResetCursor {
        source: String,
        subsource: Option<String>,
    },
}

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

/// Build the `(source_name, subsources)` mapping for all configured sources.
/// Used to hydrate state-file migration and for loading per-subsource state.
fn subsources_by_source(config: &Config) -> Vec<(String, Vec<String>)> {
    config
        .sources
        .iter()
        .map(|s| match s {
            SourceConfig::Jira(j) => (j.name.clone(), j.projects.clone()),
            SourceConfig::Confluence(c) => (c.name.clone(), c.spaces.clone()),
        })
        .collect()
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
    let mut state = SyncState::load(state_path, &subsources_by_source(config))?;
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
/// If `embedder` is None, documents are pushed without embeddings (for testing/mock mode).
/// If `max_docs` is Some, stop after syncing that many documents per source.
pub async fn run_sync(
    config: &Config,
    state_path: &Path,
    embedding: &EmbeddingConfig,
    index_mode: IndexMode,
    embedder: Option<&dyn embedder::Embedder>,
    max_docs: Option<u64>,
) -> Result<()> {
    let (_tx, mut rx) = never_command_channel();
    run_sync_with(
        config, state_path, embedding, index_mode, embedder, max_docs, &mut rx,
    )
    .await
}

/// Run a sync cycle while observing `cmd_rx` for TUI commands. The engine
/// polls at source/subsource/batch boundaries and reacts to
/// `Pause`/`Resume`/`Shutdown`/`ResetCursor` appropriately.
#[allow(clippy::too_many_arguments)]
pub async fn run_sync_with(
    config: &Config,
    state_path: &Path,
    embedding: &EmbeddingConfig,
    index_mode: IndexMode,
    embedder: Option<&dyn embedder::Embedder>,
    max_docs: Option<u64>,
    cmd_rx: &mut mpsc::Receiver<UiCommand>,
) -> Result<()> {
    setup_indexes(config, embedding, index_mode).await?;
    let azure = SearchClient::new(&config.azure.endpoint, &config.azure.api_key);
    let subs = subsources_by_source(config);
    let mut state = SyncState::load(state_path, &subs)?;
    let mut paused = false;
    let mut failures = Vec::new();

    info!(phase = "cycle_started", "Cycle starting");

    for source_config in &config.sources {
        if let EngineOutcome::Shutdown = poll_commands(cmd_rx, &mut paused).await {
            tracing::info!(phase = "cycle_finished", "Cycle shutdown");
            return Ok(());
        }
        if let Err(e) = sync_source(
            &azure,
            embedder,
            source_config,
            config,
            &mut state,
            state_path,
            max_docs,
            cmd_rx,
            &mut paused,
        )
        .await
        {
            let error_chain = format_error_chain(&e);
            error!(
                phase = "source_failed",
                source = source_config.name(),
                error = %error_chain,
                "Sync failed for source"
            );
            failures.push(format!("{}: {}", source_config.name(), error_chain));
        }
    }

    info!(phase = "cycle_finished", "Cycle finished");

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

    // Fetch all IDs from the source across all subsources
    let mut source_ids = std::collections::HashSet::new();
    for subsource in connector.subsources() {
        let ids = connector
            .fetch_all_ids(subsource)
            .await
            .context("failed to fetch IDs from source")?;
        for id in ids {
            source_ids.insert(id);
        }
    }

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

#[allow(clippy::too_many_arguments)]
async fn sync_source(
    azure: &SearchClient,
    embedder: Option<&dyn embedder::Embedder>,
    source_config: &SourceConfig,
    config: &Config,
    state: &mut SyncState,
    state_path: &Path,
    max_docs: Option<u64>,
    cmd_rx: &mut mpsc::Receiver<UiCommand>,
    paused: &mut bool,
) -> Result<()> {
    match source_config {
        SourceConfig::Jira(jira_config) => {
            let connector = JiraConnector::new(jira_config);
            sync_with_connector(
                azure, embedder, &connector, config, state, state_path, max_docs, cmd_rx, paused,
            )
            .await
            .map(|_| ())
        }
        SourceConfig::Confluence(conf_config) => {
            let connector = ConfluenceConnector::new(conf_config);
            sync_with_connector(
                azure, embedder, &connector, config, state, state_path, max_docs, cmd_rx, paused,
            )
            .await
            .map(|_| ())
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn sync_with_connector<C: SourceConnector>(
    azure: &SearchClient,
    embedder: Option<&dyn embedder::Embedder>,
    connector: &C,
    config: &Config,
    state: &mut SyncState,
    state_path: &Path,
    max_docs: Option<u64>,
    cmd_rx: &mut mpsc::Receiver<UiCommand>,
    paused: &mut bool,
) -> Result<EngineOutcome> {
    let source_name = connector.source_name();
    info!(source = source_name, "Starting source");

    for subsource_key in connector.subsources() {
        // Command poll at subsource boundary
        match poll_commands(cmd_rx, paused).await {
            EngineOutcome::Shutdown => return Ok(EngineOutcome::Shutdown),
            EngineOutcome::ResetCursor {
                source: s,
                subsource,
            } if s == source_name => {
                state.reset_source(source_name, subsource.as_deref());
                state
                    .save(state_path)
                    .context("failed to save sync state")?;
                continue;
            }
            _ => {}
        }

        sync_single_subsource(
            azure,
            embedder,
            connector,
            subsource_key,
            config,
            state,
            state_path,
            max_docs,
            cmd_rx,
            paused,
        )
        .await?;
    }

    info!(source = source_name, "Finished source");
    Ok(EngineOutcome::Continue)
}

/// Non-blocking drain of command channel. Applies `Pause`/`Resume` in place
/// (updates `paused`) and returns the first actionable outcome for the caller.
async fn poll_commands(cmd_rx: &mut mpsc::Receiver<UiCommand>, paused: &mut bool) -> EngineOutcome {
    loop {
        match cmd_rx.try_recv() {
            Ok(UiCommand::Pause) => {
                *paused = true;
            }
            Ok(UiCommand::Resume) => {
                *paused = false;
            }
            Ok(UiCommand::Shutdown) => return EngineOutcome::Shutdown,
            Ok(UiCommand::ResetCursor { source, subsource }) => {
                return EngineOutcome::ResetCursor { source, subsource };
            }
            Ok(UiCommand::SyncNow) | Ok(UiCommand::PurgeNow { .. }) => {
                // SyncNow is only meaningful during the watch sleep.
                // PurgeNow is handled by the caller in run_sync_with.
            }
            Err(_) => break,
        }
    }
    // Block while paused — but still handle Resume/Shutdown.
    while *paused {
        match cmd_rx.recv().await {
            Some(UiCommand::Resume) => {
                *paused = false;
                break;
            }
            Some(UiCommand::Shutdown) => return EngineOutcome::Shutdown,
            Some(UiCommand::Pause) => { /* already paused */ }
            Some(UiCommand::ResetCursor { source, subsource }) => {
                return EngineOutcome::ResetCursor { source, subsource };
            }
            Some(_) => { /* ignore while paused */ }
            None => {
                *paused = false;
                break;
            }
        }
    }
    EngineOutcome::Continue
}

#[allow(clippy::too_many_arguments)]
async fn sync_single_subsource<C: SourceConnector>(
    azure: &SearchClient,
    embedder: Option<&dyn embedder::Embedder>,
    connector: &C,
    subsource: &str,
    config: &Config,
    state: &mut SyncState,
    state_path: &Path,
    max_docs: Option<u64>,
    cmd_rx: &mut mpsc::Receiver<UiCommand>,
    paused: &mut bool,
) -> Result<()> {
    let source_name = connector.source_name();
    let index_name = connector.index_name();

    let src_state = state.get_source(source_name);
    let mut cursor = src_state
        .subsources
        .get(subsource)
        .and_then(|s| s.last_cursor)
        .map(|ts| SyncCursor { last_updated: ts });

    info!(
        phase = "subsource_started",
        source = source_name,
        subsource = subsource,
        "Starting subsource"
    );

    let mut total_synced: u64 = 0;
    let mut batch_num: u64 = 0;
    let mut soft_limit_reached = false;

    loop {
        if soft_limit_reached {
            break;
        }

        // Command poll at batch boundary
        match poll_commands(cmd_rx, paused).await {
            EngineOutcome::Shutdown => {
                tracing::info!(
                    phase = "subsource_finished",
                    source = source_name,
                    subsource = subsource,
                    "Shutdown mid-subsource"
                );
                return Ok(());
            }
            EngineOutcome::ResetCursor {
                source: s,
                subsource: Some(sub),
            } if s == source_name && sub == subsource => {
                state.reset_source(source_name, Some(subsource));
                if let Err(e) = state.save(state_path) {
                    tracing::warn!(
                        source = source_name,
                        subsource = subsource,
                        error = %e,
                        "failed to persist reset"
                    );
                }
                cursor = None;
            }
            _ => {}
        }

        batch_num += 1;
        let result = connector
            .fetch_changes(subsource, cursor.as_ref(), config.sync.batch_size)
            .await
            .context("failed to fetch changes from source")?;

        let result_cursor = result.cursor;
        let result_has_more = result.has_more;

        // Filter out documents from before the cursor's minute. JQL uses minute
        // precision ("updated >= 2026-01-12 13:56"), so the filter must also use
        // minute precision. We truncate the cursor to its minute start and keep
        // all docs after that.
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
            info!(
                phase = "subsource_empty",
                source = source_name,
                subsource = subsource,
                batches = batch_num,
                total = total_synced,
                "No changes to sync"
            );
            break;
        }

        // Emit per-doc tracing events — tracing layer will surface as DocSynced.
        for doc in &new_docs {
            let id = doc.fields.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            let updated = doc
                .fields
                .get("updated_at")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            info!(
                phase = "doc_synced",
                source = source_name,
                subsource = subsource,
                doc_id = id,
                updated = updated,
                "doc"
            );
        }

        // Generate embeddings if an embedder is available.
        let embeddings: Option<Vec<Vec<f32>>> = if let Some(emb) = embedder {
            let mut vecs = Vec::with_capacity(new_docs.len());
            for doc in &new_docs {
                let content = doc
                    .fields
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let id = doc.fields.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let embedding = embed_with_retry(emb, id, content, source_name)
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

        let sample_id = new_docs
            .last()
            .and_then(|d| d.fields.get("id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        state.update_subsource(
            source_name,
            subsource,
            result_cursor.last_updated,
            doc_count,
            sample_id.clone(),
        );
        state
            .save(state_path)
            .context("failed to save sync state")?;

        info!(
            phase = "subsource_batch",
            source = source_name,
            subsource = subsource,
            batch = batch_num,
            fetched = doc_count,
            cursor = %result_cursor.last_updated,
            sample_id = sample_id.as_deref().unwrap_or(""),
            "Batch pushed"
        );

        cursor = Some(result_cursor);

        if let Some(limit) = max_docs
            && total_synced >= limit
        {
            soft_limit_reached = true;
        }
        if !result_has_more {
            break;
        }
    }

    info!(
        phase = "subsource_finished",
        source = source_name,
        subsource = subsource,
        total = total_synced,
        "Subsource complete"
    );

    Ok(())
}

/// Embed a single document's content, retrying with progressively truncated text
/// if the embedding model rejects it for exceeding the token limit.
///
/// Strategy: on a token-limit error, calculate the reduction ratio from the error
/// message if possible, otherwise halve the content. Retry up to 5 times.
async fn embed_with_retry(
    embedder: &dyn embedder::Embedder,
    doc_id: &str,
    content: &str,
    source_name: &str,
) -> Result<Vec<f32>> {
    const MAX_RETRIES: usize = 5;

    let mut text = content.to_string();

    for attempt in 0..=MAX_RETRIES {
        match embedder.embed_one(&text).await {
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
