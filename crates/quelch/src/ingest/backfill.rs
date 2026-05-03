//! Initial backfill and backfill-resume logic.
//!
//! See `docs/sync.md` — "Initial backfill" for the full algorithm.

use chrono::{Duration, Utc};

use crate::{
    cosmos::meta::{Cursor, CursorKey},
    cosmos::{CosmosBackend, meta},
    ingest::{config::CycleConfig, cycle::CycleOutcome, window::floor_to_minute},
    sources::SourceConnector,
};

use crate::sources::BackfillCheckpoint as SourceCheckpoint;

use super::cycle::document_envelope;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Start a fresh backfill for a `(source, subsource)` pair that has never
/// been synced.
///
/// Sets `backfill_in_progress = true` and `backfill_target` to the current
/// safety-lagged minute, then immediately calls [`resume`] to process the
/// first batch of pages.
pub async fn start<C>(
    connector: &C,
    cosmos: &dyn CosmosBackend,
    key: &CursorKey,
    cfg: &CycleConfig,
) -> CycleOutcome
where
    C: SourceConnector,
{
    let now = Utc::now();
    let target = floor_to_minute(now) - Duration::minutes(i64::from(cfg.safety_lag_minutes));

    let mut cursor = meta::load(cosmos, &cfg.meta_container, key)
        .await
        .unwrap_or_default();

    cursor.backfill_in_progress = true;
    cursor.backfill_target = Some(target);
    cursor.backfill_last_seen = None;

    if let Err(e) = meta::save(cosmos, &cfg.meta_container, key, &cursor).await {
        return CycleOutcome::Failed {
            error: format!("save cursor (backfill start): {e}"),
        };
    }

    resume(connector, cosmos, key, cursor, cfg).await
}

/// Resume (or complete) an in-progress backfill.
///
/// Fetches pages until the source signals completion (empty page), persisting
/// a checkpoint after every page so the operation can be safely interrupted
/// and restarted.
pub async fn resume<C>(
    connector: &C,
    cosmos: &dyn CosmosBackend,
    key: &CursorKey,
    mut cursor: Cursor,
    cfg: &CycleConfig,
) -> CycleOutcome
where
    C: SourceConnector,
{
    let target = match cursor.backfill_target {
        Some(t) => t,
        None => {
            return CycleOutcome::Failed {
                error: "backfill_in_progress=true but backfill_target is None".into(),
            };
        }
    };

    let mut total = 0usize;

    loop {
        // Convert meta::BackfillCheckpoint → sources::BackfillCheckpoint for the API call.
        let source_last_seen: Option<SourceCheckpoint> =
            cursor.backfill_last_seen.as_ref().map(meta_ckpt_to_source);

        let page = match connector
            .fetch_backfill_page(
                &key.subsource,
                target,
                source_last_seen.as_ref(),
                cfg.batch_size,
            )
            .await
        {
            Ok(p) => p,
            Err(e) => {
                // Persist whatever progress has been made so far.
                let _ = meta::save(cosmos, &cfg.meta_container, key, &cursor).await;
                return CycleOutcome::Failed {
                    error: format!("fetch_backfill_page: {e}"),
                };
            }
        };

        if page.documents.is_empty() {
            // End of backfill data.
            break;
        }

        let docs: Vec<serde_json::Value> = page
            .documents
            .iter()
            .map(|d| document_envelope(d, connector.source_name()))
            .collect();

        if let Err(e) = cosmos
            .bulk_upsert(connector.primary_container(), docs)
            .await
        {
            let _ = meta::save(cosmos, &cfg.meta_container, key, &cursor).await;
            return CycleOutcome::Failed {
                error: format!("upsert (backfill): {e}"),
            };
        }

        total += page.documents.len();

        // Persist checkpoint after each successful page so we can resume
        // after a crash without re-fetching everything.
        // Convert sources::BackfillCheckpoint → meta::BackfillCheckpoint.
        cursor.backfill_last_seen = page.last_seen.map(source_ckpt_to_meta);
        cursor.documents_synced_total += page.documents.len() as u64;
        cursor.last_sync_at = Some(Utc::now());

        if let Err(e) = meta::save(cosmos, &cfg.meta_container, key, &cursor).await {
            return CycleOutcome::Failed {
                error: format!("save cursor (backfill page): {e}"),
            };
        }
    }

    // -------------------------------------------------------------------------
    // Backfill complete — flip flags and set last_complete_minute.
    // -------------------------------------------------------------------------
    cursor.last_complete_minute = Some(target);
    cursor.backfill_in_progress = false;
    cursor.backfill_target = None;
    cursor.backfill_last_seen = None;
    cursor.last_error = None;

    if let Err(e) = meta::save(cosmos, &cfg.meta_container, key, &cursor).await {
        return CycleOutcome::Failed {
            error: format!("save cursor (backfill complete): {e}"),
        };
    }

    CycleOutcome::BackfillCompleted {
        documents_written: total,
        target,
    }
}

// ---------------------------------------------------------------------------
// Checkpoint conversions
// ---------------------------------------------------------------------------

/// Convert a `cosmos::meta::BackfillCheckpoint` to a `sources::BackfillCheckpoint`.
///
/// Both types have identical fields; they exist in separate modules because
/// `meta` owns the cursor schema and `sources` owns the connector API.
fn meta_ckpt_to_source(ckpt: &crate::cosmos::meta::BackfillCheckpoint) -> SourceCheckpoint {
    SourceCheckpoint {
        updated: ckpt.updated,
        key: ckpt.key.clone(),
    }
}

/// Convert a `sources::BackfillCheckpoint` to a `cosmos::meta::BackfillCheckpoint`.
fn source_ckpt_to_meta(ckpt: SourceCheckpoint) -> crate::cosmos::meta::BackfillCheckpoint {
    crate::cosmos::meta::BackfillCheckpoint {
        updated: ckpt.updated,
        key: ckpt.key,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    use crate::{
        cosmos::{
            InMemoryCosmos,
            meta::{Cursor, CursorKey},
        },
        ingest::{
            config::CycleConfig,
            cycle::CycleOutcome,
            test_helpers::{MockConnector, make_source_doc},
        },
        sources::BackfillCheckpoint,
    };

    fn make_key() -> CursorKey {
        CursorKey {
            deployment_name: "test".into(),
            source_name: "mock".into(),
            subsource: "DO".into(),
        }
    }

    fn ts(h: u32, m: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 6, 1, h, m, 0).single().unwrap()
    }

    // -------------------------------------------------------------------------
    // backfill_completes_and_clears_flags
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn backfill_completes_and_clears_flags() {
        let cosmos = InMemoryCosmos::new();
        let key = make_key();

        let connector = MockConnector::new("mock", "jira-issues");
        // Two backfill pages; third call returns empty → done.
        let ckpt1 = BackfillCheckpoint {
            updated: ts(9, 30),
            key: "DO-5".into(),
        };
        connector.push_backfill_page(
            vec![make_source_doc("DO-1", "DO"), make_source_doc("DO-5", "DO")],
            Some(ckpt1.clone()),
        );
        connector.push_backfill_page(
            vec![make_source_doc("DO-10", "DO")],
            Some(BackfillCheckpoint {
                updated: ts(9, 40),
                key: "DO-10".into(),
            }),
        );
        // Third call → empty → end of backfill.

        let cfg = CycleConfig::default();
        let outcome = start(&connector, &cosmos, &key, &cfg).await;

        assert!(
            matches!(
                outcome,
                CycleOutcome::BackfillCompleted {
                    documents_written: 3,
                    ..
                }
            ),
            "expected BackfillCompleted(3), got {outcome:?}"
        );

        // Verify cursor state.
        let cursor = meta::load(&cosmos, "quelch-meta", &key).await.unwrap();
        assert!(!cursor.backfill_in_progress);
        assert!(cursor.backfill_target.is_none());
        assert!(cursor.backfill_last_seen.is_none());
        assert!(cursor.last_complete_minute.is_some());
        assert!(cursor.last_error.is_none());
        assert_eq!(cursor.documents_synced_total, 3);
    }

    // -------------------------------------------------------------------------
    // backfill_resumes_after_crash
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn backfill_resumes_after_crash() {
        let cosmos = InMemoryCosmos::new();
        let key = make_key();

        // Simulate a crash after page 1: manually set a cursor with
        // backfill_in_progress=true, backfill_target set, and backfill_last_seen
        // pointing to the last doc from page 1.
        let target = floor_to_minute(Utc::now()) - Duration::minutes(2);
        let mut cursor = Cursor::default();
        cursor.backfill_in_progress = true;
        cursor.backfill_target = Some(target);
        cursor.backfill_last_seen = Some(crate::cosmos::meta::BackfillCheckpoint {
            updated: ts(9, 30),
            key: "DO-5".into(),
        });
        cursor.documents_synced_total = 2; // already wrote 2 docs in page 1
        meta::save(&cosmos, "quelch-meta", &key, &cursor)
            .await
            .unwrap();

        // Now set up the connector to return pages starting from the resume point.
        let connector = MockConnector::new("mock", "jira-issues");
        // Page 2 (the only page on resume — page 1 was already persisted).
        connector.push_backfill_page(
            vec![
                make_source_doc("DO-10", "DO"),
                make_source_doc("DO-15", "DO"),
            ],
            Some(BackfillCheckpoint {
                updated: ts(9, 45),
                key: "DO-15".into(),
            }),
        );
        // Third call → empty → done.

        let cfg = CycleConfig::default();
        let outcome = resume(&connector, &cosmos, &key, cursor, &cfg).await;

        assert!(
            matches!(
                outcome,
                CycleOutcome::BackfillCompleted {
                    documents_written: 2,
                    ..
                }
            ),
            "expected BackfillCompleted(2), got {outcome:?}"
        );

        // Cursor must be clean.
        let loaded = meta::load(&cosmos, "quelch-meta", &key).await.unwrap();
        assert!(!loaded.backfill_in_progress);
        assert!(loaded.backfill_target.is_none());
        assert!(loaded.backfill_last_seen.is_none());
        // Total should include the pre-existing 2 from the simulated crash.
        assert_eq!(loaded.documents_synced_total, 4);
    }

    // -------------------------------------------------------------------------
    // backfill_persists_progress_on_mid_run_failure
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn backfill_persists_progress_on_mid_run_failure() {
        let cosmos = InMemoryCosmos::new();
        let key = make_key();

        let connector = MockConnector::new("mock", "jira-issues");
        let ckpt1 = BackfillCheckpoint {
            updated: ts(9, 10),
            key: "DO-1".into(),
        };
        let ckpt2 = BackfillCheckpoint {
            updated: ts(9, 20),
            key: "DO-2".into(),
        };
        // Page 1 succeeds.
        connector.push_backfill_page(vec![make_source_doc("DO-1", "DO")], Some(ckpt1.clone()));
        // Page 2 succeeds.
        connector.push_backfill_page(vec![make_source_doc("DO-2", "DO")], Some(ckpt2.clone()));
        // Page 3 errors.
        connector.push_backfill_error("source timeout on page 3");

        let cfg = CycleConfig::default();
        let outcome = start(&connector, &cosmos, &key, &cfg).await;

        assert!(
            matches!(outcome, CycleOutcome::Failed { .. }),
            "expected Failed, got {outcome:?}"
        );

        // Progress from pages 1 and 2 must have been persisted.
        let cursor = meta::load(&cosmos, "quelch-meta", &key).await.unwrap();
        assert!(
            cursor.backfill_in_progress,
            "backfill_in_progress should still be true"
        );
        // last_seen should be from page 2.
        let last_seen = cursor
            .backfill_last_seen
            .expect("backfill_last_seen should be set");
        assert_eq!(last_seen.key, "DO-2");
        assert_eq!(cursor.documents_synced_total, 2);
    }
}
