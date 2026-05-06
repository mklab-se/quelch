mod cli;

use anyhow::{Context, Result};
use clap::Parser;
use quelch::config;
use std::path::Path;
use tracing_subscriber::EnvFilter;

use cli::{
    AgentCommands, AgentTarget, AzureCommands, Cli, Commands, IndexerCommands, InstanceCommand,
};

// Suppress `set_var` deprecation on Rust 1.80+ (we only use it for env var
// passthrough at process startup, never in multi-threaded context).
#[allow(deprecated)]
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("quelch=info"))
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Validate => cmd_validate(&cli.config).await,
        Commands::Init {
            directory,
            non_interactive,
            from_template,
            force,
        } => {
            if !directory.exists() {
                std::fs::create_dir_all(&directory)
                    .with_context(|| format!("creating directory {}", directory.display()))?;
            }
            let path = directory.join("quelch.yaml");
            quelch::init::run(
                &path,
                quelch::init::InitOptions {
                    non_interactive,
                    from_template,
                    force,
                },
            )
            .await
        }
        Commands::Mock { port } => quelch::mock::run_mock_server(port).await,
        Commands::Ai { command } => quelch::ai::run(command).await,
        Commands::Status {
            instance,
            json,
            tui,
        } => {
            let config = quelch::config::load_config(&cli.config)?;
            quelch::commands::status::run(
                &config,
                quelch::commands::status::StatusOptions {
                    instance,
                    json,
                    tui,
                },
            )
            .await
        }
        Commands::Reset {
            instance,
            source,
            subsource,
            take_ownership,
            yes,
        } => {
            let config = quelch::config::load_config(&cli.config)?;
            quelch::commands::reset::run(
                &config,
                quelch::commands::reset::ResetOptions {
                    instance,
                    source,
                    subsource,
                    take_ownership,
                    yes,
                },
            )
            .await
        }
        Commands::Query {
            data_source,
            r#where,
            where_file,
            order_by,
            top,
            cursor,
            count_only,
            include_deleted,
            json,
            instance,
        } => {
            let config = quelch::config::load_config(&cli.config)?;
            let where_val = parse_where_arg(r#where.as_deref(), where_file.as_deref())?;
            let order_by_parsed = order_by
                .iter()
                .map(|s| quelch::commands::query::parse_order_by(s))
                .collect::<Result<Vec<_>>>()?;
            quelch::commands::query::run(
                &config,
                quelch::commands::query::QueryOptions {
                    data_source,
                    where_: where_val,
                    order_by: order_by_parsed,
                    top,
                    cursor,
                    count_only,
                    include_deleted,
                    json,
                    instance,
                },
            )
            .await
        }
        Commands::Search {
            query,
            data_sources,
            r#where,
            top,
            cursor,
            include_content,
            include_deleted,
            json,
            instance,
        } => {
            let config = quelch::config::load_config(&cli.config)?;
            let where_val = parse_where_arg(r#where.as_deref(), None)?;
            let data_sources_parsed = data_sources
                .as_deref()
                .map(|s| s.split(',').map(|p| p.trim().to_string()).collect());
            quelch::commands::search::run(
                &config,
                quelch::commands::search::SearchOptions {
                    query,
                    data_sources: data_sources_parsed,
                    where_: where_val,
                    top,
                    cursor,
                    include_content,
                    include_deleted,
                    json,
                    instance,
                },
            )
            .await
        }
        Commands::Get {
            id,
            data_source,
            include_deleted,
            json,
            instance,
        } => {
            let config = quelch::config::load_config(&cli.config)?;
            quelch::commands::get::run(
                &config,
                quelch::commands::get::GetOptions {
                    id,
                    data_source,
                    include_deleted,
                    json,
                    instance,
                },
            )
            .await
        }
        Commands::Agent {
            command:
                AgentCommands::Generate {
                    target,
                    format: _format,
                    output,
                    instance,
                    url,
                },
        } => cmd_agent_generate(&cli.config, target, output, instance, url),
        Commands::Ingest {
            instance,
            once,
            max_docs,
        } => {
            let config = quelch::config::load_config(&cli.config)?;
            let name = quelch::cli_helpers::resolve_instance(
                &config,
                instance.as_deref(),
                quelch::config::InstanceKind::Ingest,
            )?
            .to_string();
            quelch::ingest::worker::run(
                &config,
                &name,
                quelch::ingest::worker::WorkerOptions { once, max_docs },
            )
            .await
        }
        Commands::Dev {
            use_real_search,
            use_cosmos_emulator,
            mcp_port,
            seed,
            rate_multiplier,
        } => {
            quelch::dev::run(quelch::dev::DevOptions {
                use_real_search,
                use_cosmos_emulator,
                mcp_port,
                seed,
                rate_multiplier,
                no_tui: cli.no_tui,
                once: false,
            })
            .await
        }
        Commands::Mcp {
            instance,
            port,
            bind,
            api_key,
        } => {
            let config = quelch::config::load_config(&cli.config)?;
            if let Some(key) = api_key {
                // SAFETY: called once at process start before spawning async tasks.
                unsafe { std::env::set_var("QUELCH_MCP_API_KEY", key) };
            }
            let name = quelch::cli_helpers::resolve_instance(
                &config,
                instance.as_deref(),
                quelch::config::InstanceKind::Mcp,
            )?
            .to_string();
            quelch::mcp::run_server(&config, &name, &format!("{bind}:{port}")).await
        }
        Commands::Azure { command } => match command {
            AzureCommands::Plan => cmd_azure_plan(&cli.config).await,
            AzureCommands::Apply { yes } => cmd_azure_apply(&cli.config, yes).await,
            AzureCommands::Indexer { command } => cmd_azure_indexer(&cli.config, command).await,
        },
        Commands::Instance { command } => {
            let cfg = quelch::config::load_config(&cli.config)?;
            let stdout = std::io::stdout();
            let mut out = stdout.lock();
            match command {
                InstanceCommand::List => quelch::commands::instance::list(&cfg, &mut out),
                InstanceCommand::Config { name, kind, output } => {
                    quelch::commands::instance::config(
                        &cfg,
                        &name,
                        kind,
                        output.as_deref(),
                        &mut out,
                    )
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// quelch azure plan / apply
// ---------------------------------------------------------------------------

async fn cmd_azure_plan(config_path: &Path) -> Result<()> {
    let cfg = quelch::config::load_config(config_path)?;
    let cosmos_client = quelch::azure::build_cosmos_client(&cfg)?;
    let rigg_client = quelch::azure::build_rigg_client(&cfg)?;
    let plan = quelch::azure::plan::compute(&cfg, &cosmos_client, &rigg_client).await?;
    print!("{}", quelch::azure::plan::render(&plan));
    Ok(())
}

async fn cmd_azure_apply(config_path: &Path, yes: bool) -> Result<()> {
    let cfg = quelch::config::load_config(config_path)?;
    let cosmos_client = quelch::azure::build_cosmos_client(&cfg)?;
    let rigg_client = quelch::azure::build_rigg_client(&cfg)?;

    let plan = quelch::azure::plan::compute(&cfg, &cosmos_client, &rigg_client).await?;
    print!("{}", quelch::azure::plan::render(&plan));

    if !yes {
        let confirmed = inquire::Confirm::new("Apply these changes?")
            .with_default(false)
            .prompt()?;
        if !confirmed {
            println!("Aborted.");
            return Ok(());
        }
    }

    quelch::azure::apply::apply(&cfg, &cosmos_client, &rigg_client).await?;
    println!("done.");
    Ok(())
}

// ---------------------------------------------------------------------------
// CLI helpers
// ---------------------------------------------------------------------------

fn parse_where_arg(
    where_str: Option<&str>,
    where_file: Option<&Path>,
) -> Result<Option<serde_json::Value>> {
    if let Some(s) = where_str {
        let v: serde_json::Value = serde_json::from_str(s)
            .map_err(|e| anyhow::anyhow!("--where is not valid JSON: {e}"))?;
        return Ok(Some(v));
    }
    if let Some(p) = where_file {
        let s = std::fs::read_to_string(p)
            .map_err(|e| anyhow::anyhow!("cannot read --where-file '{}': {e}", p.display()))?;
        let v: serde_json::Value = serde_json::from_str(&s)
            .map_err(|e| anyhow::anyhow!("--where-file is not valid JSON: {e}"))?;
        return Ok(Some(v));
    }
    Ok(None)
}

async fn cmd_validate(config_path: &Path) -> Result<()> {
    let config = config::load_config(config_path)?;
    println!("Config is valid.");
    if let Some(sub) = &config.azure.cosmos.subscription_id {
        println!("  Azure subscription: {sub}");
    }
    if let Some(rg) = &config.azure.cosmos.resource_group {
        println!("  Resource group:     {rg}");
    }
    println!("  Source connections: {}", config.source_connections.len());
    for source in &config.source_connections {
        println!("    - {}", source.name);
    }
    println!("  Instances:          {}", config.instances.len());
    for instance in &config.instances {
        println!("    - {} ({:?})", instance.name, instance.kind());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// quelch azure indexer
// ---------------------------------------------------------------------------

async fn cmd_azure_indexer(config_path: &Path, command: IndexerCommands) -> Result<()> {
    let config = quelch::config::load_config(config_path)?;
    let endpoint = config
        .azure
        .search
        .as_ref()
        .map(|s| s.endpoint.as_str())
        .unwrap_or("https://quelch-prod-search.search.windows.net");
    let service = endpoint
        .trim_start_matches("https://")
        .split('.')
        .next()
        .unwrap_or("quelch-prod-search");

    match command {
        IndexerCommands::Run { name } => {
            quelch::azure::indexer::run(service, &name)?;
            println!("Triggered indexer run for '{name}'.");
        }
        IndexerCommands::Reset { name } => {
            quelch::azure::indexer::reset(service, &name)?;
            println!("Reset indexer '{name}' — full re-index will run on next schedule.");
        }
        IndexerCommands::Status => {
            let statuses = quelch::azure::indexer::status(service)?;
            if statuses.is_empty() {
                println!("No indexers found in service '{service}'.");
            } else {
                println!("{:<40} {:<20} LAST RUN AT", "NAME", "LAST RESULT");
                println!("{}", "-".repeat(80));
                for s in &statuses {
                    println!(
                        "{:<40} {:<20} {}",
                        s.name,
                        s.last_result.as_deref().unwrap_or("—"),
                        s.last_run_at
                            .map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                            .unwrap_or_else(|| "—".to_string()),
                    );
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// quelch agent generate
// ---------------------------------------------------------------------------

fn cmd_agent_generate(
    config_path: &Path,
    target: AgentTarget,
    output: std::path::PathBuf,
    instance: Option<String>,
    url: Option<String>,
) -> Result<()> {
    let config = quelch::config::load_config(config_path)?;

    let instance_name = quelch::cli_helpers::resolve_instance(
        &config,
        instance.as_deref(),
        quelch::config::InstanceKind::Mcp,
    )?
    .to_string();

    // Build the bundle.
    let mut bundle = quelch::agent::bundle::build(&config, &instance_name)?;

    // Allow URL override.
    if let Some(explicit_url) = url {
        bundle.connection.url = explicit_url;
    }

    // Dispatch to the appropriate target writer.
    match target {
        AgentTarget::CopilotStudio => {
            quelch::agent::targets::copilot_studio::write(&bundle, &output)?;
        }
        AgentTarget::ClaudeCode => {
            quelch::agent::targets::claude_code::write(&bundle, &output)?;
        }
        AgentTarget::CopilotCli => {
            quelch::agent::targets::copilot_cli::write(&bundle, &output)?;
        }
        AgentTarget::VscodeCopilot => {
            quelch::agent::targets::vscode_copilot::write(&bundle, &output)?;
        }
        AgentTarget::Codex => {
            quelch::agent::targets::codex::write(&bundle, &output)?;
        }
        AgentTarget::Markdown => {
            quelch::agent::targets::markdown::write(&bundle, &output)?;
        }
    }

    println!("Wrote agent bundle to {}", output.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod decide_mode_tests {
    use super::*;

    #[test]
    fn cli_parses() {
        let cli = Cli::parse_from(["quelch", "validate"]);
        assert!(matches!(cli.command, Commands::Validate));
    }

    #[test]
    fn cli_parses_azure_plan() {
        let cli = Cli::parse_from(["quelch", "azure", "plan"]);
        assert!(matches!(
            cli.command,
            Commands::Azure {
                command: AzureCommands::Plan
            }
        ));
    }

    #[test]
    fn cli_parses_azure_apply() {
        let cli = Cli::parse_from(["quelch", "azure", "apply"]);
        assert!(matches!(
            cli.command,
            Commands::Azure {
                command: AzureCommands::Apply { yes: false }
            }
        ));
    }

    #[test]
    fn cli_parses_azure_apply_yes() {
        let cli = Cli::parse_from(["quelch", "azure", "apply", "--yes"]);
        assert!(matches!(
            cli.command,
            Commands::Azure {
                command: AzureCommands::Apply { yes: true }
            }
        ));
    }

    #[test]
    fn cli_parses_azure_indexer_status() {
        let cli = Cli::parse_from(["quelch", "azure", "indexer", "status"]);
        assert!(matches!(
            cli.command,
            Commands::Azure {
                command: AzureCommands::Indexer {
                    command: IndexerCommands::Status
                }
            }
        ));
    }

    #[test]
    fn cli_parses_azure_indexer_run() {
        let cli = Cli::parse_from(["quelch", "azure", "indexer", "run", "jira-issues"]);
        if let Commands::Azure {
            command:
                AzureCommands::Indexer {
                    command: IndexerCommands::Run { name },
                },
        } = cli.command
        {
            assert_eq!(name, "jira-issues");
        } else {
            panic!("expected azure indexer run");
        }
    }
}
