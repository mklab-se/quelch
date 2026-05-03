//! MCP (Model Context Protocol) server module.
//!
//! This module houses the MCP server implementation, including:
//! - Filter grammar parser and translators (Tasks 6.1–6.3)
//! - Tool definitions (Tasks 6.4–6.5)
//! - HTTP transport (future)

pub mod error;
pub mod expose;
pub mod filter;
pub mod schema;
pub mod tools;
