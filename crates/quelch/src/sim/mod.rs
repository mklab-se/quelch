//! quelch simulator — runs the real engine against an in-process fake world.

pub mod opts;

use anyhow::Result;

pub use opts::SimOpts;

/// Entry point. Filled in by subsequent tasks.
pub async fn run(_opts: SimOpts) -> Result<()> {
    anyhow::bail!("sim::run not yet implemented");
}
