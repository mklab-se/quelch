//! `quelch status` — show sync-cursor state for all (or filtered) sources.
//!
//! Reads `quelch-meta` from Cosmos and renders a human-readable table (or
//! machine-readable JSON when `--json` is passed).

use std::sync::Arc;

use crate::config::Config;
use crate::cosmos::meta::Cursor;
use crate::cosmos::meta::CursorKey;
use crate::cosmos::{factory::build_cosmos_backend, meta};
use chrono::Utc;
use colored::Colorize;

/// Options for `quelch status`.
#[derive(Debug, Default)]
pub struct StatusOptions {
    /// When set, filter results to only cursors from this deployment.
    pub deployment: Option<String>,
    /// Emit machine-readable JSON instead of a table.
    pub json: bool,
    /// Launch the interactive TUI. Planned for Phase 10; errors out for now.
    pub tui: bool,
}

/// Run `quelch status`.
pub async fn run(config: &Config, options: StatusOptions) -> anyhow::Result<()> {
    if options.tui {
        let cosmos = build_cosmos_backend(config).await?;
        return crate::tui::run_status_dashboard(
            Arc::from(cosmos),
            config.cosmos.meta_container.clone(),
            std::time::Duration::from_secs(5),
        )
        .await;
    }

    let cosmos = build_cosmos_backend(config).await?;
    let cursors = meta::list_all(cosmos.as_ref(), &config.cosmos.meta_container).await?;

    let filtered: Vec<_> = cursors
        .into_iter()
        .filter(|(key, _)| match &options.deployment {
            Some(d) => &key.deployment_name == d,
            None => true,
        })
        .collect();

    if options.json {
        let payload: Vec<_> = filtered
            .iter()
            .map(|(k, c)| {
                serde_json::json!({
                    "deployment": k.deployment_name,
                    "source": k.source_name,
                    "subsource": k.subsource,
                    "last_complete_minute": c.last_complete_minute,
                    "documents_synced_total": c.documents_synced_total,
                    "last_sync_at": c.last_sync_at,
                    "last_error": c.last_error,
                    "backfill_in_progress": c.backfill_in_progress,
                    "last_reconciliation_at": c.last_reconciliation_at,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        print_status_table(&filtered);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Table rendering
// ---------------------------------------------------------------------------

/// Render a human-readable status table to stdout.
fn print_status_table(rows: &[(CursorKey, Cursor)]) {
    let sep = "─".repeat(85);
    println!("Quelch status");
    println!("{sep}");
    println!(
        "{:<22} {:<18} {:<12} {:<12} {:<8} State",
        "Deployment", "Source", "Subsource", "Last sync", "Docs"
    );
    println!("{sep}");

    if rows.is_empty() {
        println!("  (no cursors found — nothing has synced yet)");
    } else {
        for (key, cursor) in rows {
            let last_sync = fmt_last_sync(cursor);
            let docs = if cursor.documents_synced_total == 0 {
                "—".to_string()
            } else {
                cursor.documents_synced_total.to_string()
            };
            let state = fmt_state(cursor);

            println!(
                "{:<22} {:<18} {:<12} {:<12} {:<8} {}",
                key.deployment_name, key.source_name, key.subsource, last_sync, docs, state
            );
        }
    }

    println!("{sep}");
}

/// Format the last sync time as a relative "N ago" string.
fn fmt_last_sync(cursor: &Cursor) -> String {
    if cursor.backfill_in_progress {
        return "backfill...".to_string();
    }
    match cursor.last_sync_at {
        None => "—".to_string(),
        Some(t) => {
            let secs = Utc::now().signed_duration_since(t).num_seconds();
            if secs < 0 {
                "just now".to_string()
            } else if secs < 120 {
                format!("{secs}s ago")
            } else if secs < 7200 {
                format!("{}m ago", secs / 60)
            } else {
                format!("{}h ago", secs / 3600)
            }
        }
    }
}

/// Format the state indicator: `ok`, `backfilling`, or `error: <msg>`.
fn fmt_state(cursor: &Cursor) -> String {
    if let Some(err) = &cursor.last_error {
        let msg = format!("error: {}", err.chars().take(40).collect::<String>());
        return msg.red().to_string();
    }
    if cursor.backfill_in_progress {
        return "backfilling".yellow().to_string();
    }
    "ok".green().to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosmos::InMemoryCosmos;
    use crate::cosmos::meta::{Cursor, CursorKey, save};
    use chrono::Utc;
    use serde_json::Value;

    const META: &str = "quelch-meta";

    fn key(deployment: &str, source: &str, subsource: &str) -> CursorKey {
        CursorKey {
            deployment_name: deployment.to_string(),
            source_name: source.to_string(),
            subsource: subsource.to_string(),
        }
    }

    /// Populate an `InMemoryCosmos` with cursors from two deployments and
    /// return the raw list, so tests can call the filter logic.
    async fn populate_two_deployments() -> Vec<(CursorKey, Cursor)> {
        let cosmos = InMemoryCosmos::new();

        let k1 = key("prod", "jira-cloud", "DO");
        let c1 = Cursor {
            documents_synced_total: 1842,
            last_sync_at: Some(Utc::now()),
            ..Default::default()
        };
        save(&cosmos, META, &k1, &c1).await.unwrap();

        let k2 = key("prod", "jira-cloud", "INT");
        let c2 = Cursor {
            documents_synced_total: 312,
            last_sync_at: Some(Utc::now()),
            ..Default::default()
        };
        save(&cosmos, META, &k2, &c2).await.unwrap();

        let k3 = key("staging", "confluence", "WIKI");
        let c3 = Cursor {
            documents_synced_total: 99,
            last_error: Some("429 too many requests".to_string()),
            ..Default::default()
        };
        save(&cosmos, META, &k3, &c3).await.unwrap();

        meta::list_all(&cosmos, META).await.unwrap()
    }

    #[tokio::test]
    async fn status_filter_by_deployment_returns_only_matching() {
        let all = populate_two_deployments().await;

        // Simulate the filter logic from `run`.
        let filtered: Vec<_> = all
            .iter()
            .filter(|(k, _)| k.deployment_name == "prod")
            .collect();

        assert_eq!(filtered.len(), 2, "should see exactly 2 prod cursors");
        for (key, _) in &filtered {
            assert_eq!(key.deployment_name, "prod");
        }
    }

    #[tokio::test]
    async fn status_filter_none_returns_all() {
        let all = populate_two_deployments().await;

        // No filter
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn status_json_output_is_valid() {
        let cosmos = InMemoryCosmos::new();
        let k = key("prod", "jira-cloud", "DO");
        let c = Cursor {
            documents_synced_total: 42,
            last_sync_at: Some(Utc::now()),
            ..Default::default()
        };
        save(&cosmos, META, &k, &c).await.unwrap();

        let rows = meta::list_all(&cosmos, META).await.unwrap();

        // Build JSON the same way `run` does.
        let payload: Vec<serde_json::Value> = rows
            .iter()
            .map(|(k, c)| {
                serde_json::json!({
                    "deployment": k.deployment_name,
                    "source": k.source_name,
                    "subsource": k.subsource,
                    "last_complete_minute": c.last_complete_minute,
                    "documents_synced_total": c.documents_synced_total,
                    "last_sync_at": c.last_sync_at,
                    "last_error": c.last_error,
                    "backfill_in_progress": c.backfill_in_progress,
                    "last_reconciliation_at": c.last_reconciliation_at,
                })
            })
            .collect();

        let json_str = serde_json::to_string_pretty(&payload).unwrap();

        // Must parse back cleanly.
        let parsed: Value = serde_json::from_str(&json_str).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["deployment"], "prod");
        assert_eq!(arr[0]["documents_synced_total"], 42);
    }

    #[tokio::test]
    async fn status_backfilling_shows_in_state() {
        let c = Cursor {
            backfill_in_progress: true,
            ..Default::default()
        };

        let state = fmt_state(&c);
        // Strip ANSI codes for assertion (colored may or may not add them in tests)
        assert!(state.contains("backfilling"));
    }

    #[tokio::test]
    async fn status_error_shows_in_state() {
        let c = Cursor {
            last_error: Some("429 rate limit".to_string()),
            ..Default::default()
        };

        let state = fmt_state(&c);
        assert!(state.contains("error:"));
        assert!(state.contains("429"));
    }
}
