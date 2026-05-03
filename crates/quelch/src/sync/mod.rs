// TODO(quelch v2 phase 3+): re-enable when ingest engine lands.
//
// This module (v1 sync engine) is stubbed for the v2 config layer work (Phase 1).
// It will be replaced by the new `ingest/` module in Phase 3.

pub mod embedder;
pub mod phases;
pub mod state;

use tokio::sync::mpsc;

/// Commands the TUI sends back to the engine.
#[derive(Debug, Clone)]
pub enum UiCommand {
    Pause,
    Resume,
    SyncNow,
    ResetCursor {
        source: String,
        subsource: Option<String>,
    },
    PurgeNow {
        source: String,
    },
    Shutdown,
}

/// Outcome of a command-poll tick or one iteration of the engine loop.
#[derive(Debug)]
pub enum EngineOutcome {
    Continue,
    Shutdown,
    ResetCursor {
        source: String,
        subsource: Option<String>,
    },
}

/// Controls how missing indexes are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexMode {
    Interactive,
    AutoCreate,
    RequireExisting,
}

/// Build a never-firing command channel for plain-log runs.
pub fn never_command_channel() -> (mpsc::Sender<UiCommand>, mpsc::Receiver<UiCommand>) {
    mpsc::channel(1)
}

/// Stub — will be replaced in Phase 3.
pub fn load_embedding_config() -> anyhow::Result<crate::azure::schema::EmbeddingConfig> {
    anyhow::bail!("v1 sync engine is disabled in v2; use quelch ingest instead")
}

/// Stub — will be replaced in Phase 3.
pub async fn setup_indexes(
    _config: &crate::config::Config,
    _embedding: &crate::azure::schema::EmbeddingConfig,
    _mode: IndexMode,
) -> anyhow::Result<Vec<String>> {
    anyhow::bail!("v1 sync engine is disabled in v2; use quelch ingest instead")
}

/// Stub — will be replaced in Phase 3.
pub async fn reset_indexes(
    _config: &crate::config::Config,
    _state_path: &std::path::Path,
) -> anyhow::Result<Vec<String>> {
    anyhow::bail!("v1 sync engine is disabled in v2; use quelch ingest instead")
}

/// Stub — will be replaced in Phase 3.
pub async fn run_sync(
    _config: &crate::config::Config,
    _state_path: &std::path::Path,
    _embedding: &crate::azure::schema::EmbeddingConfig,
    _mode: IndexMode,
    _embedder: Option<&dyn embedder::Embedder>,
    _max_docs: Option<u64>,
) -> anyhow::Result<()> {
    anyhow::bail!("v1 sync engine is disabled in v2; use quelch ingest instead")
}

/// Stub — will be replaced in Phase 3.
#[allow(clippy::too_many_arguments)]
pub async fn run_sync_with(
    _config: &crate::config::Config,
    _state_path: &std::path::Path,
    _embedding: &crate::azure::schema::EmbeddingConfig,
    _mode: IndexMode,
    _embedder: Option<&dyn embedder::Embedder>,
    _max_docs: Option<u64>,
    _cmd_rx: &mut mpsc::Receiver<UiCommand>,
    _cycle: u64,
) -> anyhow::Result<EngineOutcome> {
    anyhow::bail!("v1 sync engine is disabled in v2; use quelch ingest instead")
}

/// Stub — will be replaced in Phase 3.
pub async fn run_purge(_config: &crate::config::Config) -> anyhow::Result<()> {
    anyhow::bail!("v1 sync engine is disabled in v2; use quelch ingest instead")
}
