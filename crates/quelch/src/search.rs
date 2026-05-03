// TODO(quelch v2 phase 3+): re-enable when MCP search tool lands.
//
// The v1 search command (`quelch search`) is disabled in v2.
// It will be replaced by `quelch mcp` tool in Phase 5.

use anyhow::Result;

/// Stub — will be replaced in Phase 5.
pub async fn run_search(
    _config: &crate::config::Config,
    _query: &str,
    _index_filter: Option<&str>,
    _top: usize,
    _json_output: bool,
) -> Result<()> {
    anyhow::bail!("quelch search is not available in v2; use `quelch mcp` instead (Phase 5)")
}
