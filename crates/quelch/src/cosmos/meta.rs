//! Cursor state CRUD for the `quelch-meta` Cosmos container.
//!
//! Each row is keyed by `(source_name, subsource)` and tracks incremental
//! sync progress for one logical data stream. The `owner_instance` field
//! records which Q-Ingest instance currently writes this cursor — used to
//! enforce the rule that two ingest instances must not overlap on the same
//! `(source_name, subsource)` tuple.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::cosmos::{CosmosBackend, CosmosError};

// ---------------------------------------------------------------------------
// Key
// ---------------------------------------------------------------------------

/// Identifies a cursor row: one `(source, subsource)` pair.
#[derive(Debug, Clone)]
pub struct CursorKey {
    /// Source connection name as defined in `quelch.yaml` (e.g. `"my-jira"`).
    pub source_name: String,
    /// Subsource identifier (e.g. a Jira project key like `"DO"` or a
    /// Confluence space key like `"ENG"`).
    pub subsource: String,
}

impl CursorKey {
    /// Stable, human-readable Cosmos document `id`.
    pub fn id(&self) -> String {
        format!("{}::{}", self.source_name, self.subsource)
    }

    /// Partition key — equal to `source_name` so all cursors for a given
    /// source connection land in the same physical partition.
    pub fn partition_key(&self) -> &str {
        &self.source_name
    }
}

// ---------------------------------------------------------------------------
// Cursor document
// ---------------------------------------------------------------------------

/// Per-`(source, subsource)` sync cursor stored in Cosmos.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cursor {
    /// Name of the Q-Ingest instance that currently owns this cursor.
    ///
    /// `None` means unowned (fresh row). The first ingest cycle to encounter
    /// it claims it by writing its own instance name. A later cycle from a
    /// different instance must refuse to write this cursor unless the
    /// operator has run `quelch reset --take-ownership` to transfer it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_instance: Option<String>,

    /// The latest *complete* minute whose changed items have been ingested.
    /// Incremental sync resumes from this point on the next run.
    pub last_complete_minute: Option<DateTime<Utc>>,

    /// Running total of documents ingested for this subsource.
    #[serde(default)]
    pub documents_synced_total: u64,

    /// Wall-clock time of the last successful sync iteration.
    pub last_sync_at: Option<DateTime<Utc>>,

    /// Human-readable message from the last error, if any.
    pub last_error: Option<String>,

    /// `true` while a backfill (historical) pass is in progress.
    #[serde(default)]
    pub backfill_in_progress: bool,

    /// The earliest timestamp the backfill is trying to reach.
    pub backfill_target: Option<DateTime<Utc>>,

    /// Last seen position in the backfill crawl (for crash recovery).
    pub backfill_last_seen: Option<BackfillCheckpoint>,

    /// Wall-clock time of the last soft-delete reconciliation pass.
    pub last_reconciliation_at: Option<DateTime<Utc>>,

    /// Number of documents marked soft-deleted in the last reconciliation pass.
    #[serde(default)]
    pub last_reconciliation_deleted: u64,
}

/// Position in a backfill crawl, used to resume after an interruption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackfillCheckpoint {
    /// The `updated` timestamp of the last item processed.
    pub updated: DateTime<Utc>,
    /// The source-specific key of the last item processed (e.g. a Jira issue key).
    pub key: String,
}

// ---------------------------------------------------------------------------
// CRUD
// ---------------------------------------------------------------------------

/// Load a cursor from Cosmos.
///
/// Returns `Cursor::default()` (all fields `None` / `0` / `false`) if the row
/// does not exist yet.
pub async fn load(
    backend: &dyn CosmosBackend,
    meta_container: &str,
    key: &CursorKey,
) -> Result<Cursor, CosmosError> {
    match backend
        .get(meta_container, &key.id(), key.partition_key())
        .await?
    {
        Some(value) => {
            let cursor: Cursor = serde_json::from_value(value)?;
            Ok(cursor)
        }
        None => Ok(Cursor::default()),
    }
}

/// Try to load a cursor from Cosmos, returning `None` if the row doesn't exist.
///
/// Unlike [`load`], this distinguishes between "cursor row absent" and
/// "cursor row present but freshly defaulted" — useful for the ownership
/// claim path which needs to know whether a cursor has ever been written.
pub async fn try_load(
    backend: &dyn CosmosBackend,
    meta_container: &str,
    key: &CursorKey,
) -> Result<Option<Cursor>, CosmosError> {
    match backend
        .get(meta_container, &key.id(), key.partition_key())
        .await?
    {
        Some(value) => {
            let cursor: Cursor = serde_json::from_value(value)?;
            Ok(Some(cursor))
        }
        None => Ok(None),
    }
}

/// Persist a cursor to Cosmos (upsert semantics).
///
/// In addition to the `Cursor` fields the stored document carries `id`,
/// `source_name`, `subsource`, and `_partition_key` so that it can be
/// queried / point-read without any auxiliary index.
pub async fn save(
    backend: &dyn CosmosBackend,
    meta_container: &str,
    key: &CursorKey,
    cursor: &Cursor,
) -> Result<(), CosmosError> {
    let mut doc = serde_json::to_value(cursor)?;
    let obj = doc
        .as_object_mut()
        .ok_or_else(|| CosmosError::Validation("cursor serialised to non-object".into()))?;

    obj.insert("id".into(), key.id().into());
    obj.insert("source_name".into(), key.source_name.clone().into());
    obj.insert("subsource".into(), key.subsource.clone().into());
    obj.insert("_partition_key".into(), key.source_name.clone().into());

    backend.upsert(meta_container, doc).await
}

/// List every cursor stored in the given meta container.
///
/// Used by `quelch status` to enumerate all known sync streams. Performs a
/// full container scan via `SELECT * FROM c`.
pub async fn list_all(
    backend: &dyn CosmosBackend,
    meta_container: &str,
) -> Result<Vec<(CursorKey, Cursor)>, CosmosError> {
    let mut stream = backend
        .query(meta_container, "SELECT * FROM c", vec![])
        .await?;

    let mut results = Vec::new();

    while let Some(page) = stream.next_page().await? {
        for value in page {
            let source_name = value
                .get("source_name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    CosmosError::Validation("cursor doc missing string `source_name`".into())
                })?
                .to_string();
            let subsource = value
                .get("subsource")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    CosmosError::Validation("cursor doc missing string `subsource`".into())
                })?
                .to_string();

            let cursor: Cursor = serde_json::from_value(value)?;
            let cursor_key = CursorKey {
                source_name,
                subsource,
            };
            results.push((cursor_key, cursor));
        }
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosmos::InMemoryCosmos;
    use chrono::Utc;

    const META: &str = "quelch-meta";

    fn key(source: &str, subsource: &str) -> CursorKey {
        CursorKey {
            source_name: source.to_string(),
            subsource: subsource.to_string(),
        }
    }

    #[tokio::test]
    async fn save_and_load_cursor_round_trip() {
        let backend = InMemoryCosmos::new();
        let k = key("my-jira", "DO");

        let cursor = Cursor {
            documents_synced_total: 42,
            last_complete_minute: Some(Utc::now()),
            backfill_in_progress: true,
            backfill_last_seen: Some(BackfillCheckpoint {
                updated: Utc::now(),
                key: "DO-99".into(),
            }),
            ..Default::default()
        };

        save(&backend, META, &k, &cursor).await.unwrap();
        let loaded = load(&backend, META, &k).await.unwrap();

        assert_eq!(loaded.documents_synced_total, 42);
        assert!(loaded.last_complete_minute.is_some());
        assert!(loaded.backfill_in_progress);
        assert_eq!(loaded.backfill_last_seen.as_ref().unwrap().key, "DO-99");
    }

    #[tokio::test]
    async fn load_returns_default_when_missing() {
        let backend = InMemoryCosmos::new();
        let k = key("confluence", "DOCS");

        let cursor = load(&backend, META, &k).await.unwrap();

        assert_eq!(cursor.documents_synced_total, 0);
        assert!(cursor.last_complete_minute.is_none());
        assert!(!cursor.backfill_in_progress);
        assert!(cursor.owner_instance.is_none());
    }

    #[tokio::test]
    async fn save_overwrites_previous_value() {
        let backend = InMemoryCosmos::new();
        let k = key("my-jira", "DO");

        let c1 = Cursor {
            documents_synced_total: 10,
            ..Default::default()
        };
        save(&backend, META, &k, &c1).await.unwrap();

        let c2 = Cursor {
            documents_synced_total: 99,
            ..Default::default()
        };
        save(&backend, META, &k, &c2).await.unwrap();

        let loaded = load(&backend, META, &k).await.unwrap();
        assert_eq!(loaded.documents_synced_total, 99);
    }

    #[tokio::test]
    async fn cursor_key_id_and_partition_key() {
        let k = key("my-jira", "DO");
        assert_eq!(k.id(), "my-jira::DO");
        assert_eq!(k.partition_key(), "my-jira");
    }

    #[tokio::test]
    async fn list_all_returns_all_cursors() {
        let backend = InMemoryCosmos::new();

        let k1 = key("my-jira", "DO");
        let k2 = key("confluence", "WIKI");
        let k3 = key("my-jira", "HR");

        let c1 = Cursor {
            documents_synced_total: 1,
            ..Default::default()
        };
        let c2 = Cursor {
            documents_synced_total: 2,
            ..Default::default()
        };
        let c3 = Cursor {
            documents_synced_total: 3,
            ..Default::default()
        };

        save(&backend, META, &k1, &c1).await.unwrap();
        save(&backend, META, &k2, &c2).await.unwrap();
        save(&backend, META, &k3, &c3).await.unwrap();

        let all = list_all(&backend, META).await.unwrap();
        assert_eq!(all.len(), 3);

        // Verify each key is present with correct total
        let totals: std::collections::HashMap<String, u64> = all
            .into_iter()
            .map(|(k, c)| (k.id(), c.documents_synced_total))
            .collect();

        assert_eq!(totals["my-jira::DO"], 1);
        assert_eq!(totals["confluence::WIKI"], 2);
        assert_eq!(totals["my-jira::HR"], 3);
    }

    #[tokio::test]
    async fn list_all_empty_returns_empty_vec() {
        let backend = InMemoryCosmos::new();
        let all = list_all(&backend, META).await.unwrap();
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn save_stores_required_envelope_fields() {
        // Verify the raw stored document has id, source_name, subsource,
        // and _partition_key so queries work.
        let backend = InMemoryCosmos::new();
        let k = key("my-jira", "DO");
        save(&backend, META, &k, &Cursor::default()).await.unwrap();

        let raw = backend
            .get(META, &k.id(), k.partition_key())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(raw["id"], "my-jira::DO");
        assert_eq!(raw["source_name"], "my-jira");
        assert_eq!(raw["subsource"], "DO");
        assert_eq!(raw["_partition_key"], "my-jira");
    }

    #[test]
    fn cursor_serializes_with_owner_instance() {
        let c = Cursor {
            owner_instance: Some("ingest-internal".into()),
            documents_synced_total: 42,
            ..Default::default()
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"owner_instance\":\"ingest-internal\""));
        let back: Cursor = serde_json::from_str(&json).unwrap();
        assert_eq!(back.owner_instance.as_deref(), Some("ingest-internal"));
        assert_eq!(back.documents_synced_total, 42);
    }

    #[test]
    fn cursor_omits_owner_instance_when_none() {
        let c = Cursor::default();
        let json = serde_json::to_string(&c).unwrap();
        assert!(!json.contains("owner_instance"));
    }
}
