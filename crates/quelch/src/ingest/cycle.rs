//! Per-cycle ingest algorithm.
//!
//! `cycle::run` is the main entrypoint: it reads the cursor, delegates to
//! backfill if needed, or advances the steady-state incremental window.

use chrono::Utc;
use serde_json::Value;

use crate::{
    cosmos::meta::CursorKey,
    cosmos::{CosmosBackend, meta},
    ingest::{backfill, config::CycleConfig, window},
    sources::{Companions, SourceConnector, SourceDocument},
};

// ---------------------------------------------------------------------------
// Outcome
// ---------------------------------------------------------------------------

/// The result of one `cycle::run` call.
#[derive(Debug)]
pub enum CycleOutcome {
    /// Cursor advanced; window completed; N docs written.
    Advanced {
        documents_written: usize,
        window_end: chrono::DateTime<Utc>,
    },
    /// Backfill in progress; this cycle made progress but isn't done yet.
    BackfillInProgress { documents_written: usize },
    /// Backfill completed.
    BackfillCompleted {
        documents_written: usize,
        target: chrono::DateTime<Utc>,
    },
    /// Cursor already past `target_now - safety_lag`; nothing to do.
    NothingToDo,
    /// Cycle failed mid-flight; cursor not advanced.
    Failed { error: String },
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run one ingest cycle for `(key.source_name, key.subsource)`.
///
/// Reads the cursor from Cosmos, then:
///
/// - If `backfill_in_progress` → delegates to [`backfill::resume`].
/// - If `last_complete_minute` is `None` → starts a fresh backfill.
/// - Otherwise → advances the steady-state incremental window.
pub async fn run<C>(
    connector: &C,
    cosmos: &dyn CosmosBackend,
    key: &CursorKey,
    cfg: &CycleConfig,
) -> CycleOutcome
where
    C: SourceConnector,
{
    let cursor = match meta::load(cosmos, &cfg.meta_container, key).await {
        Ok(c) => c,
        Err(e) => {
            return CycleOutcome::Failed {
                error: format!("load cursor: {e}"),
            };
        }
    };

    // Delegate to backfill path if needed.
    if cursor.backfill_in_progress {
        return backfill::resume(connector, cosmos, key, cursor, cfg).await;
    }

    if cursor.last_complete_minute.is_none() {
        // First-ever cycle for this (source, subsource): start backfill.
        return backfill::start(connector, cosmos, key, cfg).await;
    }

    // -----------------------------------------------------------------------
    // Steady-state incremental window.
    // -----------------------------------------------------------------------
    let now = Utc::now();
    let Some(w) =
        window::plan_next_window(cursor.last_complete_minute, now, cfg.safety_lag_minutes)
    else {
        return CycleOutcome::NothingToDo;
    };

    let mut total_written = 0usize;
    let mut page_token: Option<String> = None;

    loop {
        let page = match connector
            .fetch_window(
                &key.subsource,
                w.start,
                w.end,
                cfg.batch_size,
                page_token.as_deref(),
            )
            .await
        {
            Ok(p) => p,
            Err(e) => {
                return CycleOutcome::Failed {
                    error: format!("fetch_window: {e}"),
                };
            }
        };

        if !page.documents.is_empty() {
            let docs: Vec<Value> = page
                .documents
                .iter()
                .map(|d| document_envelope(d, connector.source_name()))
                .collect();
            if let Err(e) = cosmos
                .bulk_upsert(connector.primary_container(), docs)
                .await
            {
                return CycleOutcome::Failed {
                    error: format!("upsert: {e}"),
                };
            }
        }
        total_written += page.documents.len();

        match page.next_page_token {
            Some(t) => page_token = Some(t),
            None => break,
        }
    }

    // Companions (sprints, fix_versions, projects, spaces). Fetch once per cycle.
    if let Ok(companions) = connector.fetch_companions(&key.subsource).await {
        upsert_companions(cosmos, &companions, cfg).await.ok();
    }

    // Advance cursor — only on full success.
    let mut new_cursor = cursor;
    new_cursor.last_complete_minute = Some(w.end);
    new_cursor.documents_synced_total += total_written as u64;
    new_cursor.last_sync_at = Some(Utc::now());
    new_cursor.last_error = None;

    if let Err(e) = meta::save(cosmos, &cfg.meta_container, key, &new_cursor).await {
        return CycleOutcome::Failed {
            error: format!("save cursor: {e}"),
        };
    }

    CycleOutcome::Advanced {
        documents_written: total_written,
        window_end: w.end,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build the Cosmos document envelope for a [`SourceDocument`].
pub(crate) fn document_envelope(doc: &SourceDocument, source_name: &str) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("id".into(), doc.id.clone().into());
    map.insert("_partition_key".into(), doc.partition_key.clone().into());
    map.insert("source_name".into(), source_name.to_string().into());
    map.insert("source_link".into(), doc.source_link.clone().into());
    map.insert("updated".into(), doc.updated_at.to_rfc3339().into());
    for (k, v) in &doc.fields {
        map.insert(k.clone(), v.clone());
    }
    Value::Object(map)
}

/// Upsert companion documents to their dedicated containers.
async fn upsert_companions(
    cosmos: &dyn CosmosBackend,
    companions: &Companions,
    cfg: &CycleConfig,
) -> Result<(), String> {
    let pairs: &[(&str, &[SourceDocument])] = &[
        ("sprints", &companions.sprints),
        ("fix_versions", &companions.fix_versions),
        ("projects", &companions.projects),
        ("spaces", &companions.spaces),
    ];

    for (category, docs) in pairs {
        if docs.is_empty() {
            continue;
        }
        let Some(container) = cfg.companion_containers.get(*category) else {
            continue;
        };
        let values: Vec<Value> = docs
            .iter()
            .map(|d| document_envelope(d, category))
            .collect();
        cosmos
            .bulk_upsert(container, values)
            .await
            .map_err(|e| format!("companion upsert ({category}): {e}"))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    use crate::{
        cosmos::{InMemoryCosmos, meta::CursorKey},
        ingest::{
            config::CycleConfig,
            test_helpers::{MockConnector, make_source_doc},
        },
        sources::Companions,
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

    /// Seed the cosmos meta container with a cursor pointing at `last_minute`.
    async fn seed_cursor(
        cosmos: &InMemoryCosmos,
        key: &CursorKey,
        last_minute: chrono::DateTime<Utc>,
    ) {
        let cursor = crate::cosmos::meta::Cursor {
            last_complete_minute: Some(last_minute),
            ..Default::default()
        };
        meta::save(cosmos, "quelch-meta", key, &cursor)
            .await
            .unwrap();
    }

    // -------------------------------------------------------------------------
    // cycle_advances_cursor_after_full_window
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn cycle_advances_cursor_after_full_window() {
        let cosmos = InMemoryCosmos::new();
        let key = make_key();

        // Seed cursor at 10:00; now will be ~10:10, lag=2 → window [10:00, 10:08)
        seed_cursor(&cosmos, &key, ts(10, 0)).await;

        let connector = MockConnector::new("mock", "jira-issues");
        // Two docs, single page, no next_page_token.
        connector.push_window_page(
            vec![
                make_source_doc("doc-A", "DO"),
                make_source_doc("doc-B", "DO"),
            ],
            None,
        );

        // Use a fixed "now" by manipulating the safety_lag to guarantee a window.
        // With last=10:00, now=10:10, lag=2 → target=10:08.
        // We override safety_lag to 2 so the window is non-empty.
        let cfg = CycleConfig {
            safety_lag_minutes: 2,
            ..CycleConfig::default()
        };

        // We can't directly control "now" in cycle::run — we need
        // last_complete_minute to be sufficiently behind real time.
        // Seed the cursor at a point far enough in the past that
        // the safety-lagged target is > last_complete_minute.
        // Re-seed with a timestamp 10 minutes in the past.
        let past = Utc::now() - Duration::minutes(15);
        let past = crate::ingest::window::floor_to_minute(past);
        {
            let cursor = crate::cosmos::meta::Cursor {
                last_complete_minute: Some(past),
                ..Default::default()
            };
            meta::save(&cosmos, "quelch-meta", &key, &cursor)
                .await
                .unwrap();
        }

        let outcome = run(&connector, &cosmos, &key, &cfg).await;
        assert!(
            matches!(
                outcome,
                CycleOutcome::Advanced {
                    documents_written: 2,
                    ..
                }
            ),
            "expected Advanced(2), got {outcome:?}"
        );

        // Cursor must have advanced.
        let loaded = meta::load(&cosmos, "quelch-meta", &key).await.unwrap();
        assert!(loaded.last_complete_minute.unwrap() > past);
        assert_eq!(loaded.documents_synced_total, 2);
        assert!(loaded.last_error.is_none());
    }

    // -------------------------------------------------------------------------
    // cycle_writes_docs_to_primary_container
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn cycle_writes_docs_to_primary_container() {
        let cosmos = InMemoryCosmos::new();
        let key = make_key();

        let past = crate::ingest::window::floor_to_minute(Utc::now() - Duration::minutes(15));
        {
            let cursor = crate::cosmos::meta::Cursor {
                last_complete_minute: Some(past),
                ..Default::default()
            };
            meta::save(&cosmos, "quelch-meta", &key, &cursor)
                .await
                .unwrap();
        }

        let connector = MockConnector::new("mock", "jira-issues");
        connector.push_window_page(vec![make_source_doc("issue-1", "DO")], None);

        let cfg = CycleConfig::default();
        run(&connector, &cosmos, &key, &cfg).await;

        // The doc must be in the primary container.
        let doc = cosmos.get("jira-issues", "issue-1", "DO").await.unwrap();
        assert!(doc.is_some(), "expected doc in jira-issues container");
        let doc = doc.unwrap();
        assert_eq!(doc["id"], "issue-1");
        assert_eq!(doc["source_name"], "mock");
    }

    // -------------------------------------------------------------------------
    // cycle_does_not_advance_cursor_on_failure
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn cycle_does_not_advance_cursor_on_failure() {
        let cosmos = InMemoryCosmos::new();
        let key = make_key();

        let past = crate::ingest::window::floor_to_minute(Utc::now() - Duration::minutes(15));
        {
            let cursor = crate::cosmos::meta::Cursor {
                last_complete_minute: Some(past),
                ..Default::default()
            };
            meta::save(&cosmos, "quelch-meta", &key, &cursor)
                .await
                .unwrap();
        }

        let connector = MockConnector::new("mock", "jira-issues");
        // First page succeeds, second page errors.
        connector.push_window_page(vec![make_source_doc("doc-1", "DO")], Some("page2".into()));
        connector.push_window_error("network failure");

        let cfg = CycleConfig::default();
        let outcome = run(&connector, &cosmos, &key, &cfg).await;

        assert!(
            matches!(outcome, CycleOutcome::Failed { .. }),
            "expected Failed, got {outcome:?}"
        );

        // Cursor must NOT have advanced.
        let loaded = meta::load(&cosmos, "quelch-meta", &key).await.unwrap();
        assert_eq!(
            loaded.last_complete_minute.unwrap(),
            past,
            "cursor should not have advanced after failure"
        );
    }

    // -------------------------------------------------------------------------
    // cycle_skips_when_no_progress_possible
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn cycle_skips_when_no_progress_possible() {
        let cosmos = InMemoryCosmos::new();
        let key = make_key();

        // Seed cursor with a timestamp only 1 minute in the past, lag=2 → no window.
        let recent = crate::ingest::window::floor_to_minute(Utc::now() - Duration::minutes(1));
        {
            let cursor = crate::cosmos::meta::Cursor {
                last_complete_minute: Some(recent),
                ..Default::default()
            };
            meta::save(&cosmos, "quelch-meta", &key, &cursor)
                .await
                .unwrap();
        }

        let connector = MockConnector::new("mock", "jira-issues");
        let cfg = CycleConfig {
            safety_lag_minutes: 2,
            ..CycleConfig::default()
        };
        let outcome = run(&connector, &cosmos, &key, &cfg).await;

        assert!(
            matches!(outcome, CycleOutcome::NothingToDo),
            "expected NothingToDo, got {outcome:?}"
        );
    }

    // -------------------------------------------------------------------------
    // cycle_writes_companions
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn cycle_writes_companions() {
        let cosmos = InMemoryCosmos::new();
        let key = make_key();

        let past = crate::ingest::window::floor_to_minute(Utc::now() - Duration::minutes(15));
        {
            let cursor = crate::cosmos::meta::Cursor {
                last_complete_minute: Some(past),
                ..Default::default()
            };
            meta::save(&cosmos, "quelch-meta", &key, &cursor)
                .await
                .unwrap();
        }

        let connector = MockConnector::new("mock", "jira-issues");
        connector.push_window_page(vec![], None);

        // Set up companions — one sprint.
        let mut sprint = make_source_doc("sprint-1", "DO");
        sprint.id = "sprint-1".into();
        let companions = Companions {
            sprints: vec![sprint],
            ..Default::default()
        };
        connector.set_companions(companions);

        let cfg = CycleConfig::default();
        run(&connector, &cosmos, &key, &cfg).await;

        // Sprint must be in the sprints container.
        let sprint_doc = cosmos.get("jira-sprints", "sprint-1", "DO").await.unwrap();
        assert!(
            sprint_doc.is_some(),
            "expected sprint in jira-sprints container"
        );
    }

    // -------------------------------------------------------------------------
    // cycle_starts_backfill_when_cursor_unset
    // -------------------------------------------------------------------------
    #[tokio::test]
    async fn cycle_starts_backfill_when_cursor_unset() {
        let cosmos = InMemoryCosmos::new();
        let key = make_key();
        // No cursor saved → last_complete_minute is None.

        let connector = MockConnector::new("mock", "jira-issues");
        // No backfill pages → immediate end of backfill.

        let cfg = CycleConfig::default();
        let outcome = run(&connector, &cosmos, &key, &cfg).await;

        // With no backfill pages, backfill::resume immediately completes.
        assert!(
            matches!(
                outcome,
                CycleOutcome::BackfillCompleted { .. } | CycleOutcome::BackfillInProgress { .. }
            ),
            "expected BackfillCompleted or BackfillInProgress, got {outcome:?}"
        );
    }

    // -------------------------------------------------------------------------
    // document_envelope helper
    // -------------------------------------------------------------------------
    #[test]
    fn document_envelope_includes_required_fields() {
        let doc = make_source_doc("my-id", "my-pk");
        let env = document_envelope(&doc, "test-source");
        assert_eq!(env["id"], "my-id");
        assert_eq!(env["_partition_key"], "my-pk");
        assert_eq!(env["source_name"], "test-source");
        assert!(env.get("source_link").is_some());
        assert!(env.get("updated").is_some());
        assert_eq!(env["title"], "Doc my-id");
    }
}
