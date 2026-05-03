// TODO(quelch v2 phase 3+): re-enable v1 commands as they are replaced by v2 equivalents.
//
// The v1 CLI commands (sync, watch, setup, reset-indexes, status, search, sim,
// generate-agent) are stubbed for the v2 config layer work (Phase 1).
// Each will be replaced by v2 commands in Phases 3–8.

mod cli;

use anyhow::Result;
use clap::Parser;
use quelch::config;
use std::path::Path;
use tracing_subscriber::EnvFilter;

use cli::{Cli, Commands};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("quelch=info"))
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Validate => cmd_validate(&cli.config),
        Commands::EffectiveConfig { name } => cmd_effective_config(&cli.config, &name),
        Commands::Init => cmd_init(),
        Commands::Mock { port } => quelch::mock::run_mock_server(port).await,
        Commands::Ai { command } => quelch::ai::run(command).await,
        // TODO(quelch v2 phase 3+): wire up remaining commands
        Commands::Sync { .. } => {
            anyhow::bail!("quelch sync is not available in v2; use `quelch ingest` (Phase 3)")
        }
        Commands::Watch { .. } => {
            anyhow::bail!("quelch watch is not available in v2; use `quelch ingest` (Phase 3)")
        }
        Commands::Setup { .. } => {
            anyhow::bail!(
                "quelch setup is not available in v2; use `quelch azure deploy` (Phase 4)"
            )
        }
        Commands::Status => {
            anyhow::bail!(
                "quelch status is not available in v2; use `quelch status --tui` (Phase 8)"
            )
        }
        Commands::Reset { .. } => {
            anyhow::bail!(
                "quelch reset is not available in v2; use `quelch azure indexer reset` (Phase 4)"
            )
        }
        Commands::ResetIndexes => {
            anyhow::bail!(
                "quelch reset-indexes is not available in v2; use `quelch azure indexer reset` (Phase 4)"
            )
        }
        Commands::Search { .. } => {
            anyhow::bail!("quelch search is not available in v2; use `quelch mcp` (Phase 5)")
        }
        Commands::Sim { .. } => {
            anyhow::bail!("quelch sim is not available in v2; use `quelch dev` (Phase 3/4)")
        }
        Commands::GenerateAgent { .. } => {
            anyhow::bail!(
                "quelch generate-agent is not available in v2; use `quelch agent generate` (Phase 7)"
            )
        }
        Commands::Ingest {
            deployment,
            once,
            max_docs,
        } => {
            let config = quelch::config::load_config(&cli.config)?;
            quelch::ingest::worker::run(
                &config,
                &deployment,
                quelch::ingest::worker::WorkerOptions { once, max_docs },
            )
            .await
        }
    }
}

fn cmd_validate(config_path: &Path) -> Result<()> {
    let config = config::load_config(config_path)?;
    println!("Config is valid.");
    println!("  Azure subscription: {}", config.azure.subscription_id);
    println!("  Resource group:     {}", config.azure.resource_group);
    println!("  Region:             {}", config.azure.region);
    println!("  Sources:            {}", config.sources.len());
    for source in &config.sources {
        println!("    - {}", source.name());
    }
    println!("  Deployments:        {}", config.deployments.len());
    for deployment in &config.deployments {
        println!("    - {}", deployment.name);
    }
    Ok(())
}

fn cmd_effective_config(config_path: &Path, name: &str) -> Result<()> {
    let config = config::load_config(config_path)?;
    let sliced = config::slice::for_deployment(&config, name)?;
    let yaml = serde_yaml::to_string(&sliced)?;
    print!("{yaml}");
    Ok(())
}

fn cmd_init() -> Result<()> {
    let path = Path::new("quelch.yaml");
    if path.exists() {
        anyhow::bail!("quelch.yaml already exists — remove it first or edit it directly");
    }

    let template = r#"# quelch.yaml — v2 configuration

azure:
  subscription_id: "${AZURE_SUBSCRIPTION_ID}"
  resource_group: "rg-quelch-prod"
  region: "swedencentral"
  naming:
    prefix: "quelch"
    environment: "prod"

cosmos:
  database: "quelch"
  throughput:
    mode: "serverless"

search:
  sku: "basic"

openai:
  endpoint: "https://${AOI_ACCOUNT}.openai.azure.com"
  embedding_deployment: "text-embedding-3-large"
  embedding_dimensions: 3072

sources:
  - type: jira
    name: my-jira
    url: "https://your-company.atlassian.net"
    auth:
      email: "${JIRA_EMAIL}"
      api_token: "${JIRA_API_TOKEN}"
    projects: ["PROJ"]

deployments:
  - name: ingest
    role: ingest
    target: azure
    azure:
      container_app: { cpu: 0.5, memory: "1.0Gi" }
    sources:
      - source: my-jira
  - name: mcp
    role: mcp
    target: azure
    azure:
      container_app: { cpu: 1.0, memory: "2.0Gi", min_replicas: 0 }
    expose:
      - jira_issues
    auth:
      mode: "api_key"

mcp:
  data_sources:
    jira_issues:
      kind: jira_issue
      backed_by:
        - container: jira-issues
"#;

    std::fs::write(path, template)?;
    println!("Created quelch.yaml — edit it with your Azure and source credentials");
    Ok(())
}

#[cfg(test)]
mod decide_mode_tests {
    use super::*;

    #[test]
    fn cli_parses() {
        // Verify the CLI can be constructed — smoke test only.
        let cli = Cli::parse_from(["quelch", "validate"]);
        assert!(matches!(cli.command, Commands::Validate));
    }
}
