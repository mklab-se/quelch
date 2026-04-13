#![allow(dead_code)]

mod azure;
mod config;
mod sources;
mod sync;

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "quelch",
    version,
    about = "Ingest data directly into Azure AI Search"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Run a one-shot sync of all configured sources
    Sync,
    /// Run continuous sync (polls at configured interval)
    Watch,
    /// Show sync status for all sources
    Status,
    /// Reset sync state (force full re-sync on next run)
    Reset,
    /// Validate config file without running
    Validate,
    /// Generate a starter quelch.yaml config
    Init,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Sync => println!("sync not yet implemented"),
        Commands::Watch => println!("watch not yet implemented"),
        Commands::Status => println!("status not yet implemented"),
        Commands::Reset => println!("reset not yet implemented"),
        Commands::Validate => println!("validate not yet implemented"),
        Commands::Init => println!("init not yet implemented"),
    }

    Ok(())
}
