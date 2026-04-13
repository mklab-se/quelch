use ailloy::config::Config;
use ailloy::config_tui;
use anyhow::Result;

const APP_NAME: &str = "quelch";

#[derive(clap::Subcommand)]
pub enum AiCommands {
    /// Test embedding model connectivity
    Test,
    /// Enable AI features
    Enable,
    /// Disable AI features
    Disable,
    /// Configure AI embedding model interactively
    Config,
    /// Show AI configuration status
    Status,
}

pub async fn run(cmd: Option<AiCommands>) -> Result<()> {
    match cmd {
        None => config_tui::print_ai_status(APP_NAME, &["embedding"]),
        Some(AiCommands::Test) => {
            let client = ailloy::Client::for_capability("embedding")?;
            let response = client.embed_one("Quelch test embedding").await?;
            println!("Embedding test successful ({} dimensions)", response.len());
            Ok(())
        }
        Some(AiCommands::Enable) => config_tui::enable_ai(APP_NAME),
        Some(AiCommands::Disable) => config_tui::disable_ai(APP_NAME),
        Some(AiCommands::Config) => {
            let mut config = Config::load_global()?;
            config_tui::run_interactive_config(&mut config, &["embedding"]).await?;
            Ok(())
        }
        Some(AiCommands::Status) => config_tui::print_ai_status(APP_NAME, &["embedding"]),
    }
}

/// Check if AI (embedding) is configured and active.
pub fn is_ai_active() -> bool {
    config_tui::is_ai_active(APP_NAME)
}

/// Get embedding metadata from the configured ailloy node.
/// Returns None if no embedding model is configured.
pub fn embedding_metadata() -> Option<ailloy::EmbeddingMetadata> {
    let config = ailloy::config::Config::load().ok()?;
    let (_id, node) = config.default_node_for("embedding").ok()?;
    Some(node.embedding_metadata())
}
