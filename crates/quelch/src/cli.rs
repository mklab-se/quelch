use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "quelch",
    version,
    about = "Ingest data directly into Azure AI Search"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Config file path
    #[arg(short, long, default_value = "quelch.yaml", global = true)]
    pub config: PathBuf,

    /// Increase verbosity (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Suppress TUI, only log errors
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Output logs as JSON
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(clap::Subcommand)]
pub enum Commands {
    /// Run a one-shot sync of all configured sources
    Sync {
        /// Auto-create missing indexes without prompting
        #[arg(long)]
        create_indexes: bool,
    },
    /// Run continuous sync (polls at configured interval)
    Watch {
        /// Auto-create missing indexes without prompting
        #[arg(long)]
        create_indexes: bool,
    },
    /// Check and create Azure AI Search indexes needed by the config
    Setup {
        /// Auto-create without prompting
        #[arg(short, long)]
        yes: bool,
    },
    /// Show sync status for all sources
    Status,
    /// Reset sync state (force full re-sync on next run)
    Reset {
        /// Source name to reset (omit to reset all)
        source: Option<String>,
    },
    /// Validate config file without running
    Validate,
    /// Generate a starter quelch.yaml config
    Init,
    /// Start a local mock Jira and Confluence server for testing
    Mock {
        /// Port to listen on
        #[arg(short, long, default_value = "9999")]
        port: u16,
    },
}
