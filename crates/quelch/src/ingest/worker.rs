//! Q-Ingest worker: per-cycle loop that drives connector → cosmos transfers.
//!
//! [`run`] is the production entry wired to `quelch ingest --instance NAME`.
//! It slices the config to the named instance, builds connectors for every
//! `(source_connection, subsource)` tuple in scope, claims their cursors,
//! and then defers to [`run_with`] for the actual cycle loop.

use anyhow::Context;
use tracing::{error, info};

use crate::config::Config;
use crate::config::schema::{InstanceSpec, SourceType};
use crate::cosmos::CosmosBackend;
use crate::cosmos::meta::{CursorKey, save, try_load};
use crate::ingest::config::CycleConfig;
use crate::ingest::connector_kind::AnyConnector;
use crate::ingest::{cycle, reconcile};
use crate::sources::SourceConnector;

/// Tunables for one worker run.
#[derive(Debug, Clone, Default)]
pub struct WorkerOptions {
    /// Run a single cycle, then exit.
    pub once: bool,
    /// Reserved — currently unused (no in-process accounting yet).
    pub max_docs: Option<u64>,
}

/// Run the worker for the named ingest instance against the configured
/// Cosmos endpoint.
///
/// Errors out at startup if the instance is missing, is not an ingest
/// instance, or fails to claim a cursor that another instance owns.
pub async fn run(
    config: &Config,
    instance_name: &str,
    options: WorkerOptions,
) -> anyhow::Result<()> {
    let sliced = crate::config::slice::slice_for_instance(config, instance_name)?;
    // The slice always contains exactly one instance; verify it's an ingest one.
    let inst = sliced
        .instances
        .first()
        .ok_or_else(|| anyhow::anyhow!("slice for '{instance_name}' produced no instances"))?;
    let _ig = match &inst.spec {
        InstanceSpec::Ingest(i) => i,
        InstanceSpec::Mcp(_) => {
            anyhow::bail!(
                "instance '{instance_name}' is an mcp instance; quelch ingest requires kind=ingest"
            )
        }
    };

    let cosmos: Box<dyn CosmosBackend> = crate::cosmos::factory::build_cosmos_backend(&sliced)
        .await
        .context("build cosmos backend")?;

    let cfg = CycleConfig::from_config(&sliced, instance_name);
    let connectors = build_connectors(&sliced)?;

    // Claim every cursor we're about to write before the cycle loop starts.
    for (key, _) in &connectors {
        ensure_claim(cosmos.as_ref(), &cfg.meta_container, instance_name, key).await?;
    }

    run_with(connectors, cosmos, cfg, options).await
}

/// Build `(CursorKey, AnyConnector)` pairs for every `(source_connection,
/// subsource)` tuple in the sliced config.
fn build_connectors(sliced: &Config) -> anyhow::Result<Vec<(CursorKey, AnyConnector)>> {
    use crate::ingest::rate_limit::build_rate_limited_client;

    let http = build_rate_limited_client(reqwest::Client::new(), 5);
    let mut out: Vec<(CursorKey, AnyConnector)> = Vec::new();

    for conn in &sliced.source_connections {
        match conn.source_type {
            SourceType::Jira => {
                let connector = crate::sources::jira::JiraConnector::new(conn, http.clone())
                    .map_err(|e| anyhow::anyhow!("build JiraConnector '{}': {e}", conn.name))?;
                for project in &conn.projects {
                    out.push((
                        CursorKey {
                            source_name: conn.name.clone(),
                            subsource: project.clone(),
                        },
                        AnyConnector::Jira(connector.clone()),
                    ));
                }
            }
            SourceType::Confluence => {
                let connector =
                    crate::sources::confluence::ConfluenceConnector::new(conn, http.clone())
                        .map_err(|e| {
                            anyhow::anyhow!("build ConfluenceConnector '{}': {e}", conn.name)
                        })?;
                for space in &conn.spaces {
                    out.push((
                        CursorKey {
                            source_name: conn.name.clone(),
                            subsource: space.clone(),
                        },
                        AnyConnector::Confluence(connector.clone()),
                    ));
                }
            }
        }
    }

    Ok(out)
}

/// Ensure the named instance holds the cursor for `key`.
///
/// - Cursor missing or unowned: write it with `owner_instance = Some(self)`.
/// - Cursor owned by `self`: no-op.
/// - Cursor owned by a different instance: abort with a helpful error.
pub async fn ensure_claim(
    backend: &dyn CosmosBackend,
    meta_container: &str,
    instance_name: &str,
    key: &CursorKey,
) -> anyhow::Result<()> {
    let existing = try_load(backend, meta_container, key).await?;
    match existing {
        Some(c) if c.owner_instance.as_deref() == Some(instance_name) => Ok(()),
        Some(c) if c.owner_instance.is_some() => {
            let owner = c.owner_instance.unwrap_or_default();
            anyhow::bail!(
                "cursor {} is already owned by instance '{}'; this instance is '{}'. \
                 Run `quelch reset --instance {} --source {} --subsource {} --take-ownership` \
                 to transfer.",
                key.id(),
                owner,
                instance_name,
                instance_name,
                key.source_name,
                key.subsource,
            )
        }
        _ => {
            // Unowned (None) or absent — claim it.
            let mut c = existing.unwrap_or_default();
            c.owner_instance = Some(instance_name.to_string());
            save(backend, meta_container, key, &c).await?;
            Ok(())
        }
    }
}

/// Run the cycle loop with caller-provided connectors and cosmos backend.
///
/// Used both by the production [`run`] above and by `quelch dev`. Does not
/// itself enforce ownership — call `ensure_claim` for every key before
/// invoking this function.
pub async fn run_with<C>(
    connectors: Vec<(CursorKey, C)>,
    cosmos: Box<dyn CosmosBackend>,
    cfg: CycleConfig,
    options: WorkerOptions,
) -> anyhow::Result<()>
where
    C: SourceConnector,
{
    let mut cycle_n: u64 = 0;

    loop {
        cycle_n += 1;

        for (key, connector) in &connectors {
            let outcome = cycle::run(connector, cosmos.as_ref(), key, &cfg).await;
            info!(?outcome, key = %key.id(), "cycle complete");

            if cycle_n.is_multiple_of(cfg.reconcile_every) {
                match reconcile::run(connector, cosmos.as_ref(), key, &cfg).await {
                    Ok(deleted) => info!(deleted, key = %key.id(), "reconcile complete"),
                    Err(e) => error!(error = %e, key = %key.id(), "reconcile failed"),
                }
            }
        }

        if options.once {
            break;
        }

        tokio::time::sleep(cfg.poll_interval).await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosmos::InMemoryCosmos;
    use crate::cosmos::meta::Cursor;
    use crate::ingest::test_helpers::{MockConnector, make_source_doc};

    fn test_key(subsource: &str) -> CursorKey {
        CursorKey {
            source_name: "test-source".into(),
            subsource: subsource.into(),
        }
    }

    #[tokio::test]
    async fn worker_runs_one_cycle_and_exits_when_once_set() {
        let cosmos: Box<dyn CosmosBackend> = Box::new(InMemoryCosmos::new());

        let mock = MockConnector::new("test-source", "jira-issues");
        mock.push_window_page(vec![make_source_doc("DO-1", "DO")], None);

        let connectors: Vec<(CursorKey, AnyConnector)> =
            vec![(test_key("DO"), AnyConnector::Mock(mock))];

        let cfg = CycleConfig {
            reconcile_every: 12,
            ..CycleConfig::default()
        };
        let options = WorkerOptions {
            once: true,
            ..Default::default()
        };

        run_with(connectors, cosmos, cfg, options)
            .await
            .expect("worker should complete without error");
    }

    #[tokio::test]
    async fn worker_runs_multiple_connectors() {
        let cosmos: Box<dyn CosmosBackend> = Box::new(InMemoryCosmos::new());

        let mock_a = MockConnector::new("source-a", "jira-issues");
        mock_a.push_window_page(vec![make_source_doc("A-1", "A")], None);

        let mock_b = MockConnector::new("source-b", "confluence-pages");
        mock_b.push_window_page(vec![make_source_doc("B-1", "ENG")], None);

        let connectors: Vec<(CursorKey, AnyConnector)> = vec![
            (test_key("A"), AnyConnector::Mock(mock_a)),
            (test_key("B"), AnyConnector::Mock(mock_b)),
        ];

        let cfg = CycleConfig {
            reconcile_every: 100,
            ..CycleConfig::default()
        };
        let options = WorkerOptions {
            once: true,
            ..Default::default()
        };

        run_with(connectors, cosmos, cfg, options)
            .await
            .expect("worker with multiple connectors should complete");
    }

    #[tokio::test]
    async fn ingest_claims_unowned_cursor_on_first_run() {
        let backend = InMemoryCosmos::new();
        let key = CursorKey {
            source_name: "jira-x".into(),
            subsource: "DO".into(),
        };
        ensure_claim(&backend, "quelch-meta", "instance-b", &key)
            .await
            .unwrap();

        let stored = try_load(&backend, "quelch-meta", &key).await.unwrap();
        let cursor = stored.expect("cursor should have been written");
        assert_eq!(cursor.owner_instance.as_deref(), Some("instance-b"));
    }

    #[tokio::test]
    async fn ingest_refuses_when_cursor_owned_by_other_instance() {
        let backend = InMemoryCosmos::new();
        let key = CursorKey {
            source_name: "jira-x".into(),
            subsource: "DO".into(),
        };
        save(
            &backend,
            "quelch-meta",
            &key,
            &Cursor {
                owner_instance: Some("instance-a".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let err = ensure_claim(&backend, "quelch-meta", "instance-b", &key)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("instance-a"), "missing instance-a: {msg}");
        assert!(msg.contains("instance-b"), "missing instance-b: {msg}");
        assert!(msg.contains("DO"), "missing DO: {msg}");
        assert!(
            msg.contains("--take-ownership"),
            "missing --take-ownership hint: {msg}"
        );
    }

    #[tokio::test]
    async fn ensure_claim_is_idempotent_for_same_instance() {
        let backend = InMemoryCosmos::new();
        let key = CursorKey {
            source_name: "jira-x".into(),
            subsource: "DO".into(),
        };
        ensure_claim(&backend, "quelch-meta", "instance-a", &key)
            .await
            .unwrap();
        // Second call must succeed without error.
        ensure_claim(&backend, "quelch-meta", "instance-a", &key)
            .await
            .unwrap();
    }
}
