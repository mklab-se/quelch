//! quelch simulator — runs the real engine against an in-process fake world.

pub mod confluence_gen;
pub mod embedder;
pub mod jira_gen;
pub mod opts;
pub mod scheduler;
pub mod world;

use anyhow::Result;

pub use opts::SimOpts;

/// Entry point. Filled in by subsequent tasks.
pub async fn run(_opts: SimOpts) -> Result<()> {
    anyhow::bail!("sim::run not yet implemented");
}
