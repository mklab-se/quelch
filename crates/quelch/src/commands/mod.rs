//! Operator CLI command implementations.
//!
//! Each module corresponds to a top-level subcommand that an operator runs
//! (e.g. `quelch status`, `quelch reset`, `quelch query`).

pub mod get;
pub mod mcp_key;
pub mod query;
pub mod reset;
pub mod search;
pub mod status;
