// TODO(quelch v2 phase 10.2): re-enable when `quelch dev` lands.
//
// The v1 simulator is stubbed for the v2 config layer work (Phase 1).
// It will be replaced by `quelch dev` in Phase 10.2.

pub mod azure_faults;
pub mod confluence_gen;
pub mod jira_gen;
pub mod opts;
pub mod scheduler;
pub mod world;

use anyhow::Result;

pub use opts::SimOpts;

pub const MOCK_PAT: &str = "mock-pat-token";

/// Stub — will be replaced in Phase 10.2 when `quelch dev` lands.
pub async fn run(_opts: SimOpts) -> Result<()> {
    anyhow::bail!("quelch sim is not available in v2; use `quelch dev` instead (Phase 10.2)")
}
