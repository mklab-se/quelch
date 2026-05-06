use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tracing::info;

use crate::config::{
    AiChat, AiConfig, AiEmbedding, AiProvider, AzureConfig, Config, ContainerLayout, CosmosConfig,
    IngestInstance, InstanceConfig, InstanceSpec, McpInstance, SearchConfig, SourceAuth,
    SourceConnection, SourceType,
};
use crate::cosmos::{CosmosBackend, InMemoryCosmos};
use crate::ingest::config::CycleConfig;
use crate::ingest::connector_kind::AnyConnector;
use crate::ingest::worker::{WorkerOptions, run_with};

#[derive(Debug)]
pub struct DevOptions {
    pub use_real_search: bool,
    pub use_cosmos_emulator: bool,
    pub mcp_port: u16,
    pub seed: Option<u64>,
    pub rate_multiplier: f64,
    pub no_tui: bool,
    pub once: bool,
}

impl Default for DevOptions {
    fn default() -> Self {
        Self {
            use_real_search: false,
            use_cosmos_emulator: false,
            mcp_port: 8080,
            seed: None,
            rate_multiplier: 1.0,
            no_tui: false,
            once: false,
        }
    }
}

pub async fn run(options: DevOptions) -> Result<()> {
    let cancel = tokio_util::sync::CancellationToken::new();

    let mock_base_url = start_mock_server(cancel.clone()).await?;
    let mock_jira_url = format!("{mock_base_url}/jira");
    let mock_confluence_url = format!("{mock_base_url}/confluence");

    info!(%mock_jira_url, %mock_confluence_url, "dev mock servers started");

    let cosmos = if options.use_cosmos_emulator {
        None
    } else {
        Some(InMemoryCosmos::new())
    };

    let (mcp_cosmos, ingest_cosmos_box): (Arc<dyn CosmosBackend>, Box<dyn CosmosBackend>) =
        if options.use_cosmos_emulator {
            let endpoint = "https://localhost:8081".to_string();
            let client = crate::cosmos::CosmosClient::new(&endpoint, "quelch").await?;
            let arc: Arc<dyn CosmosBackend> = Arc::new(client);
            let client2 = crate::cosmos::CosmosClient::new(&endpoint, "quelch").await?;
            (arc, Box::new(client2))
        } else {
            let mem = cosmos.unwrap();
            let arc: Arc<dyn CosmosBackend> = Arc::new(mem.clone());
            let boxed: Box<dyn CosmosBackend> = Box::new(mem);
            (arc, boxed)
        };

    let config = build_dev_config(&mock_jira_url, &mock_confluence_url, options.mcp_port);

    let connectors = build_dev_connectors(&config)?;
    let cycle_cfg = CycleConfig::from_config(&config, "dev-ingest");

    let worker_options = WorkerOptions {
        once: options.once,
        max_docs: None,
    };
    let ingest_handle = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            tokio::select! {
                res = run_with(connectors, ingest_cosmos_box, cycle_cfg, worker_options) => {
                    if let Err(e) = res {
                        tracing::error!(error = %e, "ingest worker exited with error");
                    }
                }
                _ = cancel.cancelled() => {
                    info!("ingest worker cancelled");
                }
            }
        }
    });

    let mcp_port = options.mcp_port;
    let mcp_cosmos_clone = mcp_cosmos.clone();
    let mcp_config = config.clone();
    let mcp_handle = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            let bind_addr = format!("0.0.0.0:{mcp_port}");
            tokio::select! {
                res = crate::mcp::run_server_in_memory(
                    &mcp_config, "dev-mcp", &bind_addr, mcp_cosmos_clone
                ) => {
                    if let Err(e) = res {
                        tracing::error!(error = %e, "MCP server exited with error");
                    }
                }
                _ = cancel.cancelled() => {
                    info!("MCP server cancelled");
                }
            }
        }
    });

    if options.once {
        let _ = ingest_handle.await;
        tokio::time::sleep(Duration::from_millis(200)).await;
    } else if options.no_tui {
        info!(
            mcp_port,
            "quelch dev running (no TUI) — press Ctrl-C to stop"
        );
        tokio::signal::ctrl_c().await?;
    } else {
        crate::tui::run_status_dashboard(mcp_cosmos, "quelch-meta".into(), Duration::from_secs(2))
            .await?;
    }

    cancel.cancel();
    let _ = mcp_handle.await;

    Ok(())
}

async fn start_mock_server(cancel: tokio_util::sync::CancellationToken) -> Result<String> {
    use std::net::SocketAddr;

    let addr = SocketAddr::from(([127, 0, 0, 1], 0));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;
    let url = format!("http://{bound_addr}");

    let app = crate::mock::build_router();

    tokio::spawn(async move {
        tokio::select! {
            res = axum::serve(listener, app) => {
                if let Err(e) = res {
                    tracing::error!(error = %e, "mock server exited with error");
                }
            }
            _ = cancel.cancelled() => {
                tracing::debug!("mock server shutdown");
            }
        }
    });

    Ok(url)
}

fn build_dev_config(mock_jira_url: &str, mock_confluence_url: &str, _mcp_port: u16) -> Config {
    Config {
        azure: AzureConfig {
            cosmos: CosmosConfig {
                subscription_id: None,
                resource_group: None,
                account: None,
                endpoint: "https://dev-cosmos.example".into(),
                database: "quelch".into(),
                containers: ContainerLayout::default(),
                meta_container: "quelch-meta".into(),
            },
            search: Some(SearchConfig {
                endpoint: "https://dev-search.example".into(),
            }),
            ai: Some(AiConfig {
                provider: AiProvider::AzureOpenai,
                endpoint: "https://dev.openai.azure.com".into(),
                embedding: AiEmbedding {
                    deployment: "dev-te".into(),
                    dimensions: 1536,
                },
                chat: AiChat {
                    deployment: "gpt-5-mini".into(),
                    model_name: "gpt-5-mini".into(),
                },
            }),
        },
        source_connections: vec![
            SourceConnection {
                name: "dev-jira".into(),
                source_type: SourceType::Jira,
                base_url: mock_jira_url.into(),
                auth: SourceAuth::Pat {
                    token: crate::mock::MOCK_TOKEN.into(),
                },
                projects: vec!["QUELCH".into(), "DEMO".into()],
                spaces: Vec::new(),
            },
            SourceConnection {
                name: "dev-confluence".into(),
                source_type: SourceType::Confluence,
                base_url: mock_confluence_url.into(),
                auth: SourceAuth::Pat {
                    token: crate::mock::MOCK_TOKEN.into(),
                },
                projects: Vec::new(),
                spaces: vec!["QUELCH".into(), "INFRA".into()],
            },
        ],
        instances: vec![
            InstanceConfig {
                name: "dev-ingest".into(),
                spec: InstanceSpec::Ingest(IngestInstance {
                    connections: vec!["dev-jira".into(), "dev-confluence".into()],
                    cycle_interval: Duration::from_secs(10),
                }),
            },
            InstanceConfig {
                name: "dev-mcp".into(),
                spec: InstanceSpec::Mcp(McpInstance {
                    expose: vec![
                        "jira_issues".into(),
                        "confluence_pages".into(),
                        "jira_sprints".into(),
                        "confluence_spaces".into(),
                    ],
                    api_key: "dev".into(),
                    knowledge_base: "dev-kb".into(),
                    listen: "0.0.0.0:8080".into(),
                }),
            },
        ],
    }
}

fn build_dev_connectors(
    config: &Config,
) -> Result<Vec<(crate::cosmos::meta::CursorKey, AnyConnector)>> {
    use crate::cosmos::meta::CursorKey;
    use crate::ingest::rate_limit::build_rate_limited_client;

    let sliced = crate::config::slice::slice_for_instance(config, "dev-ingest")?;
    let http = build_rate_limited_client(reqwest::Client::new(), 5);
    let dep = sliced.instances.first().expect("slice guarantees one inst");
    let mut out: Vec<(CursorKey, AnyConnector)> = Vec::new();

    for conn in &sliced.source_connections {
        match conn.source_type {
            SourceType::Jira => {
                let connector = crate::sources::jira::JiraConnector::new(conn, http.clone())
                    .map_err(|e| anyhow::anyhow!("build JiraConnector '{}': {e}", conn.name))?;
                for project in &conn.projects {
                    out.push((
                        CursorKey {
                            deployment_name: dep.name.clone(),
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
                            deployment_name: dep.name.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::InstanceKind;

    #[test]
    fn build_dev_config_has_expected_instances() {
        let cfg = build_dev_config(
            "http://127.0.0.1:9999/jira",
            "http://127.0.0.1:9999/confluence",
            8080,
        );
        assert_eq!(cfg.instances.len(), 2);
        assert_eq!(cfg.instances[0].name, "dev-ingest");
        assert_eq!(cfg.instances[0].kind(), InstanceKind::Ingest);
        assert_eq!(cfg.instances[1].name, "dev-mcp");
        assert_eq!(cfg.instances[1].kind(), InstanceKind::Mcp);
        assert_eq!(cfg.source_connections.len(), 2);
    }

    #[test]
    fn build_dev_config_has_expected_sources() {
        let cfg = build_dev_config(
            "http://127.0.0.1:9999/jira",
            "http://127.0.0.1:9999/confluence",
            8080,
        );
        let names: Vec<&str> = cfg
            .source_connections
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert!(names.contains(&"dev-jira"));
        assert!(names.contains(&"dev-confluence"));
    }
}
