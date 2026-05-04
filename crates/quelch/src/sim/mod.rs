// Activity simulator. Wired up by `quelch dev` (see `crates/quelch/src/dev/`),
// which uses the per-source generators (`jira_gen`, `confluence_gen`, etc.) to
// drive the local mock servers. There is no longer a standalone `quelch sim`
// command — `quelch dev` is the entry point.

pub mod azure_faults;
pub mod confluence_gen;
pub mod jira_gen;
pub mod opts;
pub mod scheduler;
pub mod world;

pub use opts::SimOpts;

/// PAT bearer token the mock servers accept. Used by the simulator's
/// generators when issuing requests to the mock Jira / Confluence endpoints.
pub const MOCK_PAT: &str = "mock-pat-token";
