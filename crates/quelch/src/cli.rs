use clap::Parser;
use quelch::ai::AiCommands;
use quelch::commands::instance::InstanceKindArg;
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
    /// Show sync status for all sources
    Status {
        /// Filter to cursors belonging to this instance
        #[arg(long)]
        instance: Option<String>,
        /// Emit machine-readable JSON instead of a table
        #[arg(long)]
        json: bool,
        /// Launch the interactive TUI (planned for Phase 10)
        #[arg(long)]
        tui: bool,
    },
    /// Reset sync state for a single `(source, subsource)` cursor.
    ///
    /// The instance must already own the cursor — pass `--take-ownership`
    /// to rewrite the cursor's owner from another instance to this one.
    Reset {
        /// Instance that owns (or wants to own) the cursor.
        #[arg(long)]
        instance: String,
        /// Source connection name as defined in `quelch.yaml`.
        #[arg(long)]
        source: String,
        /// Subsource (project key for Jira, space key for Confluence).
        #[arg(long)]
        subsource: String,
        /// Rewrite the cursor's `owner_instance` to this instance even if a
        /// different instance currently owns it.
        #[arg(long)]
        take_ownership: bool,
        /// Skip the interactive confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
    /// Validate config file without running
    Validate,
    /// Interactive wizard to scaffold a quelch.yaml config
    Init {
        /// Folder to write `quelch.yaml` into. Defaults to the current
        /// directory; `quelch init .` is the explicit form. The folder is
        /// created if it does not already exist.
        #[arg(default_value = ".")]
        directory: PathBuf,
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
    /// Start a local mock Jira and Confluence server for testing
    Mock {
        /// Port to listen on
        #[arg(short, long, default_value = "9999")]
        port: u16,
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
        /// MCP instance name; auto-detected when the config declares
        /// exactly one MCP instance.
        #[arg(long)]
        instance: Option<String>,
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
        /// MCP instance name; auto-detected when the config declares
        /// exactly one MCP instance.
        #[arg(long)]
        instance: Option<String>,
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
        /// MCP instance name; auto-detected when the config declares
        /// exactly one MCP instance.
        #[arg(long)]
        instance: Option<String>,
    },
    /// All-in-one local development mode (sim + ingest + MCP in one process).
    ///
    /// Starts a mock Jira/Confluence server, an in-memory Cosmos backend, an
    /// ingest worker, and an embedded MCP server — no cloud accounts needed.
    Dev {
        /// Use the real Azure AI Search adapter (requires Azure credentials).
        #[arg(long)]
        use_real_search: bool,
        /// Use the Cosmos emulator at https://localhost:8081 instead of in-memory.
        #[arg(long)]
        use_cosmos_emulator: bool,
        /// Port for the embedded MCP server.
        #[arg(long, default_value = "8080")]
        mcp_port: u16,
        /// Seed the fixture data generator (reserved for future use).
        #[arg(long)]
        seed: Option<u64>,
        /// Scale activity rate (reserved for future use).
        #[arg(long, default_value = "1.0")]
        rate_multiplier: f64,
    },
    /// Manage AI embedding configuration
    Ai {
        #[command(subcommand)]
        command: Option<AiCommands>,
    },
    /// Generate an agent or skill bundle for a specific platform
    Agent {
        #[command(subcommand)]
        command: AgentCommands,
    },
    /// Run the continuous ingest worker for an instance
    Ingest {
        /// Instance name — which slice of the config this worker owns.
        ///
        /// Optional: if the config declares exactly one ingest instance it
        /// is auto-selected. With multiple ingest instances this flag is
        /// required to disambiguate.
        #[arg(long)]
        instance: Option<String>,
        /// Run one cycle then exit (useful for debugging and CI).
        #[arg(long)]
        once: bool,
        /// Stop after ingesting N documents (debugging).
        #[arg(long)]
        max_docs: Option<u64>,
    },
    /// Azure resource management commands (plan, indexer).
    Azure {
        #[command(subcommand)]
        command: AzureCommands,
    },
    /// Manage named instances declared in `quelch.yaml`.
    ///
    /// Use these subcommands to inspect the instances declared in the master
    /// config and to emit per-instance config slices ready to copy onto the
    /// host that runs Q-Ingest or Q-MCP.
    Instance {
        #[command(subcommand)]
        command: InstanceCommand,
    },
    /// Start the MCP HTTP server for an instance.
    ///
    /// Agents (GitHub Copilot, Claude, etc.) connect to this server to query
    /// indexed data via the Model Context Protocol.
    ///
    /// Example: quelch mcp --instance mcp --port 8080
    Mcp {
        /// Instance name. Tells the server which slice of the config it owns
        /// and which data sources it exposes.
        ///
        /// Optional: if the config declares exactly one MCP instance it is
        /// auto-selected. With multiple MCP instances this flag is required
        /// to disambiguate.
        #[arg(long)]
        instance: Option<String>,
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

/// Top-level `quelch azure` subcommands.
#[derive(clap::Subcommand)]
pub enum AzureCommands {
    /// Compute the Cosmos + AI Search diff against the configured Azure
    /// account and print it to stdout. Read-only; never mutates Azure.
    Plan,
    /// Compute the diff, prompt for confirmation, then push the desired
    /// Cosmos containers and AI Search resources to Azure.
    Apply {
        /// Skip the interactive `[y/N]` confirmation prompt — useful in
        /// CI / non-TTY environments.
        #[arg(long)]
        yes: bool,
    },
    /// Operate Azure AI Search Indexers.
    Indexer {
        #[command(subcommand)]
        command: IndexerCommands,
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

        /// MCP instance name (defaults to the first MCP instance in config).
        #[arg(long)]
        instance: Option<String>,

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

/// `quelch instance` subcommands.
#[derive(clap::Subcommand, Debug)]
pub enum InstanceCommand {
    /// List instances declared in the master config.
    List,

    /// Emit a per-instance config file (slimmed slice of the master).
    ///
    /// The emitted YAML contains only the configuration the named instance
    /// needs at runtime: control-plane fields (`subscription_id`,
    /// `resource_group`, `account`) and the `ai:` block are always stripped;
    /// `search:` is stripped for ingest instances; `source_connections` are
    /// stripped for MCP instances and limited to the referenced ones for
    /// ingest instances.
    Config {
        /// Instance name from `quelch.yaml`.
        name: String,
        /// Sanity-check that the instance has the expected kind.
        ///
        /// Errors out if the instance's actual kind in the config does not
        /// match this flag — guards against a typo in the instance name
        /// dispatching the wrong slice to the wrong host.
        #[arg(long, value_enum)]
        kind: InstanceKindArg,
        /// Write to this path instead of stdout.
        #[arg(long)]
        output: Option<PathBuf>,
    },
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
