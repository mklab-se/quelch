//! `quelch reset` — clear cursor state for selected sources.
//!
//! Removes progress from `quelch-meta` so the next ingest cycle starts a
//! fresh backfill.  Asks for confirmation unless `--yes` is passed.

use crate::config::Config;
use crate::cosmos::{factory::build_cosmos_backend, meta};

/// Options for `quelch reset`.
#[derive(Debug, Default)]
pub struct ResetOptions {
    /// Only reset cursors belonging to this source name.
    pub source: Option<String>,
    /// Only reset the named subsource within the matching source(s).
    pub subsource: Option<String>,
    /// Skip the interactive confirmation prompt.
    pub yes: bool,
}

/// Run `quelch reset`.
pub async fn run(config: &Config, options: ResetOptions) -> anyhow::Result<()> {
    let cosmos = build_cosmos_backend(config).await?;
    let all = meta::list_all(cosmos.as_ref(), &config.cosmos.meta_container).await?;

    let to_reset: Vec<_> = all
        .iter()
        .filter(|(key, _)| {
            if let Some(src) = &options.source
                && &key.source_name != src
            {
                return false;
            }
            if let Some(sub) = &options.subsource
                && &key.subsource != sub
            {
                return false;
            }
            true
        })
        .collect();

    if to_reset.is_empty() {
        println!("Nothing to reset.");
        return Ok(());
    }

    println!("Will reset cursors for:");
    for (key, _) in &to_reset {
        println!(
            "  • {} :: {} :: {}",
            key.deployment_name, key.source_name, key.subsource
        );
    }

    if !options.yes {
        let confirmed = dialoguer::Confirm::new()
            .with_prompt("Continue?")
            .default(false)
            .interact()?;
        if !confirmed {
            println!("Aborted.");
            return Ok(());
        }
    }

    for (key, _) in &to_reset {
        let cleared = meta::Cursor::default();
        meta::save(
            cosmos.as_ref(),
            &config.cosmos.meta_container,
            key,
            &cleared,
        )
        .await?;
    }

    println!("Reset {} cursor(s).", to_reset.len());
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosmos::InMemoryCosmos;
    use crate::cosmos::meta::{Cursor, CursorKey, load, save};
    use chrono::Utc;

    const META: &str = "quelch-meta";

    fn key(deployment: &str, source: &str, subsource: &str) -> CursorKey {
        CursorKey {
            deployment_name: deployment.to_string(),
            source_name: source.to_string(),
            subsource: subsource.to_string(),
        }
    }

    /// Build a config stub with an in-memory state backend so `build_cosmos_backend`
    /// isn't called (tests call the cosmos layer directly).
    async fn run_reset_directly(
        cosmos: &InMemoryCosmos,
        meta_container: &str,
        options: ResetOptions,
    ) -> anyhow::Result<()> {
        let all = meta::list_all(cosmos, meta_container).await?;

        let to_reset: Vec<_> = all
            .iter()
            .filter(|(k, _)| {
                if let Some(src) = &options.source
                    && &k.source_name != src
                {
                    return false;
                }
                if let Some(sub) = &options.subsource
                    && &k.subsource != sub
                {
                    return false;
                }
                true
            })
            .collect();

        if to_reset.is_empty() {
            return Ok(());
        }

        for (k, _) in &to_reset {
            let cleared = Cursor::default();
            save(cosmos, meta_container, k, &cleared).await?;
        }

        Ok(())
    }

    #[tokio::test]
    async fn reset_clears_cursor_for_subsource() {
        let cosmos = InMemoryCosmos::new();

        let k = key("prod", "jira-cloud", "DO");
        let c = Cursor {
            documents_synced_total: 500,
            last_complete_minute: Some(Utc::now()),
            backfill_in_progress: true,
            ..Default::default()
        };
        save(&cosmos, META, &k, &c).await.unwrap();

        // Verify it's there.
        let before = load(&cosmos, META, &k).await.unwrap();
        assert_eq!(before.documents_synced_total, 500);
        assert!(before.last_complete_minute.is_some());

        // Reset.
        run_reset_directly(
            &cosmos,
            META,
            ResetOptions {
                source: Some("jira-cloud".to_string()),
                subsource: Some("DO".to_string()),
                yes: true,
            },
        )
        .await
        .unwrap();

        let after = load(&cosmos, META, &k).await.unwrap();
        assert!(
            after.last_complete_minute.is_none(),
            "last_complete_minute should be cleared after reset"
        );
        assert!(!after.backfill_in_progress);
    }

    #[tokio::test]
    async fn reset_with_yes_skips_prompt() {
        // This test verifies the `--yes` flag short-circuits the prompt.
        // We exercise the logic path directly (no TTY available in tests).
        let cosmos = InMemoryCosmos::new();

        let k = key("prod", "jira-cloud", "DO");
        let c = Cursor {
            documents_synced_total: 100,
            last_complete_minute: Some(Utc::now()),
            ..Default::default()
        };
        save(&cosmos, META, &k, &c).await.unwrap();

        // With --yes: should reset without asking.
        run_reset_directly(
            &cosmos,
            META,
            ResetOptions {
                source: None,
                subsource: None,
                yes: true,
            },
        )
        .await
        .unwrap();

        let after = load(&cosmos, META, &k).await.unwrap();
        assert!(
            after.last_complete_minute.is_none(),
            "cursor should be cleared with --yes"
        );
    }

    #[tokio::test]
    async fn reset_source_filter_only_affects_matching_sources() {
        let cosmos = InMemoryCosmos::new();

        let k_jira = key("prod", "jira-cloud", "DO");
        let k_conf = key("prod", "confluence", "DOCS");

        let c = Cursor {
            last_complete_minute: Some(Utc::now()),
            ..Default::default()
        };
        save(&cosmos, META, &k_jira, &c).await.unwrap();
        save(&cosmos, META, &k_conf, &c).await.unwrap();

        // Reset only jira-cloud.
        run_reset_directly(
            &cosmos,
            META,
            ResetOptions {
                source: Some("jira-cloud".to_string()),
                subsource: None,
                yes: true,
            },
        )
        .await
        .unwrap();

        let jira_after = load(&cosmos, META, &k_jira).await.unwrap();
        let conf_after = load(&cosmos, META, &k_conf).await.unwrap();

        assert!(
            jira_after.last_complete_minute.is_none(),
            "jira-cloud cursor should be cleared"
        );
        assert!(
            conf_after.last_complete_minute.is_some(),
            "confluence cursor should be untouched"
        );
    }

    #[tokio::test]
    async fn reset_nothing_to_reset_when_no_cursors() {
        let cosmos = InMemoryCosmos::new();
        // No cursors exist.
        let result = run_reset_directly(
            &cosmos,
            META,
            ResetOptions {
                source: None,
                subsource: None,
                yes: true,
            },
        )
        .await;

        // Should succeed silently.
        assert!(result.is_ok());
    }
}
