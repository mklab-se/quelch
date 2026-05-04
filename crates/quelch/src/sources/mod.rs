pub mod confluence;
pub mod jira;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A document fetched from a source, ready for transformation and Cosmos upsert.
#[derive(Debug, Clone)]
pub struct SourceDocument {
    /// Composite document ID, e.g. `"jira-prod-DO-1234"`.
    pub id: String,
    /// Cosmos DB partition key for this document.
    pub partition_key: String,
    /// Arbitrary document body fields.
    pub fields: HashMap<String, serde_json::Value>,
    /// Canonical ordering field — when the source last modified this document.
    pub updated_at: DateTime<Utc>,
    /// Mandatory deep-link back to the source entity.
    pub source_link: String,
}

/// One page of fetched documents, plus paging metadata.
pub struct FetchPage {
    /// Documents returned by this page.
    pub documents: Vec<SourceDocument>,
    /// Token to pass to the next `fetch_window` call; `None` means this is the last page.
    pub next_page_token: Option<String>,
    /// Backfill resume marker; populated only by `fetch_backfill_page`.
    pub last_seen: Option<BackfillCheckpoint>,
}

/// Resume marker for incremental backfill.
///
/// See `docs/sync.md` — "Initial backfill" section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackfillCheckpoint {
    /// `updated_at` of the last document seen in the previous page.
    pub updated: DateTime<Utc>,
    /// Document key (ID) of the last document seen — used as a tiebreaker.
    pub key: String,
}

/// Companion documents fetched alongside primary entities.
///
/// These feed separate Cosmos containers (sprints, fix_versions, etc.).
#[derive(Default)]
pub struct Companions {
    /// Jira sprint documents.
    pub sprints: Vec<SourceDocument>,
    /// Jira fix-version documents.
    pub fix_versions: Vec<SourceDocument>,
    /// Jira project documents.
    pub projects: Vec<SourceDocument>,
    /// Confluence space documents.
    pub spaces: Vec<SourceDocument>,
}

/// Trait implemented by each source connector (Jira, Confluence, …).
///
/// All methods are `async` and `Send`-safe via `trait_variant::make`.
/// The trait surface is split into four operations:
///
/// - `fetch_window` — closed minute-resolution incremental window
/// - `fetch_backfill_page` — cursor-paged historical backfill
/// - `list_all_ids` — reconciliation / deletion detection
/// - `fetch_companions` — optional companion-container documents
#[trait_variant::make(Send)]
pub trait SourceConnector: Sync {
    /// Human-readable source type, e.g. `"jira"` or `"confluence"`.
    fn source_type(&self) -> &str;

    /// The source name from config — used as a stable identifier in state / Cosmos.
    fn source_name(&self) -> &str;

    /// Subsource identifiers — Jira project keys or Confluence space keys.
    fn subsources(&self) -> &[String];

    /// Target Cosmos container for this source's primary entity.
    fn primary_container(&self) -> &str;

    /// Fetch a closed minute-resolution window of documents updated in
    /// `[window_start, window_end)`.
    ///
    /// `page_token` is `None` on the first call; subsequent calls pass
    /// `FetchPage::next_page_token` from the previous response.
    async fn fetch_window(
        &self,
        subsource: &str,
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
        batch_size: usize,
        page_token: Option<&str>,
    ) -> anyhow::Result<FetchPage>;

    /// Fetch one page of backfill, resuming after `last_seen`.
    ///
    /// `last_seen = None` starts from the beginning (oldest first).
    /// `backfill_target` is the upper bound — documents strictly older than
    /// this timestamp are eligible.
    async fn fetch_backfill_page(
        &self,
        subsource: &str,
        backfill_target: DateTime<Utc>,
        last_seen: Option<&BackfillCheckpoint>,
        batch_size: usize,
    ) -> anyhow::Result<FetchPage>;

    /// List all document IDs currently visible in the source for a subsource.
    ///
    /// Used for deletion reconciliation: any ID in Cosmos but absent from this
    /// list should be tombstoned.
    async fn list_all_ids(&self, subsource: &str) -> anyhow::Result<Vec<String>>;

    /// Fetch companion documents (sprints, fix versions, spaces, …).
    ///
    /// Default implementation returns an empty [`Companions`]; sources that
    /// have companion containers override this.
    async fn fetch_companions(&self, _subsource: &str) -> anyhow::Result<Companions>;
}
