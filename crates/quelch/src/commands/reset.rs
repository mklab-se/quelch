//! `quelch reset` — clear cursor state for a single `(source, subsource)` pair.
//!
//! Resets only cursors that this instance already owns. To rewrite a cursor
//! that another instance currently claims (transfer ownership), pass
//! `--take-ownership`.

use crate::config::Config;
use crate::cosmos::meta::{Cursor, CursorKey};
use crate::cosmos::{CosmosBackend, factory::build_cosmos_backend, meta};

/// Options for `quelch reset`.
#[derive(Debug)]
pub struct ResetOptions {
    /// Name of the Q-Ingest instance issuing the reset.
    pub instance: String,
    /// Source connection name (matches `quelch.yaml`).
    pub source: String,
    /// Subsource (project or space key).
    pub subsource: String,
    /// Rewrite the cursor's `owner_instance` to `instance` even if a different
    /// instance currently owns it.
    pub take_ownership: bool,
    /// Skip the interactive confirmation prompt.
    pub yes: bool,
}

/// Run `quelch reset` against the configured Cosmos account.
pub async fn run(config: &Config, options: ResetOptions) -> anyhow::Result<()> {
    let cosmos = build_cosmos_backend(config).await?;
    let key = CursorKey {
        source_name: options.source.clone(),
        subsource: options.subsource.clone(),
    };

    let existing =
        meta::try_load(cosmos.as_ref(), &config.azure.cosmos.meta_container, &key).await?;

    if let Some(c) = &existing
        && let Some(owner) = &c.owner_instance
        && owner != &options.instance
        && !options.take_ownership
    {
        anyhow::bail!(
            "cursor {}::{} owned by '{}', refusing to reset from '{}'. \
             Pass --take-ownership to override.",
            key.source_name,
            key.subsource,
            owner,
            options.instance,
        );
    }

    if !options.yes {
        let prompt = format!(
            "Reset cursor {}::{} for instance '{}'?",
            key.source_name, key.subsource, options.instance
        );
        let confirmed = inquire::Confirm::new(&prompt)
            .with_default(false)
            .prompt()?;
        if !confirmed {
            println!("Aborted.");
            return Ok(());
        }
    }

    reset_cursor(
        cosmos.as_ref(),
        &config.azure.cosmos.meta_container,
        &options.instance,
        &key,
    )
    .await?;

    println!(
        "reset cursor {}::{} (owner: {})",
        key.source_name, key.subsource, options.instance
    );
    Ok(())
}

/// Internal helper that performs the cursor write.
///
/// The wrapper is reused by tests that bring their own `CosmosBackend` and
/// don't want to construct a full [`Config`].
async fn reset_cursor(
    backend: &dyn CosmosBackend,
    meta_container: &str,
    instance: &str,
    key: &CursorKey,
) -> anyhow::Result<()> {
    let cleared = Cursor {
        owner_instance: Some(instance.to_string()),
        ..Default::default()
    };
    meta::save(backend, meta_container, key, &cleared).await?;
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

    const META: &str = "quelch-meta";

    fn key(source: &str, subsource: &str) -> CursorKey {
        CursorKey {
            source_name: source.to_string(),
            subsource: subsource.to_string(),
        }
    }

    /// Test-only entry point that mirrors `run` but takes a backend and an
    /// already-resolved meta container — it skips config loading and the
    /// interactive prompt so it can be exercised in unit tests.
    async fn reset(
        backend: &dyn CosmosBackend,
        instance: &str,
        source: &str,
        subsource: &str,
        take_ownership: bool,
    ) -> anyhow::Result<()> {
        let k = CursorKey {
            source_name: source.into(),
            subsource: subsource.into(),
        };
        let existing = meta::try_load(backend, META, &k).await?;
        if let Some(c) = &existing
            && let Some(owner) = &c.owner_instance
            && owner != instance
            && !take_ownership
        {
            anyhow::bail!(
                "cursor owned by '{}', refusing to reset from '{}'. \
                 Pass --take-ownership to override.",
                owner,
                instance,
            );
        }
        reset_cursor(backend, META, instance, &k).await
    }

    #[tokio::test]
    async fn reset_clears_cursor_for_subsource() {
        let cosmos = InMemoryCosmos::new();

        let k = key("jira-cloud", "DO");
        let c = Cursor {
            owner_instance: Some("ingest-internal".into()),
            documents_synced_total: 500,
            backfill_in_progress: true,
            ..Default::default()
        };
        save(&cosmos, META, &k, &c).await.unwrap();

        // Reset.
        reset(&cosmos, "ingest-internal", "jira-cloud", "DO", false)
            .await
            .unwrap();

        let after = load(&cosmos, META, &k).await.unwrap();
        assert_eq!(
            after.documents_synced_total, 0,
            "documents_synced_total should be cleared"
        );
        assert!(!after.backfill_in_progress);
        assert_eq!(after.owner_instance.as_deref(), Some("ingest-internal"));
    }

    #[tokio::test]
    async fn reset_without_take_ownership_refuses_other_instance_cursor() {
        let backend = InMemoryCosmos::new();
        save(
            &backend,
            META,
            &CursorKey {
                source_name: "jira-x".into(),
                subsource: "DO".into(),
            },
            &Cursor {
                owner_instance: Some("instance-a".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let err = reset(&backend, "instance-b", "jira-x", "DO", false)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("instance-a"), "missing 'instance-a': {msg}");
        assert!(
            msg.contains("--take-ownership"),
            "missing --take-ownership hint: {msg}"
        );
    }

    #[tokio::test]
    async fn reset_with_take_ownership_rewrites_owner() {
        let backend = InMemoryCosmos::new();
        save(
            &backend,
            META,
            &CursorKey {
                source_name: "jira-x".into(),
                subsource: "DO".into(),
            },
            &Cursor {
                owner_instance: Some("instance-a".into()),
                documents_synced_total: 999,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        reset(&backend, "instance-b", "jira-x", "DO", true)
            .await
            .unwrap();

        let stored = load(
            &backend,
            META,
            &CursorKey {
                source_name: "jira-x".into(),
                subsource: "DO".into(),
            },
        )
        .await
        .unwrap();

        assert_eq!(stored.owner_instance.as_deref(), Some("instance-b"));
        assert_eq!(
            stored.documents_synced_total, 0,
            "reset clears progress after ownership transfer"
        );
    }

    #[tokio::test]
    async fn reset_creates_cursor_when_none_exists() {
        let backend = InMemoryCosmos::new();
        reset(&backend, "instance-a", "jira-x", "DO", false)
            .await
            .unwrap();

        let stored = load(
            &backend,
            META,
            &CursorKey {
                source_name: "jira-x".into(),
                subsource: "DO".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(stored.owner_instance.as_deref(), Some("instance-a"));
    }
}
