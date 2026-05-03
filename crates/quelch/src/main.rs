// TODO(quelch v2 phase 3+): re-enable v1 commands as they are replaced by v2 equivalents.
//
// The v1 CLI commands (sync, watch, setup, reset-indexes, status, search, sim,
// generate-agent) are stubbed for the v2 config layer work (Phase 1).
// Each will be replaced by v2 commands in Phases 3–8.

mod cli;

use anyhow::Result;
use clap::Parser;
use quelch::azure::deploy::whatif::WhatIfReport;
use quelch::config;
use quelch::config::DeploymentTarget;
use std::path::{Path, PathBuf};
use tracing_subscriber::EnvFilter;

use cli::{
    AgentCommands, AgentTarget, AzureCommands, Cli, Commands, IndexerCommands, OnpremTargetArg,
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
        Commands::Validate => cmd_validate(&cli.config),
        Commands::EffectiveConfig { name } => cmd_effective_config(&cli.config, &name),
        Commands::Init {
            non_interactive,
            from_template,
            force,
        } => {
            let path = std::path::PathBuf::from("quelch.yaml");
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
        Commands::GenerateDeployment {
            name,
            target,
            output,
        } => cmd_generate_deployment(&cli.config, &name, target, &output),
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
        Commands::Status {
            deployment,
            json,
            tui,
        } => {
            let config = quelch::config::load_config(&cli.config)?;
            quelch::commands::status::run(
                &config,
                quelch::commands::status::StatusOptions {
                    deployment,
                    json,
                    tui,
                },
            )
            .await
        }
        Commands::Reset {
            source,
            subsource,
            yes,
        } => {
            let config = quelch::config::load_config(&cli.config)?;
            quelch::commands::reset::run(
                &config,
                quelch::commands::reset::ResetOptions {
                    source,
                    subsource,
                    yes,
                },
            )
            .await
        }
        Commands::ResetIndexes => {
            anyhow::bail!(
                "quelch reset-indexes is not available in v2; use `quelch azure indexer reset` (Phase 4)"
            )
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
        } => {
            let config = quelch::config::load_config(&cli.config)?;
            // Parse --where / --where-file into a JSON Value.
            let where_val = parse_where_arg(r#where.as_deref(), where_file.as_deref())?;
            // Parse --order-by strings.
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
                },
            )
            .await
        }
        Commands::Get {
            id,
            data_source,
            include_deleted,
            json,
        } => {
            let config = quelch::config::load_config(&cli.config)?;
            quelch::commands::get::run(
                &config,
                quelch::commands::get::GetOptions {
                    id,
                    data_source,
                    include_deleted,
                    json,
                },
            )
            .await
        }
        Commands::Sim { .. } => {
            anyhow::bail!("quelch sim is not available in v2; use `quelch dev` (Phase 3/4)")
        }
        Commands::GenerateAgent { .. } => {
            anyhow::bail!(
                "quelch generate-agent is not available in v2; use `quelch agent generate` (Phase 8)"
            )
        }
        Commands::Agent {
            command:
                AgentCommands::Generate {
                    target,
                    format: _format,
                    output,
                    deployment,
                    url,
                },
        } => cmd_agent_generate(&cli.config, target, output, deployment, url),
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
            deployment,
            port,
            bind,
            api_key,
        } => {
            let config = quelch::config::load_config(&cli.config)?;
            if let Some(key) = api_key {
                // SAFETY: called once at process start before spawning async tasks.
                unsafe { std::env::set_var("QUELCH_MCP_API_KEY", key) };
            }
            quelch::mcp::run_server(&config, &deployment, &format!("{bind}:{port}")).await
        }
        Commands::Azure { command } => match command {
            AzureCommands::Plan {
                deployment,
                out,
                no_what_if,
            } => cmd_azure_plan(&cli.config, deployment, out, no_what_if).await,
            AzureCommands::Deploy {
                deployment,
                yes,
                dry_run,
            } => cmd_azure_deploy(&cli.config, deployment, yes, dry_run).await,
            AzureCommands::Pull { kind, diff } => cmd_azure_pull(&cli.config, kind, diff).await,
            AzureCommands::Indexer { command } => cmd_azure_indexer(&cli.config, command).await,
            AzureCommands::Logs {
                deployment,
                tail,
                follow,
                since,
            } => cmd_azure_logs(&cli.config, &deployment, tail, follow, since.as_deref()).await,
            AzureCommands::Destroy { deployment, yes } => {
                cmd_azure_destroy(&cli.config, &deployment, yes).await
            }
        },
    }
}

// ---------------------------------------------------------------------------
// CLI helpers
// ---------------------------------------------------------------------------

/// Parse a `--where` JSON string or read JSON from `--where-file`.
///
/// Returns `None` if neither argument is provided, or the parsed `Value` on
/// success.  Returns an error on invalid JSON or file-read failure.
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

// ---------------------------------------------------------------------------
// quelch generate-deployment
// ---------------------------------------------------------------------------

fn cmd_generate_deployment(
    config_path: &Path,
    name: &str,
    target: OnpremTargetArg,
    output: &Path,
) -> Result<()> {
    let config = config::load_config(config_path)?;
    let onprem_target = match target {
        OnpremTargetArg::Docker => quelch::onprem::OnpremTarget::Docker,
        OnpremTargetArg::Systemd => quelch::onprem::OnpremTarget::Systemd,
        OnpremTargetArg::K8s => quelch::onprem::OnpremTarget::K8s,
    };
    let outcome = quelch::onprem::generate(&config, name, onprem_target, output)?;
    println!(
        "Generated {} artefact(s) in {}:",
        outcome.written.len(),
        output.display()
    );
    for p in &outcome.written {
        println!("  {}", p.display());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// quelch azure plan
// ---------------------------------------------------------------------------

async fn cmd_azure_plan(
    config_path: &Path,
    deployment: Option<String>,
    out: Option<PathBuf>,
    no_what_if: bool,
) -> Result<()> {
    let config = quelch::config::load_config(config_path)?;

    let targets: Vec<&quelch::config::DeploymentConfig> = match deployment.as_deref() {
        Some(name) => vec![
            config
                .deployments
                .iter()
                .find(|d| d.name == name)
                .ok_or_else(|| anyhow::anyhow!("deployment '{}' not found", name))?,
        ],
        None => config.deployments.iter().collect(),
    };

    for dep in targets {
        plan_one(&config, dep, out.as_deref(), no_what_if).await?;
    }
    Ok(())
}

async fn plan_one(
    config: &quelch::config::Config,
    deployment: &quelch::config::DeploymentConfig,
    out: Option<&Path>,
    no_what_if: bool,
) -> Result<()> {
    println!("Planning deployment '{}'", deployment.name);

    if matches!(deployment.target, DeploymentTarget::Onprem) {
        println!("  target=onprem; use `quelch generate-deployment` instead");
        return Ok(());
    }

    // 1. Synthesise Bicep.
    let bicep = quelch::azure::deploy::bicep::generate(config, &deployment.name)?;
    let bicep_path = out
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!(".quelch/azure/{}.bicep", deployment.name)));
    if let Some(parent) = bicep_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&bicep_path, &bicep)?;
    println!("  Synthesised {}", bicep_path.display());

    // 2. Synthesise rigg files.
    let sliced = quelch::config::slice::for_deployment(config, &deployment.name)?;
    let generated = quelch::azure::rigg::generate::all(&sliced)?;
    let rigg_root = PathBuf::from(&config.rigg.dir);
    let _write_outcome =
        quelch::azure::rigg::write::write_to_disk(&generated, &config.rigg, &rigg_root)?;
    println!("  Synthesised rigg/ files at {}", rigg_root.display());

    // 3. Run Bicep what-if (unless --no-what-if).
    let bicep_report = if no_what_if {
        WhatIfReport {
            creates: vec![],
            modifies: vec![],
            deletes: vec![],
            unchanged: vec![],
            raw_json: serde_json::Value::Null,
        }
    } else {
        match quelch::azure::deploy::whatif::run(&config.azure.resource_group, &bicep_path) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  what-if failed: {e}");
                eprintln!("  (run with --no-what-if to skip)");
                return Err(anyhow::anyhow!(e));
            }
        }
    };

    // 4. Run rigg plan (requires Azure access; skipped when what-if also skipped to
    //    allow full offline use).
    let rigg_report = if no_what_if {
        quelch::azure::rigg::plan::PlanReport::default()
    } else {
        let service = config
            .search
            .service
            .as_deref()
            .unwrap_or("quelch-prod-search");
        let endpoint = format!("https://{service}.search.windows.net");
        let api_version = "2024-03-01-preview".to_string();
        let api = quelch::azure::rigg::plan::RiggClientAdapter::new(endpoint, api_version)?;
        quelch::azure::rigg::plan::run(&rigg_root, &api).await?
    };

    // 5. Render combined diff.
    let diff = quelch::azure::deploy::diff_view::render(&bicep_report, &rigg_report);
    print!("{diff}");

    Ok(())
}

// ---------------------------------------------------------------------------
// quelch azure deploy
// ---------------------------------------------------------------------------

async fn cmd_azure_deploy(
    config_path: &Path,
    deployment: Option<String>,
    yes: bool,
    dry_run: bool,
) -> Result<()> {
    if dry_run {
        // --dry-run is equivalent to plan.
        return cmd_azure_plan(config_path, deployment, None, false).await;
    }

    let config = quelch::config::load_config(config_path)?;

    let targets: Vec<&quelch::config::DeploymentConfig> = match deployment.as_deref() {
        Some(name) => vec![
            config
                .deployments
                .iter()
                .find(|d| d.name == name)
                .ok_or_else(|| anyhow::anyhow!("deployment '{}' not found", name))?,
        ],
        None => config.deployments.iter().collect(),
    };

    for dep in targets {
        deploy_one(&config, dep, yes).await?;
    }
    Ok(())
}

async fn deploy_one(
    config: &quelch::config::Config,
    deployment: &quelch::config::DeploymentConfig,
    yes: bool,
) -> Result<()> {
    // Show the plan first (with what-if).
    plan_one(config, deployment, None, false).await?;

    if matches!(deployment.target, DeploymentTarget::Onprem) {
        return Ok(());
    }

    // Prompt unless --yes.
    if !yes {
        let confirmed = dialoguer::Confirm::new()
            .with_prompt(format!(
                "Apply changes to deployment '{}'?",
                deployment.name
            ))
            .default(false)
            .interact()?;
        if !confirmed {
            println!("Aborted.");
            return Ok(());
        }
    }

    // Apply Bicep.
    let bicep_path = PathBuf::from(format!(".quelch/azure/{}.bicep", deployment.name));
    println!("  Applying Bicep for '{}'…", deployment.name);
    let outcome = quelch::azure::deploy::apply::run(&config.azure.resource_group, &bicep_path)?;
    println!("  Provisioning state: {}", outcome.provisioning_state);

    // Push rigg resources.
    let rigg_root = PathBuf::from(&config.rigg.dir);
    let service = config
        .search
        .service
        .as_deref()
        .unwrap_or("quelch-prod-search");
    let endpoint = format!("https://{service}.search.windows.net");
    let api_version = "2024-03-01-preview".to_string();
    let api = quelch::azure::rigg::plan::RiggClientAdapter::new(endpoint, api_version)?;
    let plan = quelch::azure::rigg::plan::run(&rigg_root, &api).await?;
    let push_outcome = quelch::azure::rigg::push::run(plan, &rigg_root, &api).await?;
    println!(
        "  rigg: {} created, {} updated, {} deleted.",
        push_outcome.created.len(),
        push_outcome.updated.len(),
        push_outcome.deleted.len(),
    );

    // Save last.json snapshot.
    let snapshot_dir = PathBuf::from(".quelch/azure");
    std::fs::create_dir_all(&snapshot_dir)?;
    let snapshot_path = snapshot_dir.join(format!("{}.last.json", deployment.name));
    std::fs::write(&snapshot_path, serde_json::to_string_pretty(&outcome.raw)?)?;
    println!("  Saved snapshot to {}", snapshot_path.display());

    Ok(())
}

// ---------------------------------------------------------------------------
// quelch azure pull
// ---------------------------------------------------------------------------

async fn cmd_azure_pull(config_path: &Path, kind: Option<String>, diff: bool) -> Result<()> {
    let config = quelch::config::load_config(config_path)?;

    let parsed_kind = kind
        .as_deref()
        .map(parse_resource_kind)
        .transpose()
        .map_err(|e| anyhow::anyhow!(e))?;

    let service = config
        .search
        .service
        .as_deref()
        .unwrap_or("quelch-prod-search");
    let endpoint = format!("https://{service}.search.windows.net");
    let api_version = "2024-03-01-preview".to_string();
    let api = quelch::azure::rigg::plan::RiggClientAdapter::new(endpoint, api_version)?;

    let options = quelch::azure::rigg::pull::PullOptions {
        kind: parsed_kind,
        diff_only: diff,
    };

    let rigg_root = PathBuf::from(&config.rigg.dir);
    let outcome = quelch::azure::rigg::pull::run(&rigg_root, &api, options).await?;

    if diff {
        println!(
            "Would write {} file(s); {} skipped (managed-by-user).",
            outcome.written.len(),
            outcome.skipped_managed_by_user.len(),
        );
        for p in &outcome.written {
            println!("  {}", p.display());
        }
    } else {
        println!(
            "Wrote {} file(s); {} skipped (managed-by-user).",
            outcome.written.len(),
            outcome.skipped_managed_by_user.len(),
        );
    }

    Ok(())
}

/// Parse a human-friendly resource kind string into a [`rigg_core::resources::ResourceKind`].
fn parse_resource_kind(s: &str) -> Result<rigg_core::resources::ResourceKind, String> {
    match s.to_lowercase().replace('-', "_").as_str() {
        "index" | "indexes" => Ok(rigg_core::resources::ResourceKind::Index),
        "datasource" | "datasources" | "data_source" | "data_sources" => {
            Ok(rigg_core::resources::ResourceKind::DataSource)
        }
        "skillset" | "skillsets" => Ok(rigg_core::resources::ResourceKind::Skillset),
        "indexer" | "indexers" => Ok(rigg_core::resources::ResourceKind::Indexer),
        "knowledge_source" | "knowledge_sources" => {
            Ok(rigg_core::resources::ResourceKind::KnowledgeSource)
        }
        "knowledge_base" | "knowledge_bases" => {
            Ok(rigg_core::resources::ResourceKind::KnowledgeBase)
        }
        "synonym_map" | "synonym_maps" => Ok(rigg_core::resources::ResourceKind::SynonymMap),
        "alias" | "aliases" => Ok(rigg_core::resources::ResourceKind::Alias),
        "agent" | "agents" => Ok(rigg_core::resources::ResourceKind::Agent),
        other => Err(format!("unknown resource kind '{other}'")),
    }
}

// ---------------------------------------------------------------------------
// quelch azure indexer
// ---------------------------------------------------------------------------

async fn cmd_azure_indexer(config_path: &Path, command: IndexerCommands) -> Result<()> {
    let config = quelch::config::load_config(config_path)?;
    let service = config
        .search
        .service
        .as_deref()
        .unwrap_or("quelch-prod-search");

    match command {
        IndexerCommands::Run { name } => {
            quelch::azure::deploy::indexer::run(service, &name)?;
            println!("Triggered indexer run for '{name}'.");
        }
        IndexerCommands::Reset { name } => {
            quelch::azure::deploy::indexer::reset(service, &name)?;
            println!("Reset indexer '{name}' — full re-index will run on next schedule.");
        }
        IndexerCommands::Status => {
            let statuses = quelch::azure::deploy::indexer::status(service)?;
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
// quelch azure logs
// ---------------------------------------------------------------------------

async fn cmd_azure_logs(
    config_path: &Path,
    deployment: &str,
    tail: usize,
    follow: bool,
    since: Option<&str>,
) -> Result<()> {
    let config = quelch::config::load_config(config_path)?;
    let app_name = quelch::azure::deploy::naming::container_app_name(&config, deployment);
    quelch::azure::deploy::logs::tail(
        &app_name,
        &config.azure.resource_group,
        tail,
        follow,
        since,
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// quelch azure destroy
// ---------------------------------------------------------------------------

async fn cmd_azure_destroy(config_path: &Path, deployment: &str, yes: bool) -> Result<()> {
    let config = quelch::config::load_config(config_path)?;
    let _dep = config
        .deployments
        .iter()
        .find(|d| d.name == deployment)
        .ok_or_else(|| anyhow::anyhow!("deployment '{}' not found", deployment))?;

    if !yes {
        let confirmed = dialoguer::Confirm::new()
            .with_prompt(format!(
                "Destroy Container App for deployment '{deployment}'?"
            ))
            .default(false)
            .interact()?;
        if !confirmed {
            println!("Aborted.");
            return Ok(());
        }
    }

    let app_name = quelch::azure::deploy::naming::container_app_name(&config, deployment);
    quelch::azure::deploy::destroy::run(&app_name, &config.azure.resource_group)?;
    println!("Destroyed Container App '{app_name}'.");

    // Clean up the snapshot if it exists.
    let snapshot_path = PathBuf::from(format!(".quelch/azure/{deployment}.last.json"));
    quelch::azure::deploy::destroy::remove_snapshot(&snapshot_path);

    Ok(())
}

// ---------------------------------------------------------------------------
// quelch agent generate
// ---------------------------------------------------------------------------

fn cmd_agent_generate(
    config_path: &Path,
    target: AgentTarget,
    output: PathBuf,
    deployment: Option<String>,
    url: Option<String>,
) -> Result<()> {
    let config = quelch::config::load_config(config_path)?;

    // Resolve deployment name: use explicit arg or find the first MCP deployment.
    let deployment_name = deployment
        .or_else(|| {
            config
                .deployments
                .iter()
                .find(|d| d.role == quelch::config::DeploymentRole::Mcp)
                .map(|d| d.name.clone())
        })
        .ok_or_else(|| anyhow::anyhow!("no MCP deployment found in config; use --deployment"))?;

    // Build the bundle.
    let mut bundle = quelch::agent::bundle::build(&config, &deployment_name)?;

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
        // Verify the CLI can be constructed — smoke test only.
        let cli = Cli::parse_from(["quelch", "validate"]);
        assert!(matches!(cli.command, Commands::Validate));
    }

    #[test]
    fn cli_parses_azure_plan() {
        let cli = Cli::parse_from(["quelch", "azure", "plan", "ingest", "--no-what-if"]);
        assert!(matches!(
            cli.command,
            Commands::Azure {
                command: AzureCommands::Plan {
                    no_what_if: true,
                    ..
                }
            }
        ));
    }

    #[test]
    fn cli_parses_azure_deploy_dry_run() {
        let cli = Cli::parse_from(["quelch", "azure", "deploy", "--dry-run"]);
        assert!(matches!(
            cli.command,
            Commands::Azure {
                command: AzureCommands::Deploy { dry_run: true, .. }
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

    #[test]
    fn cli_parses_azure_logs() {
        let cli = Cli::parse_from([
            "quelch", "azure", "logs", "ingest", "--tail", "200", "--follow",
        ]);
        if let Commands::Azure {
            command:
                AzureCommands::Logs {
                    deployment,
                    tail,
                    follow,
                    ..
                },
        } = cli.command
        {
            assert_eq!(deployment, "ingest");
            assert_eq!(tail, 200);
            assert!(follow);
        } else {
            panic!("expected azure logs");
        }
    }

    #[test]
    fn parse_resource_kind_recognises_common_aliases() {
        assert!(parse_resource_kind("index").is_ok());
        assert!(parse_resource_kind("indexes").is_ok());
        assert!(parse_resource_kind("indexer").is_ok());
        assert!(parse_resource_kind("knowledge_base").is_ok());
        assert!(parse_resource_kind("knowledge-base").is_ok());
        assert!(parse_resource_kind("bogus").is_err());
    }
}
