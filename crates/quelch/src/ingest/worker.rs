use tracing::{error, info};

use crate::config::Config;
use crate::cosmos::CosmosBackend;
use crate::cosmos::meta::CursorKey;
use crate::ingest::config::CycleConfig;
use crate::ingest::{cycle, reconcile};
use crate::sources::SourceConnector;

#[derive(Debug, Clone, Default)]
pub struct WorkerOptions {
    pub once: bool,
    pub max_docs: Option<u64>,
}

pub async fn run(
    _config: &Config,
    _instance_name: &str,
    _options: WorkerOptions,
) -> anyhow::Result<()> {
    todo!("phase 7: rewire ingest worker against the new instances/source_connections schema")
}

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
    use crate::ingest::connector_kind::AnyConnector;
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
}
