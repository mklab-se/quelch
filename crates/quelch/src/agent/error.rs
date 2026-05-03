//! Error types for agent bundle generation.

use thiserror::Error;

/// Errors that can occur during bundle construction or target writing.
#[derive(Debug, Error)]
pub enum BundleError {
    /// The named deployment was not found in the config.
    #[error("deployment '{0}' not found in config")]
    DeploymentNotFound(String),

    /// The named deployment is not an MCP deployment.
    #[error("deployment '{0}' is not an MCP deployment")]
    NotMcpDeployment(String),

    /// The MCP deployment has no public URL configured.
    #[error("deployment '{0}' has no public URL — set azure.container_app.url or pass --url")]
    NoPublicUrl(String),
}

/// Errors that can occur when writing a bundle to disk.
#[derive(Debug, Error)]
pub enum TargetError {
    /// An I/O error occurred while writing output files.
    #[error("I/O error writing bundle: {0}")]
    Io(#[from] std::io::Error),
}
