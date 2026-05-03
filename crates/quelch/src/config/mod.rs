/// Quelch v2 configuration: schema, loader, and validation.
///
/// Entry point: [`load_config`].
pub mod env;
pub mod schema;
pub mod validate;

pub use schema::*;

use std::path::Path;
use thiserror::Error;

/// Errors that can occur while loading or validating a config file.
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
    Validation(String),
}

/// Load, env-substitute, and validate a `quelch.yaml` file.
///
/// # Errors
/// Returns [`ConfigError`] on I/O failure, YAML parse error, missing env vars,
/// or any validation rule violation.
pub fn load_config(path: &Path) -> Result<Config, ConfigError> {
    let raw = std::fs::read_to_string(path).map_err(|e| ConfigError::ReadFile {
        path: path.display().to_string(),
        source: e,
    })?;
    let expanded = env::substitute_env_vars(&raw)?;
    let config: Config = serde_yaml::from_str(&expanded)?;
    validate::run(&config)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn loads_with_env_substitution() {
        unsafe {
            std::env::set_var("Q_TEST_SUB", "subA");
        }
        let yaml = r#"
azure:
  subscription_id: "${Q_TEST_SUB}"
  resource_group: "rg"
  region: "swedencentral"
cosmos:
  database: "quelch"
openai:
  endpoint: "https://x.openai.azure.com"
  embedding_deployment: "te"
  embedding_dimensions: 1536
sources: []
deployments: []
"#;
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        let cfg = load_config(f.path()).unwrap();
        assert_eq!(cfg.azure.subscription_id, "subA");
    }

    #[test]
    fn loads_minimal_fixture() {
        let path = std::path::Path::new("tests/fixtures/quelch.minimal.yaml");
        // The fixture uses no env vars so we can load it directly.
        let raw = std::fs::read_to_string(path).expect("fixture must exist");
        let cfg: Config = serde_yaml::from_str(&raw).expect("fixture must parse");
        validate::run(&cfg).expect("fixture must be valid");
        assert!(!cfg.azure.subscription_id.is_empty());
    }
}
