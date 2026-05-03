//! Ingest worker: builds connectors from config, drives the cycle engine.
//!
//! # Entry points
//!
//! - [`run`] — production entry point; builds everything from a [`Config`].
//! - [`run_with`] — test-friendly entry point; accepts pre-built connectors
//!   and a Cosmos backend.

use anyhow::Context as _;
use tracing::{error, info};

use crate::config::{Config, DeploymentRole, SourceConfig};
use crate::cosmos::CosmosBackend;
use crate::cosmos::meta::CursorKey;
use crate::ingest::config::CycleConfig;
use crate::ingest::connector_kind::AnyConnector;
use crate::ingest::rate_limit::build_rate_limited_client;
use crate::ingest::{cycle, reconcile};
use crate::sources::SourceConnector;

/// Options forwarded to the worker loop.
#[derive(Debug, Clone, Default)]
pub struct WorkerOptions {
    /// Run exactly one cycle for each connector, then exit.
    pub once: bool,
    /// TODO: stop after ingesting this many documents (debugging aid; not yet wired).
    pub max_docs: Option<u64>,
}

// ---------------------------------------------------------------------------
// Production entry point
// ---------------------------------------------------------------------------

/// Run the ingest worker for `deployment_name`.
///
/// Loads sources from `config`, validates that the deployment is an ingest
/// role, builds rate-limited connectors, then drives the cycle engine.
///
/// # Errors
///
/// Returns an error if:
/// - `deployment_name` is not found in `config.deployments`.
/// - The deployment role is not [`DeploymentRole::Ingest`].
/// - The Cosmos backend cannot be initialised.
pub async fn run(
    config: &Config,
    deployment_name: &str,
    options: WorkerOptions,
) -> anyhow::Result<()> {
    let sliced = crate::config::slice::for_deployment(config, deployment_name)?;

    // Validate deployment role.
    let dep = sliced
        .deployments
        .first()
        .expect("slice::for_deployment guarantees exactly one deployment");
    anyhow::ensure!(
        dep.role == DeploymentRole::Ingest,
        "worker only supports ingest deployments, but '{}' has role {:?}",
        deployment_name,
        dep.role,
    );

    let cycle_cfg = CycleConfig::from_config(&sliced, deployment_name);
    let cosmos = build_cosmos_backend(&sliced).await?;
    let connectors = build_connectors(&sliced)?;

    run_with(connectors, cosmos, cycle_cfg, options).await
}

// ---------------------------------------------------------------------------
// Test-friendly entry point
// ---------------------------------------------------------------------------

/// Run the worker loop with pre-built connectors and a Cosmos backend.
///
/// This is the testable core of the worker; [`run`] is just an adapter that
/// builds these inputs from config.
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

            // Run reconciliation every N cycles.
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

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Build one `(CursorKey, AnyConnector)` row per project/space.
fn build_connectors(config: &Config) -> anyhow::Result<Vec<(CursorKey, AnyConnector)>> {
    let http = build_rate_limited_client(reqwest::Client::new(), config.ingest.max_retries);

    // The sliced config contains exactly one deployment.
    let dep = config
        .deployments
        .first()
        .expect("slice::for_deployment guarantees exactly one deployment");

    let mut out: Vec<(CursorKey, AnyConnector)> = Vec::new();

    for src_config in &config.sources {
        match src_config {
            SourceConfig::Jira(j) => {
                let connector = crate::sources::jira::JiraConnector::new(j, http.clone())
                    .with_context(|| format!("build JiraConnector for '{}'", j.name))?;
                for project in &j.projects {
                    out.push((
                        CursorKey {
                            deployment_name: dep.name.clone(),
                            source_name: j.name.clone(),
                            subsource: project.clone(),
                        },
                        AnyConnector::Jira(connector.clone()),
                    ));
                }
            }
            SourceConfig::Confluence(c) => {
                let connector =
                    crate::sources::confluence::ConfluenceConnector::new(c, http.clone())
                        .with_context(|| format!("build ConfluenceConnector for '{}'", c.name))?;
                for space in &c.spaces {
                    out.push((
                        CursorKey {
                            deployment_name: dep.name.clone(),
                            source_name: c.name.clone(),
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

/// Construct a `CosmosBackend` from the config's `state.backend` setting.
async fn build_cosmos_backend(config: &Config) -> anyhow::Result<Box<dyn CosmosBackend>> {
    use crate::config::StateBackend;

    match &config.state.backend {
        StateBackend::Cosmos => {
            // `cosmos.account` may be just the account name (e.g. "quelch-prod") or a
            // full endpoint URL.  We normalise to the full endpoint form expected by
            // CosmosClient::new.
            let account = config.cosmos.account.as_deref().ok_or_else(|| {
                anyhow::anyhow!("cosmos.account is required for state.backend=cosmos")
            })?;

            let endpoint = if account.starts_with("https://") {
                account.to_owned()
            } else {
                // TODO(phase 5): revisit when Bicep deploy lands and confirms the
                // exact format returned by the naming module.
                format!("https://{account}.documents.azure.com:443/")
            };

            let client =
                crate::cosmos::CosmosClient::new(&endpoint, &config.cosmos.database).await?;
            Ok(Box::new(client))
        }
        StateBackend::LocalFile => {
            anyhow::bail!("local-file state backend is not supported in ingest mode")
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosmos::InMemoryCosmos;
    use crate::ingest::connector_kind::AnyConnector;
    use crate::ingest::test_helpers::{MockConnector, make_source_doc};

    fn test_key(subsource: &str) -> CursorKey {
        CursorKey {
            deployment_name: "test".into(),
            source_name: "test-source".into(),
            subsource: subsource.into(),
        }
    }

    #[tokio::test]
    async fn worker_runs_one_cycle_and_exits_when_once_set() {
        let cosmos: Box<dyn CosmosBackend> = Box::new(InMemoryCosmos::new());

        let mock = MockConnector::new("test-source", "jira-issues");
        // One doc, no more pages.
        mock.push_window_page(vec![make_source_doc("DO-1", "DO")], None);

        let connectors: Vec<(CursorKey, AnyConnector)> =
            vec![(test_key("DO"), AnyConnector::Mock(mock))];

        let cfg = CycleConfig {
            // Force a non-zero reconcile_every so reconcile doesn't run on cycle 1.
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
            reconcile_every: 100, // don't trigger reconcile on first cycle
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
}
