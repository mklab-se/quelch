mod cli;

use anyhow::{Context, Result};
use clap::Parser;
use quelch::{ai, config, copilot, search, sync};
use std::path::Path;
use tracing::info;
use tracing_subscriber::EnvFilter;

use cli::{Cli, Commands};
use sync::IndexMode;

fn setup_logging(verbose: u8, quiet: bool, json: bool) {
    let filter = match (quiet, verbose) {
        (true, _) => "error",
        (_, 0) => "quelch=info",
        (_, 1) => "quelch=debug",
        (_, 2) => "quelch=debug,reqwest=debug",
        _ => "trace",
    };

    let builder = tracing_subscriber::fmt().with_env_filter(EnvFilter::new(filter));

    if json {
        builder.json().init();
    } else {
        builder.init();
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    setup_logging(cli.verbose, cli.quiet, cli.json);

    match cli.command {
        Commands::Sync {
            create_indexes,
            purge,
            max_docs,
        } => cmd_sync(&cli.config, create_indexes, purge, max_docs).await,
        Commands::Watch {
            create_indexes,
            max_docs,
        } => cmd_watch(&cli.config, create_indexes, max_docs).await,
        Commands::Setup { yes } => cmd_setup(&cli.config, yes).await,
        Commands::Status => cmd_status(&cli.config),
        Commands::Reset { source } => cmd_reset(&cli.config, source.as_deref()),
        Commands::ResetIndexes => cmd_reset_indexes(&cli.config).await,
        Commands::Validate => cmd_validate(&cli.config),
        Commands::Init => cmd_init(),
        Commands::Search {
            query,
            index,
            top,
            json,
        } => {
            let config = config::load_config(&cli.config)?;
            search::run_search(&config, &query, index.as_deref(), top, json).await
        }
        Commands::Mock { port } => quelch::mock::run_mock_server(port).await,
        Commands::Ai { command } => ai::run(command).await,
        Commands::GenerateAgent { output } => cmd_generate_agent(&cli.config, &output),
    }
}

fn cmd_generate_agent(config_path: &Path, output_dir: &Path) -> Result<()> {
    let config = config::load_config(config_path)?;
    let output = copilot::generate(&config);

    std::fs::create_dir_all(output_dir).with_context(|| {
        format!(
            "failed to create output directory: {}",
            output_dir.display()
        )
    })?;

    let instructions_path = output_dir.join("agent-instructions.md");
    std::fs::write(&instructions_path, &output.instructions)?;
    println!("  Wrote {}", instructions_path.display());

    for topic in &output.topics {
        let topic_path = output_dir.join(&topic.filename);
        std::fs::write(&topic_path, &topic.yaml)?;
        println!("  Wrote {}", topic_path.display());
    }

    let guide_path = output_dir.join("guide.md");
    std::fs::write(&guide_path, &output.guide)?;
    println!("  Wrote {}", guide_path.display());

    println!(
        "\nGenerated {} file(s) in {}",
        output.topics.len() + 2,
        output_dir.display()
    );
    println!("Read {} to get started.", guide_path.display());

    Ok(())
}

async fn cmd_sync(
    config_path: &Path,
    auto_create: bool,
    purge: bool,
    max_docs: Option<u64>,
) -> Result<()> {
    let config = config::load_config(config_path)?;
    let state_path = Path::new(&config.sync.state_file).to_path_buf();
    let mode = if auto_create {
        IndexMode::AutoCreate
    } else {
        IndexMode::Interactive
    };
    let embedding = sync::load_embedding_config()?;
    let embed_client = ailloy::Client::for_capability("embedding")
        .context("failed to create embedding client — run 'quelch ai config' to set up")?;

    if let Some(limit) = max_docs {
        info!(max_docs = limit, "Starting one-shot sync (limited)");
    } else {
        info!("Starting one-shot sync");
    }
    sync::run_sync(
        &config,
        &state_path,
        &embedding,
        mode,
        Some(&embed_client),
        max_docs,
    )
    .await?;

    if purge {
        info!("Running orphan purge");
        sync::run_purge(&config).await?;
    }

    info!("Sync complete");
    Ok(())
}

async fn cmd_watch(config_path: &Path, auto_create: bool, max_docs: Option<u64>) -> Result<()> {
    let config = config::load_config(config_path)?;
    let state_path = Path::new(&config.sync.state_file).to_path_buf();
    let interval = std::time::Duration::from_secs(config.sync.poll_interval);

    let first_mode = if auto_create {
        IndexMode::AutoCreate
    } else {
        IndexMode::Interactive
    };
    let embedding = sync::load_embedding_config()?;
    let embed_client = ailloy::Client::for_capability("embedding")
        .context("failed to create embedding client — run 'quelch ai config' to set up")?;

    let purge_every = config.sync.purge_every;

    info!(
        poll_interval = config.sync.poll_interval,
        purge_every = purge_every,
        "Starting continuous sync (purge every {} cycles)",
        purge_every
    );

    let mut cycle: u64 = 0;
    loop {
        cycle += 1;
        let mode = if cycle == 1 {
            first_mode
        } else {
            IndexMode::RequireExisting
        };

        if let Err(e) = sync::run_sync(
            &config,
            &state_path,
            &embedding,
            mode,
            Some(&embed_client),
            max_docs,
        )
        .await
        {
            tracing::error!(error = %e, "Sync cycle failed");
        }

        // Run purge every Nth cycle
        if purge_every > 0 && cycle.is_multiple_of(purge_every) {
            info!("Running scheduled orphan purge (cycle {})", cycle);
            if let Err(e) = sync::run_purge(&config).await {
                tracing::error!(error = %e, "Purge failed");
            }
        }

        info!("Next sync in {} seconds", config.sync.poll_interval);
        tokio::time::sleep(interval).await;
    }
}

async fn cmd_setup(config_path: &Path, auto_yes: bool) -> Result<()> {
    let config = config::load_config(config_path)?;
    let mode = if auto_yes {
        IndexMode::AutoCreate
    } else {
        IndexMode::Interactive
    };
    let embedding = sync::load_embedding_config()?;

    println!("Checking indexes for {} source(s)...", config.sources.len());
    let created = sync::setup_indexes(&config, &embedding, mode).await?;

    if created.is_empty() {
        println!("\nAll indexes are ready.");
    } else {
        println!("\nCreated {} index(es).", created.len());
    }

    Ok(())
}

fn cmd_status(config_path: &Path) -> Result<()> {
    let config = config::load_config(config_path)?;
    let state_path = Path::new(&config.sync.state_file);
    let state = sync::state::SyncState::load(state_path)?;

    println!("Quelch Status");
    println!("{}", "\u{2500}".repeat(50));
    println!("Config: {}", config_path.display());
    println!("Sources: {}", config.sources.len());
    println!();

    for source_config in &config.sources {
        let name = source_config.name();
        let source_state = state.get_source(name);

        let last_sync = source_state
            .last_sync_at
            .map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "never".to_string());

        println!("  {} ({})", name, source_config.index());
        println!("    Last sync:   {}", last_sync);
        println!("    Docs synced: {}", source_state.documents_synced);
        println!("    Sync count:  {}", source_state.sync_count);
        println!();
    }

    Ok(())
}

async fn cmd_reset_indexes(config_path: &Path) -> Result<()> {
    let config = config::load_config(config_path)?;
    let state_path = Path::new(&config.sync.state_file).to_path_buf();

    println!("Deleting indexes and clearing sync state...");
    let deleted = sync::reset_indexes(&config, &state_path).await?;

    if deleted.is_empty() {
        println!("\nNo indexes to delete.");
    } else {
        println!(
            "\nDeleted {} index(es). Run 'quelch setup' to recreate.",
            deleted.len()
        );
    }

    Ok(())
}

fn cmd_reset(config_path: &Path, source: Option<&str>) -> Result<()> {
    let config = config::load_config(config_path)?;
    let state_path = Path::new(&config.sync.state_file);
    let mut state = sync::state::SyncState::load(state_path)?;

    match source {
        Some(name) => {
            state.reset_source(name);
            println!("Reset sync state for source '{}'", name);
        }
        None => {
            state.reset_all();
            println!("Reset sync state for all sources");
        }
    }

    state.save(state_path)?;
    Ok(())
}

fn cmd_validate(config_path: &Path) -> Result<()> {
    let config = config::load_config(config_path)?;
    println!("Config is valid.");
    println!("  Azure endpoint: {}", config.azure.endpoint);
    println!("  Sources: {}", config.sources.len());
    for source in &config.sources {
        println!("    - {} -> index '{}'", source.name(), source.index());
    }
    Ok(())
}

fn cmd_init() -> Result<()> {
    let template = r#"# quelch.yaml

azure:
  endpoint: "https://your-search-service.search.windows.net"
  api_key: "${AZURE_SEARCH_API_KEY}"

sources:
  # Jira Cloud example (uses email + API token)
  - type: jira
    name: "my-jira-cloud"
    url: "https://your-company.atlassian.net"
    auth:
      email: "${JIRA_EMAIL}"
      api_token: "${JIRA_API_TOKEN}"
    projects:
      - "PROJ"
    index: "jira-issues"

  # Jira Data Center example (uses PAT)
  # - type: jira
  #   name: "my-jira-dc"
  #   url: "https://jira.internal.company.com"
  #   auth:
  #     pat: "${JIRA_PAT}"
  #   projects:
  #     - "HR"
  #   index: "jira-issues"

  # Confluence Cloud example
  # - type: confluence
  #   name: "my-confluence"
  #   url: "https://your-company.atlassian.net/wiki"
  #   auth:
  #     email: "${CONFLUENCE_EMAIL}"
  #     api_token: "${CONFLUENCE_API_TOKEN}"
  #   spaces:
  #     - "ENG"
  #   index: "confluence-pages"

# Optional overrides (all have sensible defaults)
# sync:
#   poll_interval: 300
#   batch_size: 100
#   max_concurrent_per_credential: 3
#   state_file: ".quelch-state.json"
"#;

    let path = Path::new("quelch.yaml");
    if path.exists() {
        anyhow::bail!("quelch.yaml already exists — remove it first or edit it directly");
    }

    std::fs::write(path, template)?;
    println!("Created quelch.yaml — edit it with your Azure and source credentials");
    Ok(())
}
