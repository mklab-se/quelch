pub mod env;

use serde::Deserialize;
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

    #[error("validation error: {0}")]
    Validation(String),
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub azure: AzureConfig,
    pub sources: Vec<SourceConfig>,
    #[serde(default)]
    pub sync: SyncConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AzureConfig {
    pub endpoint: String,
    pub api_key: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum SourceConfig {
    #[serde(rename = "jira")]
    Jira(JiraSourceConfig),
    #[serde(rename = "confluence")]
    Confluence(ConfluenceSourceConfig),
}

impl SourceConfig {
    pub fn name(&self) -> &str {
        match self {
            SourceConfig::Jira(j) => &j.name,
            SourceConfig::Confluence(c) => &c.name,
        }
    }

    pub fn index(&self) -> &str {
        match self {
            SourceConfig::Jira(j) => &j.index,
            SourceConfig::Confluence(c) => &c.index,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct JiraSourceConfig {
    pub name: String,
    pub url: String,
    pub auth: AuthConfig,
    pub projects: Vec<String>,
    pub index: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ConfluenceSourceConfig {
    pub name: String,
    pub url: String,
    pub auth: AuthConfig,
    pub spaces: Vec<String>,
    pub index: String,
}

/// Auth configuration supporting both Cloud and Data Center deployments.
///
/// Cloud (Jira/Confluence): Basic Auth with email + API token
///   auth:
///     email: "user@example.com"
///     api_token: "${JIRA_API_TOKEN}"
///
/// Data Center: Bearer PAT
///   auth:
///     pat: "${JIRA_PAT}"
#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum AuthConfig {
    /// Cloud authentication: Basic Auth with email and API token.
    Cloud { email: String, api_token: String },
    /// Data Center authentication: Personal Access Token (Bearer).
    DataCenter { pat: String },
}

impl AuthConfig {
    /// Build the Authorization header value for this auth config.
    pub fn authorization_header(&self) -> String {
        use base64::Engine;
        match self {
            AuthConfig::Cloud { email, api_token } => {
                let credentials = format!("{email}:{api_token}");
                let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
                format!("Basic {encoded}")
            }
            AuthConfig::DataCenter { pat } => {
                format!("Bearer {pat}")
            }
        }
    }

    /// Returns true if this is a Cloud auth config.
    pub fn is_cloud(&self) -> bool {
        matches!(self, AuthConfig::Cloud { .. })
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct SyncConfig {
    #[serde(default = "default_poll_interval")]
    pub poll_interval: u64,
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_per_credential: usize,
    #[serde(default = "default_state_file")]
    pub state_file: String,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            poll_interval: default_poll_interval(),
            batch_size: default_batch_size(),
            max_concurrent_per_credential: default_max_concurrent(),
            state_file: default_state_file(),
        }
    }
}

fn default_poll_interval() -> u64 {
    300
}
fn default_batch_size() -> usize {
    100
}
fn default_max_concurrent() -> usize {
    3
}
fn default_state_file() -> String {
    ".quelch-state.json".to_string()
}

/// Load config from a YAML file, substituting environment variables.
pub fn load_config(path: &Path) -> Result<Config, ConfigError> {
    let raw = std::fs::read_to_string(path).map_err(|e| ConfigError::ReadFile {
        path: path.display().to_string(),
        source: e,
    })?;

    let expanded = env::substitute_env_vars(&raw)?;
    let config: Config = serde_yaml::from_str(&expanded)?;
    validate(&config)?;
    Ok(config)
}

fn validate(config: &Config) -> Result<(), ConfigError> {
    if config.azure.endpoint.is_empty() {
        return Err(ConfigError::Validation(
            "azure.endpoint must not be empty".to_string(),
        ));
    }
    if config.azure.api_key.is_empty() {
        return Err(ConfigError::Validation(
            "azure.api_key must not be empty".to_string(),
        ));
    }
    if config.sources.is_empty() {
        return Err(ConfigError::Validation(
            "at least one source must be configured".to_string(),
        ));
    }
    for source in &config.sources {
        if source.index().is_empty() {
            return Err(ConfigError::Validation(format!(
                "source '{}' must have an index",
                source.name()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_config(yaml: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        f
    }

    #[test]
    fn loads_cloud_auth_config() {
        unsafe {
            std::env::set_var("QUELCH_TEST_KEY", "test-api-key");
            std::env::set_var("QUELCH_TEST_EMAIL", "user@example.com");
            std::env::set_var("QUELCH_TEST_TOKEN", "cloud-token");
        }

        let yaml = r#"
azure:
  endpoint: "https://test.search.windows.net"
  api_key: "${QUELCH_TEST_KEY}"
sources:
  - type: jira
    name: "cloud-jira"
    url: "https://mycompany.atlassian.net"
    auth:
      email: "${QUELCH_TEST_EMAIL}"
      api_token: "${QUELCH_TEST_TOKEN}"
    projects:
      - "PROJ"
    index: "jira-issues"
"#;
        let f = write_config(yaml);
        let config = load_config(f.path()).unwrap();

        assert_eq!(config.azure.api_key, "test-api-key");
        if let SourceConfig::Jira(jira) = &config.sources[0] {
            match &jira.auth {
                AuthConfig::Cloud { email, api_token } => {
                    assert_eq!(email, "user@example.com");
                    assert_eq!(api_token, "cloud-token");
                }
                _ => panic!("expected Cloud auth"),
            }
        }
    }

    #[test]
    fn loads_datacenter_auth_config() {
        unsafe {
            std::env::set_var("QUELCH_TEST_KEY2", "test-api-key");
            std::env::set_var("QUELCH_TEST_PAT", "dc-pat-token");
        }

        let yaml = r#"
azure:
  endpoint: "https://test.search.windows.net"
  api_key: "${QUELCH_TEST_KEY2}"
sources:
  - type: jira
    name: "dc-jira"
    url: "https://jira.internal.company.com"
    auth:
      pat: "${QUELCH_TEST_PAT}"
    projects:
      - "HR"
    index: "jira-issues"
"#;
        let f = write_config(yaml);
        let config = load_config(f.path()).unwrap();

        if let SourceConfig::Jira(jira) = &config.sources[0] {
            match &jira.auth {
                AuthConfig::DataCenter { pat } => {
                    assert_eq!(pat, "dc-pat-token");
                }
                _ => panic!("expected DataCenter auth"),
            }
        }
    }

    #[test]
    fn auth_header_cloud() {
        let auth = AuthConfig::Cloud {
            email: "user@test.com".to_string(),
            api_token: "token123".to_string(),
        };
        let header = auth.authorization_header();
        assert!(header.starts_with("Basic "));
    }

    #[test]
    fn auth_header_datacenter() {
        let auth = AuthConfig::DataCenter {
            pat: "my-pat".to_string(),
        };
        assert_eq!(auth.authorization_header(), "Bearer my-pat");
    }

    #[test]
    fn validates_empty_endpoint() {
        unsafe { std::env::set_var("QUELCH_TEST_PAT_V", "pat") };
        let yaml = r#"
azure:
  endpoint: ""
  api_key: "key"
sources:
  - type: jira
    name: "test"
    url: "https://jira.example.com"
    auth:
      pat: "${QUELCH_TEST_PAT_V}"
    projects: ["X"]
    index: "idx"
"#;
        let f = write_config(yaml);
        let err = load_config(f.path()).unwrap_err();
        assert!(err.to_string().contains("endpoint"));
    }

    #[test]
    fn validates_no_sources() {
        let yaml = r#"
azure:
  endpoint: "https://test.search.windows.net"
  api_key: "key"
sources: []
"#;
        let f = write_config(yaml);
        let err = load_config(f.path()).unwrap_err();
        assert!(err.to_string().contains("at least one source"));
    }

    #[test]
    fn loads_with_sync_overrides() {
        unsafe { std::env::set_var("QUELCH_TEST_PAT_S", "pat") };
        let yaml = r#"
azure:
  endpoint: "https://test.search.windows.net"
  api_key: "key"
sources:
  - type: jira
    name: "test"
    url: "https://jira.example.com"
    auth:
      pat: "${QUELCH_TEST_PAT_S}"
    projects: ["X"]
    index: "idx"
sync:
  poll_interval: 60
  batch_size: 50
  state_file: "custom-state.json"
"#;
        let f = write_config(yaml);
        let config = load_config(f.path()).unwrap();
        assert_eq!(config.sync.poll_interval, 60);
        assert_eq!(config.sync.batch_size, 50);
        assert_eq!(config.sync.state_file, "custom-state.json");
    }

    #[test]
    fn defaults_for_sync() {
        unsafe { std::env::set_var("QUELCH_TEST_PAT_D", "pat") };
        let yaml = r#"
azure:
  endpoint: "https://test.search.windows.net"
  api_key: "key"
sources:
  - type: jira
    name: "test"
    url: "https://jira.example.com"
    auth:
      pat: "${QUELCH_TEST_PAT_D}"
    projects: ["X"]
    index: "idx"
"#;
        let f = write_config(yaml);
        let config = load_config(f.path()).unwrap();
        assert_eq!(config.sync.batch_size, 100);
        assert_eq!(config.sync.poll_interval, 300);
        assert_eq!(config.sync.state_file, ".quelch-state.json");
    }
}
