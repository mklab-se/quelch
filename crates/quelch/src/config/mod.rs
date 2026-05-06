/// Quelch configuration: schema, loader, and validation.
///
/// Entry point: [`load_config`].
pub mod data_sources;
pub mod env;
pub mod schema;
pub mod slice;
pub mod validate;

pub use schema::*;

use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file '{path}': {source}")]
    ReadFile {
        path: String,
        source: std::io::Error,
    },

    #[error("invalid YAML in config file: {0}")]
    InvalidYaml(#[from] serde_yaml::Error),

    #[error("environment variable error: {0}")]
    EnvVar(#[from] env::EnvVarError),

    #[error("validation: {0}")]
    Validation(#[from] validate::ValidationError),
}

pub fn load_config(path: &Path) -> Result<Config, ConfigError> {
    let raw = std::fs::read_to_string(path).map_err(|e| ConfigError::ReadFile {
        path: path.display().to_string(),
        source: e,
    })?;
    let expanded = env::substitute_env_vars(&raw)?;
    let config: Config = serde_yaml::from_str(&expanded)?;
    validate::validate(&config)?;
    Ok(config)
}
