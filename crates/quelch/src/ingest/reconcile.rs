//! Deletion reconciliation.
//!
//! Compares the set of IDs in the source against the set in Cosmos, then
//! soft-deletes (`_deleted = true`) any document that exists in Cosmos but
//! not in the source.
//!
//! See `docs/sync.md` — "Deletions".

use std::collections::HashSet;

use chrono::Utc;
use serde_json::Value;

use crate::{
    cosmos::meta::CursorKey,
    cosmos::{CosmosBackend, meta},
    ingest::config::CycleConfig,
    sources::SourceConnector,
};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run one reconciliation pass for `(key.source_name, key.subsource)`.
///
/// Returns the number of documents that were newly soft-deleted, or an error
/// string on failure.
pub async fn run<C>(
    connector: &C,
    cosmos: &dyn CosmosBackend,
    key: &CursorKey,
    cfg: &CycleConfig,
) -> Result<u64, String>
where
    C: SourceConnector,
{
    // Collect all IDs currently visible in the source.
    let source_ids: HashSet<String> = match connector.list_all_ids(&key.subsource).await {
        Ok(ids) => ids.into_iter().collect(),
        Err(e) => return Err(format!("list_all_ids: {e}")),
    };

    // Collect all IDs currently in Cosmos for this subsource.
    let cosmos_ids =
        match list_cosmos_ids(cosmos, connector.primary_container(), &key.subsource).await {
            Ok(ids) => ids,
            Err(e) => return Err(format!("list cosmos ids: {e}")),
        };

    let now = Utc::now();
    let mut deleted = 0u64;

    for row in &cosmos_ids {
        if source_ids.contains(&row.id) || row.already_deleted {
            // Still in source, or already soft-deleted — nothing to do.
            continue;
        }

        // Read the existing doc, stamp it, and re-upsert.
        let doc = match cosmos
            .get(connector.primary_container(), &row.id, &row.partition_key)
            .await
        {
            Ok(Some(d)) => d,
            Ok(None) => continue, // race — doc was removed before we got here
            Err(e) => return Err(format!("get doc '{}': {e}", row.id)),
        };

        let mut doc = doc;
        if let Some(obj) = doc.as_object_mut() {
            obj.insert("_deleted".into(), Value::Bool(true));
            obj.insert("_deleted_at".into(), now.to_rfc3339().into());
        }

        if let Err(e) = cosmos.upsert(connector.primary_container(), doc).await {
            return Err(format!("mark deleted '{}': {e}", row.id));
        }

        deleted += 1;
    }

    // Update cursor with reconciliation metadata. Failures are non-fatal: the
    // soft-deletes already landed in Cosmos, but we want a record of when the
    // reconciliation last ran so the operator dashboard reflects it.
    let mut cursor = meta::load(cosmos, &cfg.meta_container, key)
        .await
        .unwrap_or_default();
    cursor.last_reconciliation_at = Some(now);
    cursor.last_reconciliation_deleted = deleted;
    if let Err(e) = meta::save(cosmos, &cfg.meta_container, key, &cursor).await {
        tracing::warn!(
            error = %e,
            key = %key.id(),
            "could not persist reconciliation cursor metadata"
        );
    }

    Ok(deleted)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// A row returned by `list_cosmos_ids`.
struct CosmosIdRow {
    id: String,
    partition_key: String,
    already_deleted: bool,
}

/// List every `(id, partition_key, already_deleted)` triple in `container`
/// whose `_partition_key` equals `subsource`.
async fn list_cosmos_ids(
    cosmos: &dyn CosmosBackend,
    container: &str,
    subsource: &str,
) -> Result<Vec<CosmosIdRow>, String> {
    let sql = "SELECT * FROM c WHERE c._partition_key = @pk".to_string();
    let params = vec![("@pk".into(), subsource.into())];
    let mut stream = cosmos
        .query(container, &sql, params)
        .await
        .map_err(|e| format!("query: {e}"))?;

    let mut out = Vec::new();
    while let Some(page) = stream.next_page().await.map_err(|e| format!("page: {e}"))? {
        for doc in page {
            if let Some(obj) = doc.as_object() {
                let id = obj
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let pk = obj
                    .get("_partition_key")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let already_deleted = obj
                    .get("_deleted")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                out.push(CosmosIdRow {
                    id,
                    partition_key: pk,
                    already_deleted,
                });
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::{
        cosmos::{CosmosBackend, InMemoryCosmos, meta::CursorKey},
        ingest::{config::CycleConfig, test_helpers::MockConnector},
    };

    fn make_key() -> CursorKey {
        CursorKey {
            deployment_name: "test".into(),
            source_name: "mock".into(),
            subsource: "DO".into(),
        }
    }

    /// Insert a document with the given id into the primary container.
    async fn insert_doc(cosmos: &InMemoryCosmos, container: &str, id: &str, pk: &str) {
        let doc = json!({
            "id": id,
            "_partition_key": pk,
            "title": format!("Doc {id}"),
        });
        cosmos.upsert(container, doc).await.unwrap();
    }

    /// Insert an already-soft-deleted document.
    async fn insert_deleted_doc(cosmos: &InMemoryCosmos, container: &str, id: &str, pk: &str) {
        let doc = json!({
            "id": id,
            "_partition_key": pk,
            "title": format!("Deleted {id}"),
            "_deleted": true,
            "_deleted_at": "2024-01-01T00:00:00Z",
        });
        cosmos.upsert(container, doc).await.unwrap();
    }

    // -------------------------------------------------------------------------
    // reconcile_marks_missing_docs_deleted
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn reconcile_marks_missing_docs_deleted() {
        let cosmos = InMemoryCosmos::new();
        let key = make_key();

        // Pre-populate: A, B, C in Cosmos.
        insert_doc(&cosmos, "jira-issues", "A", "DO").await;
        insert_doc(&cosmos, "jira-issues", "B", "DO").await;
        insert_doc(&cosmos, "jira-issues", "C", "DO").await;

        // Source only has A and C → B is missing.
        let connector = MockConnector::new("mock", "jira-issues");
        connector.set_list_ids(vec!["A".into(), "C".into()]);

        let cfg = CycleConfig::default();
        let deleted = run(&connector, &cosmos, &key, &cfg).await.unwrap();

        assert_eq!(deleted, 1, "expected 1 doc marked deleted");

        // B must be soft-deleted.
        let b = cosmos.get("jira-issues", "B", "DO").await.unwrap().unwrap();
        assert_eq!(b["_deleted"], json!(true));
        assert!(b.get("_deleted_at").is_some());

        // A and C must be untouched.
        let a = cosmos.get("jira-issues", "A", "DO").await.unwrap().unwrap();
        assert!(a.get("_deleted").is_none() || a["_deleted"] != json!(true));
        let c = cosmos.get("jira-issues", "C", "DO").await.unwrap().unwrap();
        assert!(c.get("_deleted").is_none() || c["_deleted"] != json!(true));
    }

    // -------------------------------------------------------------------------
    // reconcile_does_not_touch_already_deleted_docs
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn reconcile_does_not_touch_already_deleted_docs() {
        let cosmos = InMemoryCosmos::new();
        let key = make_key();

        // B is already soft-deleted.
        insert_doc(&cosmos, "jira-issues", "A", "DO").await;
        insert_deleted_doc(&cosmos, "jira-issues", "B", "DO").await;

        // Source only has A — B is absent but already marked deleted.
        let connector = MockConnector::new("mock", "jira-issues");
        connector.set_list_ids(vec!["A".into()]);

        let cfg = CycleConfig::default();
        let deleted = run(&connector, &cosmos, &key, &cfg).await.unwrap();

        // B was already deleted, so reconcile should not count it again.
        assert_eq!(
            deleted, 0,
            "expected 0 new deletions — B was already deleted"
        );
    }

    // -------------------------------------------------------------------------
    // reconcile_updates_cursor_state
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn reconcile_updates_cursor_state() {
        let cosmos = InMemoryCosmos::new();
        let key = make_key();

        insert_doc(&cosmos, "jira-issues", "X", "DO").await;
        insert_doc(&cosmos, "jira-issues", "Y", "DO").await;

        let connector = MockConnector::new("mock", "jira-issues");
        // X is gone.
        connector.set_list_ids(vec!["Y".into()]);

        let cfg = CycleConfig::default();
        let deleted = run(&connector, &cosmos, &key, &cfg).await.unwrap();
        assert_eq!(deleted, 1);

        // Cursor state must be updated.
        let cursor = meta::load(&cosmos, "quelch-meta", &key).await.unwrap();
        assert!(
            cursor.last_reconciliation_at.is_some(),
            "last_reconciliation_at must be set"
        );
        assert_eq!(
            cursor.last_reconciliation_deleted, 1,
            "last_reconciliation_deleted must be 1"
        );
    }

    // -------------------------------------------------------------------------
    // reconcile_empty_source_deletes_all
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn reconcile_empty_source_deletes_all() {
        let cosmos = InMemoryCosmos::new();
        let key = make_key();

        insert_doc(&cosmos, "jira-issues", "P", "DO").await;
        insert_doc(&cosmos, "jira-issues", "Q", "DO").await;

        let connector = MockConnector::new("mock", "jira-issues");
        connector.set_list_ids(vec![]); // source has nothing

        let cfg = CycleConfig::default();
        let deleted = run(&connector, &cosmos, &key, &cfg).await.unwrap();
        assert_eq!(deleted, 2);
    }

    // -------------------------------------------------------------------------
    // reconcile_no_op_when_cosmos_empty
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn reconcile_no_op_when_cosmos_empty() {
        let cosmos = InMemoryCosmos::new();
        let key = make_key();

        let connector = MockConnector::new("mock", "jira-issues");
        connector.set_list_ids(vec!["A".into()]);

        let cfg = CycleConfig::default();
        let deleted = run(&connector, &cosmos, &key, &cfg).await.unwrap();
        assert_eq!(deleted, 0);
    }
}
