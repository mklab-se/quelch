use clap::Parser;
use quelch::ai::AiCommands;
use quelch::commands::search::IncludeContentArg;
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

    /// Disable TUI and fall back to plain structured logs
    #[arg(long, global = true)]
    pub no_tui: bool,
}

#[derive(clap::Subcommand)]
pub enum Commands {
    /// Run a one-shot sync of all configured sources
    Sync {
        /// Auto-create missing indexes without prompting
        #[arg(long)]
        create_indexes: bool,
        /// Also purge orphaned documents from indexes
        #[arg(long)]
        purge: bool,
        /// Maximum number of documents to sync (useful for debugging)
        #[arg(long)]
        max_docs: Option<u64>,
    },
    /// Run continuous sync (polls at configured interval)
    Watch {
        /// Auto-create missing indexes without prompting
        #[arg(long)]
        create_indexes: bool,
        /// Maximum number of documents to sync per cycle (useful for debugging)
        #[arg(long)]
        max_docs: Option<u64>,
    },
    /// Check and create Azure AI Search indexes needed by the config
    Setup {
        /// Auto-create without prompting
        #[arg(short, long)]
        yes: bool,
    },
    /// Show sync status for all sources
    Status {
        /// Filter to cursors belonging to this deployment
        #[arg(long)]
        deployment: Option<String>,
        /// Emit machine-readable JSON instead of a table
        #[arg(long)]
        json: bool,
        /// Launch the interactive TUI (planned for Phase 10)
        #[arg(long)]
        tui: bool,
    },
    /// Reset sync state (force full re-sync on next run)
    Reset {
        /// Source name to reset (omit to reset all)
        #[arg(long)]
        source: Option<String>,
        /// Only reset a single subsource (project or space key) within the source
        #[arg(long)]
        subsource: Option<String>,
        /// Skip the interactive confirmation prompt
        #[arg(long)]
        yes: bool,
    },
    /// Delete all configured indexes from Azure AI Search and clear sync state
    ResetIndexes,
    /// Validate config file without running
    Validate,
    /// Print the effective (sliced) config for one deployment
    EffectiveConfig {
        /// Name of the deployment to slice for
        name: String,
    },
    /// Interactive wizard to scaffold a quelch.yaml config
    Init {
        /// Skip all prompts and write a template directly.
        #[arg(long)]
        non_interactive: bool,
        /// Template name to use in non-interactive mode (minimal, multi-source, distributed).
        #[arg(long)]
        from_template: Option<String>,
        /// Overwrite an existing quelch.yaml without asking.
        #[arg(long)]
        force: bool,
    },
    /// Generate ready-to-run on-prem deployment artefacts
    GenerateDeployment {
        /// Deployment name from quelch.yaml (should be target: onprem).
        name: String,
        /// Output target: docker, systemd, or k8s.
        #[arg(long, value_enum)]
        target: OnpremTargetArg,
        /// Output directory for generated artefacts.
        #[arg(long, default_value = "./deploy")]
        output: PathBuf,
    },
    /// Start a local mock Jira and Confluence server for testing
    Mock {
        /// Port to listen on
        #[arg(short, long, default_value = "9999")]
        port: u16,
    },
    /// Run quelch against a fully simulated environment for local testing and CI.
    Sim {
        /// Run for this long then exit. Default: run until Ctrl-C. Example: 30s, 2m, 1h.
        #[arg(long)]
        duration: Option<humantime::Duration>,
        /// Seed the activity generator for reproducible runs.
        #[arg(long)]
        seed: Option<u64>,
        /// Scale activity rate. 1.0 = default, 2.0 = twice as fast.
        #[arg(long, default_value = "1.0")]
        rate_multiplier: f64,
        /// Probability each Azure request gets a 429 or 503. 0.0 disables.
        #[arg(long, default_value = "0.03")]
        fault_rate: f64,
        /// CI-friendly: fail with exit code 1 if fewer than N docs are indexed.
        #[arg(long)]
        assert_docs: Option<u64>,
        /// Render the TUI to a headless backend and write a multi-frame text
        /// dump to this file. Enables deterministic verification of the TUI
        /// from CI or an AI agent. Implies --no-tui for stdout.
        #[arg(long)]
        snapshot_to: Option<PathBuf>,
        /// Number of frames to capture when --snapshot-to is set.
        #[arg(long, default_value = "10")]
        snapshot_frames: u32,
        /// Width of the TestBackend when --snapshot-to is set. Default 120.
        #[arg(long, default_value = "120")]
        snapshot_width: u16,
        /// Height of the TestBackend when --snapshot-to is set. Default 40.
        #[arg(long, default_value = "40")]
        snapshot_height: u16,
    },
    /// Structured query against a Cosmos-backed data source
    Query {
        /// Logical data-source name (e.g. jira_issues)
        #[arg(long, value_name = "NAME")]
        data_source: String,
        /// Structured filter predicate as JSON (e.g. '{"status":"Open"}')
        #[arg(long, value_name = "JSON")]
        r#where: Option<String>,
        /// Read filter JSON from a file instead of --where
        #[arg(long, value_name = "PATH")]
        where_file: Option<PathBuf>,
        /// Sort clause — repeatable, format field:dir (e.g. updated:desc)
        #[arg(long, value_name = "FIELD:DIR")]
        order_by: Vec<String>,
        /// Maximum documents per page
        #[arg(long, default_value = "50")]
        top: usize,
        /// Pagination cursor from a prior response
        #[arg(long)]
        cursor: Option<String>,
        /// Return only the document count
        #[arg(long)]
        count_only: bool,
        /// Include soft-deleted documents
        #[arg(long)]
        include_deleted: bool,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Semantic / hybrid search via Azure AI Search
    Search {
        /// Free-text search query
        query: String,
        /// Comma-separated logical data-source names to search
        #[arg(long, value_name = "NAMES")]
        data_sources: Option<String>,
        /// Structured filter predicate as JSON
        #[arg(long, value_name = "JSON")]
        r#where: Option<String>,
        /// Maximum hits per page
        #[arg(long, default_value = "25")]
        top: usize,
        /// Pagination cursor from a prior response
        #[arg(long)]
        cursor: Option<String>,
        /// Content level to return
        #[arg(long, value_enum, default_value = "snippet")]
        include_content: IncludeContentArg,
        /// Include soft-deleted documents
        #[arg(long)]
        include_deleted: bool,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Fetch a single document by ID from a data source
    Get {
        /// Document ID
        id: String,
        /// Logical data-source name (required)
        #[arg(long)]
        data_source: String,
        /// Include soft-deleted documents
        #[arg(long)]
        include_deleted: bool,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Manage AI embedding configuration
    Ai {
        #[command(subcommand)]
        command: Option<AiCommands>,
    },
    /// Generate Copilot Studio agent topics and instructions
    GenerateAgent {
        /// Output directory for generated files
        #[arg(short, long, default_value = "copilot-studio")]
        output: PathBuf,
    },
    /// Generate an agent or skill bundle for a specific platform
    Agent {
        #[command(subcommand)]
        command: AgentCommands,
    },
    /// Run the continuous ingest worker for a deployment
    Ingest {
        /// Deployment name — which slice of the config this worker owns.
        #[arg(long)]
        deployment: String,
        /// Run one cycle then exit (useful for debugging and CI).
        #[arg(long)]
        once: bool,
        /// Stop after ingesting N documents (debugging).
        #[arg(long)]
        max_docs: Option<u64>,
    },
    /// Azure resource management commands (plan, deploy, pull, indexer, logs, destroy).
    Azure {
        #[command(subcommand)]
        command: AzureCommands,
    },
    /// Start the MCP HTTP server for a deployment.
    ///
    /// Agents (GitHub Copilot, Claude, etc.) connect to this server to query
    /// indexed data via the Model Context Protocol.
    ///
    /// Example: quelch mcp --deployment mcp --port 8080
    Mcp {
        /// Deployment name (required). Tells the server which slice of the
        /// config it owns and which data sources it exposes.
        #[arg(long)]
        deployment: String,
        /// Port to listen on.
        #[arg(short, long, default_value = "8080")]
        port: u16,
        /// Bind address.
        #[arg(long, default_value = "0.0.0.0")]
        bind: String,
        /// Override the API key (default: read from QUELCH_MCP_API_KEY env var).
        /// When neither is set the server runs in unauthenticated dev mode.
        #[arg(long)]
        api_key: Option<String>,
    },
}

/// On-prem target for `quelch generate-deployment`.
#[derive(Clone, clap::ValueEnum)]
pub enum OnpremTargetArg {
    /// Docker Compose.
    Docker,
    /// systemd unit file.
    Systemd,
    /// Kubernetes manifests.
    K8s,
}

/// Top-level `quelch azure` subcommands.
#[derive(clap::Subcommand)]
pub enum AzureCommands {
    /// Synthesise Bicep + rigg files; show the combined diff. No changes applied.
    Plan {
        /// Deployment name (omit to plan all).
        deployment: Option<String>,
        /// Write Bicep to a custom location (default .quelch/azure/<name>.bicep).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Synthesise only; skip the `az deployment group what-if` call.
        #[arg(long)]
        no_what_if: bool,
    },
    /// Plan + apply the deployment to Azure.
    Deploy {
        /// Deployment name (omit to deploy all).
        deployment: Option<String>,
        /// Skip the interactive confirmation prompt.
        #[arg(long)]
        yes: bool,
        /// Equivalent to `quelch azure plan` — show the diff but don't apply.
        #[arg(long)]
        dry_run: bool,
    },
    /// Pull live AI Search/Foundry config back into rigg/.
    Pull {
        /// Optional resource type filter (e.g. "index", "indexer").
        kind: Option<String>,
        /// Show what would change without writing.
        #[arg(long)]
        diff: bool,
    },
    /// Operate Azure AI Search Indexers.
    Indexer {
        #[command(subcommand)]
        command: IndexerCommands,
    },
    /// Tail logs from a deployed Container App.
    Logs {
        /// Deployment name.
        deployment: String,
        /// Number of log lines to show.
        #[arg(long, default_value = "100")]
        tail: usize,
        /// Stream logs continuously (Ctrl-C to stop).
        #[arg(long)]
        follow: bool,
        /// Only show logs since this time (e.g. "1h", "30m").
        #[arg(long)]
        since: Option<String>,
    },
    /// Remove a single deployment's Container App from Azure.
    Destroy {
        /// Deployment name.
        deployment: String,
        /// Skip the interactive confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

/// `quelch agent` subcommands.
#[derive(clap::Subcommand)]
pub enum AgentCommands {
    /// Generate an agent or skill bundle for the given target platform.
    Generate {
        /// Target platform to generate the bundle for.
        #[arg(long, value_enum)]
        target: AgentTarget,

        /// Output format: agent, skill, or both (where supported).
        #[arg(long, value_enum)]
        format: Option<AgentFormat>,

        /// Output directory for the generated bundle.
        #[arg(long, default_value = "./agent-bundle")]
        output: PathBuf,

        /// MCP deployment name (defaults to the first MCP deployment in config).
        #[arg(long)]
        deployment: Option<String>,

        /// Override the public URL of the MCP server.
        ///
        /// Required when the URL is not derivable from config (e.g. custom domain).
        #[arg(long)]
        url: Option<String>,
    },
}

/// Target platform for `quelch agent generate`.
#[derive(Clone, clap::ValueEnum)]
pub enum AgentTarget {
    /// Microsoft Copilot Studio (agent form).
    CopilotStudio,
    /// Anthropic Claude Code (skill form).
    ClaudeCode,
    /// GitHub Copilot CLI (skill form).
    CopilotCli,
    /// VS Code GitHub Copilot (skill form).
    VscodeCopilot,
    /// OpenAI Codex CLI (skill form).
    Codex,
    /// Generic markdown — both agent and skill forms.
    Markdown,
}

/// Output format override for `quelch agent generate`.
#[derive(Clone, clap::ValueEnum)]
pub enum AgentFormat {
    /// Generate agent-form output only.
    Agent,
    /// Generate skill-form output only.
    Skill,
    /// Generate both agent and skill forms.
    Both,
}

/// `quelch azure indexer` subcommands.
#[derive(clap::Subcommand)]
pub enum IndexerCommands {
    /// Trigger an immediate indexer run.
    Run {
        /// Indexer name.
        name: String,
    },
    /// Reset the indexer (forces full re-index on next run).
    Reset {
        /// Indexer name.
        name: String,
    },
    /// Show all indexers and their current state.
    Status,
}
