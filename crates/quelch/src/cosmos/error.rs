use thiserror::Error;

/// Errors returned by `CosmosBackend` implementations.
#[derive(Debug, Error)]
pub enum CosmosError {
    /// A document or request failed basic validation (missing id, wrong shape, etc.).
    #[error("validation: {0}")]
    Validation(String),

    /// A required document was not found (used by operations that must exist).
    #[error("not found: {0}")]
    NotFound(String),

    /// The operation is not supported by this backend implementation.
    #[error("unsupported: {0}")]
    Unsupported(String),

    /// A backend-specific error (network failure, SDK error, etc.).
    #[error("backend: {0}")]
    Backend(String),

    /// JSON serialisation / deserialisation error.
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
}
