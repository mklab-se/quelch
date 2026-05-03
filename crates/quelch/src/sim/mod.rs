// TODO(quelch v2 phase 3+): re-enable when ingest engine + dev mode land.
//
// The v1 simulator is stubbed for the v2 config layer work (Phase 1).
// It will be replaced by `quelch dev` in Phase 3/4.

pub mod azure_faults;
pub mod confluence_gen;
pub mod embedder;
pub mod jira_gen;
pub mod opts;
pub mod scheduler;
pub mod world;

use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use anyhow::Result;
use tokio::sync::mpsc;

pub use opts::SimOpts;

use crate::tui::events::QuelchEvent;

/// Bundle the event receiver + drops counter that the TUI needs.
pub type TuiInputs = (mpsc::Receiver<QuelchEvent>, Arc<AtomicU64>);

pub const MOCK_PAT: &str = "mock-pat-token";

/// Stub — will be replaced in Phase 3/4 when `quelch dev` lands.
pub async fn run(_opts: SimOpts, _tui_inputs: Option<TuiInputs>) -> Result<()> {
    anyhow::bail!("quelch sim is not available in v2; use `quelch dev` instead (Phase 3/4)")
}
