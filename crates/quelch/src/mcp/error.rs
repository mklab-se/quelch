//! Unified error type for MCP tool handlers.

use thiserror::Error;

/// Errors returned by MCP tool implementations.
///
/// These map directly to the error codes documented in `docs/mcp-api.md`.
#[derive(Debug, Error)]
pub enum McpError {
    /// The requested resource was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// The caller referenced a data source not exposed by this deployment.
    #[error("forbidden: data source '{0}' is not exposed by this deployment")]
    Forbidden(String),

    /// The request contained an invalid argument.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// Authentication is required but was not provided or is invalid.
    #[error("unauthenticated: {0}")]
    Unauthenticated(String),

    /// The backend is temporarily unavailable.
    #[error("backend unavailable: {0}")]
    Unavailable(String),

    /// An unexpected internal error occurred.
    #[error("internal: {0}")]
    Internal(String),

    /// A filter parsing or translation error.
    #[error("filter: {0}")]
    Filter(#[from] crate::mcp::filter::FilterError),

    /// A Cosmos DB backend error.
    #[error("cosmos: {0}")]
    Cosmos(#[from] crate::cosmos::CosmosError),
}
