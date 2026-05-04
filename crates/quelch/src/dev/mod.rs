//! `quelch dev` — all-in-one local development mode.
//!
//! Spins up the following components in one process:
//! 1. A mock HTTP server exposing the Jira DC and Confluence DC REST APIs,
//!    populated with the built-in fixture dataset from `mock::data`.
//! 2. An ingest worker that reads from those mock servers and writes to an
//!    in-memory Cosmos backend.
//! 3. An MCP server backed by the same in-memory Cosmos backend.
//! 4. Optionally, the fleet-dashboard TUI (default: enabled).
//!
//! This lets a developer iterate on the full stack without any cloud accounts.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tracing::info;

use crate::config::schema::{
    AzureConfig, CompanionContainersConfig, DeploymentAuthConfig, NamingConfig,
};
use crate::config::{
    AiChatConfig, AiConfig, AiEmbeddingConfig, AiProvider, AuthConfig, Config, CosmosConfig,
    DeploymentConfig, DeploymentRole, DeploymentSource, DeploymentTarget, IngestConfig,
    JiraSourceConfig, McpAuthMode, McpConfig, OutputMode, ReasoningEffort, RiggConfig,
    SearchConfig, SourceConfig, StateConfig,
};
use crate::cosmos::{CosmosBackend, InMemoryCosmos};
use crate::ingest::config::CycleConfig;
use crate::ingest::connector_kind::AnyConnector;
use crate::ingest::worker::{WorkerOptions, run_with};

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// Options for `quelch dev`.
#[derive(Debug)]
pub struct DevOptions {
    /// Use the real Azure AI Search adapter (requires Azure credentials).
    /// Default: false (use no-op in-memory mock).
    pub use_real_search: bool,
    /// Use the Cosmos emulator at `https://localhost:8081` instead of in-memory.
    /// Default: false.
    pub use_cosmos_emulator: bool,
    /// Port for the embedded MCP server. Default: 8080.
    pub mcp_port: u16,
    /// Optional RNG seed for the fixture dataset (reserved for future use).
    pub seed: Option<u64>,
    /// Activity rate multiplier (reserved for future use).
    pub rate_multiplier: f64,
    /// Skip the TUI; emit structured logs to stdout instead.
    pub no_tui: bool,
    /// Run one ingest cycle and exit.  Useful for tests.
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

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run the all-in-one local development server.
///
/// # Shutdown
///
/// - Without `--no-tui`: exits when the user presses `q` / `Esc` in the TUI.
/// - With `--no-tui` and without `--once`: waits for Ctrl-C.
/// - With `--once`: runs exactly one ingest cycle then exits.
pub async fn run(options: DevOptions) -> Result<()> {
    let cancel = tokio_util::sync::CancellationToken::new();

    // 1. Start the mock Jira + Confluence server on a random port.
    let mock_base_url = start_mock_server(cancel.clone()).await?;
    let mock_jira_url = format!("{mock_base_url}/jira");
    let mock_confluence_url = format!("{mock_base_url}/confluence");

    info!(%mock_jira_url, %mock_confluence_url, "dev mock servers started");

    // 2. Build the in-memory Cosmos backend shared between ingest and MCP.
    //    `InMemoryCosmos` is `Clone`; all clones share the same `Arc<Mutex<...>>`.
    let cosmos = if options.use_cosmos_emulator {
        None // handled below
    } else {
        Some(InMemoryCosmos::new())
    };

    // Build the shared Arc<dyn CosmosBackend> for MCP / TUI, and a Box for ingest.
    let (mcp_cosmos, ingest_cosmos_box): (Arc<dyn CosmosBackend>, Box<dyn CosmosBackend>) =
        if options.use_cosmos_emulator {
            let endpoint = "https://localhost:8081".to_string();
            let client = crate::cosmos::CosmosClient::new(&endpoint, "quelch").await?;
            let arc: Arc<dyn CosmosBackend> = Arc::new(client);
            // For the emulator, ingest and MCP cannot easily share a single backend
            // without cloning.  Use a second connection for ingest.
            let client2 = crate::cosmos::CosmosClient::new(&endpoint, "quelch").await?;
            (arc, Box::new(client2))
        } else {
            let mem = cosmos.unwrap();
            let arc: Arc<dyn CosmosBackend> = Arc::new(mem.clone());
            let boxed: Box<dyn CosmosBackend> = Box::new(mem);
            (arc, boxed)
        };

    // 3. Build a synthetic config.
    let config = build_dev_config(&mock_jira_url, &mock_confluence_url, options.mcp_port);

    // 4. Build ingest connectors from the config.
    let connectors = build_dev_connectors(&config)?;
    let cycle_cfg = CycleConfig::from_config(&config, "dev-ingest");

    // 5. Spawn the ingest worker.
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

    // 6. Spawn the MCP server.
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

    // 7. Run the TUI, or wait for signal/once.
    if options.once {
        // Wait for the ingest worker to finish (one cycle).
        let _ = ingest_handle.await;
        // Give the MCP server a moment to accept connections before cancelling.
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

    // 8. Graceful shutdown.
    cancel.cancel();
    let _ = mcp_handle.await;

    Ok(())
}

// ---------------------------------------------------------------------------
// Mock server startup
// ---------------------------------------------------------------------------

/// Start the mock Jira/Confluence server on a random OS-assigned port.
///
/// Returns the base URL (e.g. `http://127.0.0.1:54321`).
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

// ---------------------------------------------------------------------------
// Synthetic config
// ---------------------------------------------------------------------------

fn build_dev_config(mock_jira_url: &str, mock_confluence_url: &str, _mcp_port: u16) -> Config {
    use crate::config::ConfluenceSourceConfig;

    Config {
        azure: AzureConfig {
            subscription_id: "dev-subscription".into(),
            resource_group: "dev-rg".into(),
            region: "swedencentral".into(),
            naming: NamingConfig::default(),
            skip_role_assignments: true,
            resources: crate::config::AzureExistingResources::default(),
        },
        cosmos: CosmosConfig {
            account: None,
            database: "quelch".into(),
            containers: Default::default(),
            meta_container: "quelch-meta".into(),
            throughput: Default::default(),
        },
        search: SearchConfig::default(),
        ai: AiConfig {
            provider: AiProvider::AzureOpenai,
            endpoint: "https://dev.openai.azure.com".into(),
            embedding: AiEmbeddingConfig {
                deployment: "dev-te".into(),
                dimensions: 1536,
            },
            chat: AiChatConfig {
                deployment: "gpt-5-mini".into(),
                model_name: "gpt-5-mini".into(),
                retrieval_reasoning_effort: ReasoningEffort::Low,
                output_mode: OutputMode::AnswerSynthesis,
            },
        },
        sources: vec![
            SourceConfig::Jira(JiraSourceConfig {
                name: "dev-jira".into(),
                url: mock_jira_url.into(),
                auth: AuthConfig::DataCenter {
                    pat: crate::mock::MOCK_TOKEN.into(),
                },
                projects: vec!["QUELCH".into(), "DEMO".into()],
                container: None,
                companion_containers: CompanionContainersConfig::default(),
                fields: Default::default(),
            }),
            SourceConfig::Confluence(ConfluenceSourceConfig {
                name: "dev-confluence".into(),
                url: mock_confluence_url.into(),
                auth: AuthConfig::DataCenter {
                    pat: crate::mock::MOCK_TOKEN.into(),
                },
                spaces: vec!["QUELCH".into(), "INFRA".into()],
                container: None,
                companion_containers: CompanionContainersConfig::default(),
            }),
        ],
        ingest: IngestConfig {
            poll_interval: "10s".into(),
            safety_lag_minutes: 0,
            ..IngestConfig::default()
        },
        deployments: vec![
            DeploymentConfig {
                name: "dev-ingest".into(),
                role: DeploymentRole::Ingest,
                target: DeploymentTarget::Onprem,
                sources: Some(vec![
                    DeploymentSource {
                        source: "dev-jira".into(),
                        projects: None,
                        spaces: None,
                    },
                    DeploymentSource {
                        source: "dev-confluence".into(),
                        projects: None,
                        spaces: None,
                    },
                ]),
                expose: None,
                azure: None,
                auth: None,
            },
            DeploymentConfig {
                name: "dev-mcp".into(),
                role: DeploymentRole::Mcp,
                target: DeploymentTarget::Onprem,
                sources: None,
                expose: Some(vec![
                    "jira_issues".into(),
                    "confluence_pages".into(),
                    "jira_sprints".into(),
                    "confluence_spaces".into(),
                ]),
                azure: None,
                auth: Some(DeploymentAuthConfig {
                    mode: McpAuthMode::ApiKey,
                }),
            },
        ],
        mcp: McpConfig::default(),
        rigg: RiggConfig::default(),
        state: StateConfig::default(),
    }
}

// ---------------------------------------------------------------------------
// Connector builder
// ---------------------------------------------------------------------------

fn build_dev_connectors(
    config: &Config,
) -> Result<Vec<(crate::cosmos::meta::CursorKey, AnyConnector)>> {
    use crate::cosmos::meta::CursorKey;
    use crate::ingest::rate_limit::build_rate_limited_client;

    let sliced = crate::config::slice::for_deployment(config, "dev-ingest")?;
    let http = build_rate_limited_client(reqwest::Client::new(), sliced.ingest.max_retries);
    let dep = sliced
        .deployments
        .first()
        .expect("slice guarantees one dep");
    let mut out: Vec<(CursorKey, AnyConnector)> = Vec::new();

    for src_config in &sliced.sources {
        match src_config {
            SourceConfig::Jira(j) => {
                let connector = crate::sources::jira::JiraConnector::new(j, http.clone())
                    .map_err(|e| anyhow::anyhow!("build JiraConnector '{}': {e}", j.name))?;
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
                    crate::sources::confluence::ConfluenceConnector::new(c, http.clone()).map_err(
                        |e| anyhow::anyhow!("build ConfluenceConnector '{}': {e}", c.name),
                    )?;
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_dev_config_has_expected_deployments() {
        let cfg = build_dev_config(
            "http://127.0.0.1:9999/jira",
            "http://127.0.0.1:9999/confluence",
            8080,
        );
        assert_eq!(cfg.deployments.len(), 2);
        assert_eq!(cfg.deployments[0].name, "dev-ingest");
        assert_eq!(cfg.deployments[0].role, DeploymentRole::Ingest);
        assert_eq!(cfg.deployments[1].name, "dev-mcp");
        assert_eq!(cfg.deployments[1].role, DeploymentRole::Mcp);
        assert_eq!(cfg.sources.len(), 2);
    }

    #[test]
    fn build_dev_config_has_expected_sources() {
        let cfg = build_dev_config(
            "http://127.0.0.1:9999/jira",
            "http://127.0.0.1:9999/confluence",
            8080,
        );
        let names: Vec<&str> = cfg.sources.iter().map(|s| s.name()).collect();
        assert!(names.contains(&"dev-jira"));
        assert!(names.contains(&"dev-confluence"));
    }

    /// End-to-end smoke test: spin up dev mode with `once = true`, verify the
    /// MCP server responds.
    ///
    /// Marked `#[ignore]` because it is timing-sensitive and requires port
    /// availability. Run manually with:
    ///   cargo test -p quelch dev_mode_e2e -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn dev_mode_e2e_once() {
        // Find a free port.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let options = DevOptions {
            mcp_port: port,
            no_tui: true,
            once: true,
            ..Default::default()
        };

        let handle = tokio::spawn(run(options));

        // Give dev a moment to start and run one cycle.
        tokio::time::sleep(Duration::from_secs(4)).await;

        let resp = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/mcp"))
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/list",
                "params": {}
            }))
            .send()
            .await
            .expect("MCP server must respond");
        assert_eq!(resp.status(), 200);

        handle
            .await
            .expect("dev run must complete")
            .expect("dev run must not error");
    }
}
