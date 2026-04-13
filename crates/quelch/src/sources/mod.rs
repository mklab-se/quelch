pub mod jira;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Cursor for tracking incremental sync position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncCursor {
    /// Timestamp of the last synced document's update time.
    pub last_updated: DateTime<Utc>,
}

/// A document fetched from a source, ready for transformation and indexing.
#[derive(Debug, Clone)]
pub struct SourceDocument {
    /// Unique document ID (e.g., "test-jira-DO-1234").
    pub id: String,
    /// The content fields to index.
    pub fields: HashMap<String, serde_json::Value>,
    /// Timestamp of last modification in source.
    pub updated_at: DateTime<Utc>,
}

/// Result of a fetch operation — documents plus a new cursor position.
pub struct FetchResult {
    pub documents: Vec<SourceDocument>,
    pub cursor: SyncCursor,
    /// True if there are more pages to fetch.
    pub has_more: bool,
}

/// Trait implemented by each source connector (Jira, Confluence, etc.).
#[trait_variant::make(Send)]
pub trait SourceConnector: Sync {
    /// Human-readable source type name.
    fn source_type(&self) -> &str;

    /// The source name from config (used as identifier in state).
    fn source_name(&self) -> &str;

    /// The target Azure AI Search index name.
    fn index_name(&self) -> &str;

    /// Fetch documents changed since the given cursor.
    /// If cursor is None, fetch everything (initial full sync).
    async fn fetch_changes(
        &self,
        cursor: Option<&SyncCursor>,
        batch_size: usize,
    ) -> anyhow::Result<FetchResult>;

    /// Fetch IDs of all documents currently in the source.
    /// Used for detecting deletions.
    async fn fetch_all_ids(&self) -> anyhow::Result<Vec<String>>;
}
