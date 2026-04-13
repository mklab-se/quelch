# Core Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the core engine so `quelch sync` can read a YAML config, connect to Jira, fetch issues, and push them into Azure AI Search — a working end-to-end pipeline. All external APIs are mocked for testing.

**Architecture:** Config loads YAML with env var substitution and supports both Cloud (Basic Auth) and Data Center (Bearer PAT) auth modes. Azure client talks to Azure AI Search REST API (create index, push docs, query). Jira connector implements `SourceConnector` trait to fetch issues via JQL with correct auth per deployment type. Sync engine orchestrates: load state → fetch changes → transform → push → persist state. All integration tests use `wiremock` mock servers.

**Tech Stack:** Rust 2024, tokio, reqwest, serde/serde_yaml, clap, chrono, thiserror/anyhow, shellexpand, tracing, wiremock (testing)

---

## API Research Summary

### Jira REST API v2
- **Cloud auth:** Basic Auth with `email:api_token` (NOT Bearer)
- **Data Center auth:** Bearer PAT (`Authorization: Bearer <token>`)
- **Search:** `GET /rest/api/2/search?jql=...&startAt=0&maxResults=50&fields=...`
- **Response:** `{ startAt, maxResults, total, issues: [{ id, key, fields: {...} }] }`
- **Timestamps:** `"2024-04-10T14:33:21.872+0000"` (ISO 8601 with millis + offset)
- **Comments:** Paginated sub-object at `fields.comment.comments[]` — must explicitly request `comment` field
- **Cloud maxResults cap:** 100; Data Center: ~1000

### Confluence REST API v1 (CQL search)
- **Cloud:** `GET /wiki/rest/api/search?cql=...&limit=25&expand=body.storage,version,ancestors,metadata.labels`
- **Data Center:** `GET /confluence/rest/api/content/search?cql=...&limit=25&expand=body.storage,version,ancestors,metadata.labels`
- **Cloud auth:** Basic Auth with `email:api_token`
- **DC auth:** Bearer PAT
- **Body content:** Must expand with `body.storage` — not returned by default
- **Pagination:** Cursor-based via `_links.next`
- **CQL date format:** `"yyyy-MM-dd"` or `"yyyy-MM-dd HH:mm"`

### Azure AI Search REST API (2024-07-01)
- **Auth:** `api-key` header
- **Create index:** `POST /indexes?api-version=2024-07-01` → 201 Created, 409 if exists
- **Check index:** `GET /indexes/{name}?api-version=2024-07-01` → 200 or 404
- **Push docs:** `POST /indexes/{name}/docs/index?api-version=2024-07-01` with `@search.action: "mergeOrUpload"`
- **Batch limit:** 1000 docs or 16 MB per request
- **Response:** Per-document status in `value[]` with `key`, `status`, `statusCode`, `errorMessage`
- **Delete:** Same endpoint with `@search.action: "delete"` — idempotent (200 even if key doesn't exist)

---

## File Structure

```
crates/quelch/src/
├── main.rs              # Modified: wire CLI to real commands
├── cli.rs               # Create: CLI arg definitions extracted from main
├── config/
│   ├── mod.rs           # Create: Config types + loading + validation
│   └── env.rs           # Create: Env var substitution
├── sources/
│   ├── mod.rs           # Create: SourceConnector trait + SourceDocument type
│   └── jira.rs          # Create: Jira connector implementation
├── azure/
│   ├── mod.rs           # Create: Azure AI Search client
│   └── schema.rs        # Create: Index schema definitions
├── sync/
│   ├── mod.rs           # Create: Sync engine orchestration
│   └── state.rs         # Create: State persistence (cursors)
└── transform.rs         # Create: Source document → Azure doc mapping

tests/
├── mock_jira.rs         # Create: Wiremock-based Jira mock server
├── mock_azure.rs        # Create: Wiremock-based Azure AI Search mock server
├── jira_sync_test.rs    # Create: End-to-end Jira → Azure sync test
└── config_test.rs       # Create: Config loading integration tests
```

---

### Task 1: Config Types and Env Var Substitution

**Files:**
- Create: `crates/quelch/src/config/env.rs`
- Create: `crates/quelch/src/config/mod.rs`
- Modify: `crates/quelch/Cargo.toml` (add dependencies)
- Modify: `Cargo.toml` (add workspace deps)

- [ ] **Step 1: Add dependencies to crate Cargo.toml**

Replace the `[dependencies]` section in `crates/quelch/Cargo.toml` with:

```toml
[dependencies]
clap.workspace = true
anyhow.workspace = true
thiserror.workspace = true
serde.workspace = true
serde_json.workspace = true
serde_yaml.workspace = true
chrono.workspace = true
tokio.workspace = true
reqwest.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
shellexpand.workspace = true
dirs.workspace = true

[dev-dependencies]
tempfile = "3"
wiremock = "0.6"
```

Also add to workspace `Cargo.toml` `[workspace.dependencies]`:

```toml
# Testing
tempfile = "3"
wiremock = "0.6"
```

- [ ] **Step 2: Create env.rs with tests**

Create `crates/quelch/src/config/env.rs`:

```rust
use thiserror::Error;

#[derive(Debug, Error)]
#[error("environment variable '{name}' not set (referenced in config)")]
pub struct EnvVarError {
    pub name: String,
}

/// Substitute `${VAR_NAME}` patterns in a string with environment variable values.
/// Returns an error if any referenced variable is not set.
pub fn substitute_env_vars(input: &str) -> Result<String, EnvVarError> {
    let result = shellexpand::env(input).map_err(|e| EnvVarError {
        name: e.var_name,
    })?;
    Ok(result.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_env_var() {
        std::env::set_var("QUELCH_TEST_VAR", "hello");
        let result = substitute_env_vars("prefix-${QUELCH_TEST_VAR}-suffix").unwrap();
        assert_eq!(result, "prefix-hello-suffix");
    }

    #[test]
    fn returns_error_for_missing_var() {
        std::env::remove_var("QUELCH_MISSING_VAR");
        let result = substitute_env_vars("${QUELCH_MISSING_VAR}");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.name, "QUELCH_MISSING_VAR");
    }

    #[test]
    fn no_substitution_needed() {
        let result = substitute_env_vars("plain string").unwrap();
        assert_eq!(result, "plain string");
    }

    #[test]
    fn multiple_vars() {
        std::env::set_var("QUELCH_A", "one");
        std::env::set_var("QUELCH_B", "two");
        let result = substitute_env_vars("${QUELCH_A}-${QUELCH_B}").unwrap();
        assert_eq!(result, "one-two");
    }
}
```

- [ ] **Step 3: Run tests to verify env.rs**

Run: `cargo test -p quelch config::env -- --nocapture`
Expected: 4 tests pass.

- [ ] **Step 4: Create config/mod.rs with types and loading**

The auth config supports both Cloud (Basic Auth with email+token) and Data Center (Bearer PAT):

Create `crates/quelch/src/config/mod.rs`:

```rust
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
    Cloud {
        email: String,
        api_token: String,
    },
    /// Data Center authentication: Personal Access Token (Bearer).
    DataCenter {
        pat: String,
    },
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
        std::env::set_var("QUELCH_TEST_KEY", "test-api-key");
        std::env::set_var("QUELCH_TEST_EMAIL", "user@example.com");
        std::env::set_var("QUELCH_TEST_TOKEN", "cloud-token");

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
        std::env::set_var("QUELCH_TEST_KEY2", "test-api-key");
        std::env::set_var("QUELCH_TEST_PAT", "dc-pat-token");

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
        std::env::set_var("QUELCH_TEST_PAT_V", "pat");
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
        std::env::set_var("QUELCH_TEST_PAT_S", "pat");
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
        std::env::set_var("QUELCH_TEST_PAT_D", "pat");
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
```

- [ ] **Step 5: Add base64 to workspace dependencies**

Add to workspace `Cargo.toml` `[workspace.dependencies]`:

```toml
# Encoding
base64 = "0.22"
```

Add to `crates/quelch/Cargo.toml` `[dependencies]`:

```toml
base64.workspace = true
```

- [ ] **Step 6: Add mod config to main.rs**

Add `mod config;` to `crates/quelch/src/main.rs` (before `use clap::Parser;`).

- [ ] **Step 7: Run tests**

Run: `cargo test -p quelch config -- --nocapture`
Expected: All 12 tests pass (4 env + 8 config).

- [ ] **Step 8: Quality checks and commit**

Run: `cargo fmt --all && cargo clippy --workspace -- -D warnings`

```bash
git add crates/quelch/src/config/ crates/quelch/src/main.rs crates/quelch/Cargo.toml Cargo.toml Cargo.lock
git commit -m "Add config module with YAML loading, Cloud/DC auth, env var substitution"
```

---

### Task 2: Source Connector Trait and Document Types

**Files:**
- Create: `crates/quelch/src/sources/mod.rs`
- Modify: `crates/quelch/src/main.rs` (add mod)
- Modify: `Cargo.toml` (add trait-variant)

- [ ] **Step 1: Add trait-variant dependency**

Add to workspace `Cargo.toml` `[workspace.dependencies]`:

```toml
# Async traits
trait-variant = "0.1"
```

Add to `crates/quelch/Cargo.toml` `[dependencies]`:

```toml
trait-variant.workspace = true
```

- [ ] **Step 2: Create sources/mod.rs**

Create `crates/quelch/src/sources/mod.rs`:

```rust
pub mod jira;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Cursor for tracking incremental sync position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncCursor {
    /// Timestamp of the last synced document's update time.
    pub last_updated: DateTime<Utc>,
}

/// A document fetched from a source, ready for transformation and indexing.
#[derive(Debug, Clone)]
pub struct SourceDocument {
    /// Unique document ID (e.g., "test-jira-DO-1234").
    pub id: String,
    /// The content fields to index.
    pub fields: HashMap<String, serde_json::Value>,
    /// Timestamp of last modification in source.
    pub updated_at: DateTime<Utc>,
}

/// Result of a fetch operation — documents plus a new cursor position.
pub struct FetchResult {
    pub documents: Vec<SourceDocument>,
    pub cursor: SyncCursor,
    /// True if there are more pages to fetch.
    pub has_more: bool,
}

/// Trait implemented by each source connector (Jira, Confluence, etc.).
#[trait_variant::make(Send)]
pub trait SourceConnector: Sync {
    /// Human-readable source type name.
    fn source_type(&self) -> &str;

    /// The source name from config (used as identifier in state).
    fn source_name(&self) -> &str;

    /// The target Azure AI Search index name.
    fn index_name(&self) -> &str;

    /// Fetch documents changed since the given cursor.
    /// If cursor is None, fetch everything (initial full sync).
    async fn fetch_changes(
        &self,
        cursor: Option<&SyncCursor>,
        batch_size: usize,
    ) -> anyhow::Result<FetchResult>;

    /// Fetch IDs of all documents currently in the source.
    /// Used for detecting deletions.
    async fn fetch_all_ids(&self) -> anyhow::Result<Vec<String>>;
}
```

- [ ] **Step 3: Add mod sources to main.rs**

Add `mod sources;` after `mod config;` in main.rs.

- [ ] **Step 4: Verify it compiles**

Run: `cargo build --workspace`
Expected: Compiles (jira.rs doesn't exist yet — add an empty placeholder or temporarily remove `pub mod jira;` from sources/mod.rs, compile, then re-add it in Task 3).

Actually: create an empty `crates/quelch/src/sources/jira.rs` file for now so the module tree compiles.

- [ ] **Step 5: Commit**

```bash
git add crates/quelch/src/sources/ crates/quelch/src/main.rs crates/quelch/Cargo.toml Cargo.toml Cargo.lock
git commit -m "Add SourceConnector trait and document types"
```

---

### Task 3: Azure AI Search Client

**Files:**
- Create: `crates/quelch/src/azure/mod.rs`
- Create: `crates/quelch/src/azure/schema.rs`
- Modify: `crates/quelch/src/main.rs` (add mod)

- [ ] **Step 1: Create azure/schema.rs with index definitions**

Create `crates/quelch/src/azure/schema.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IndexSchema {
    pub name: String,
    pub fields: Vec<IndexField>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IndexField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub key: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub searchable: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub filterable: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sortable: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub facetable: bool,
}

/// Default index schema for Jira issues.
pub fn jira_index_schema(index_name: &str) -> IndexSchema {
    IndexSchema {
        name: index_name.to_string(),
        fields: vec![
            field("id", "Edm.String", true, false, true, false, false),
            field("source_name", "Edm.String", false, false, true, false, false),
            field("project", "Edm.String", false, false, true, false, true),
            field("issue_key", "Edm.String", false, false, true, false, false),
            field("issue_type", "Edm.String", false, false, true, false, true),
            field("summary", "Edm.String", false, true, false, false, false),
            field("description", "Edm.String", false, true, false, false, false),
            field("status", "Edm.String", false, false, true, false, true),
            field("priority", "Edm.String", false, false, true, false, true),
            field("assignee", "Edm.String", false, false, true, false, true),
            field("reporter", "Edm.String", false, false, true, false, false),
            field("labels", "Collection(Edm.String)", false, false, true, false, true),
            field("comments", "Edm.String", false, true, false, false, false),
            field("content", "Edm.String", false, true, false, false, false),
            field("created_at", "Edm.DateTimeOffset", false, false, true, true, false),
            field("updated_at", "Edm.DateTimeOffset", false, false, true, true, false),
        ],
    }
}

fn field(
    name: &str,
    field_type: &str,
    key: bool,
    searchable: bool,
    filterable: bool,
    sortable: bool,
    facetable: bool,
) -> IndexField {
    IndexField {
        name: name.to_string(),
        field_type: field_type.to_string(),
        key,
        searchable,
        filterable,
        sortable,
        facetable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jira_schema_has_correct_key() {
        let schema = jira_index_schema("test-index");
        assert_eq!(schema.name, "test-index");
        let key_field = schema.fields.iter().find(|f| f.key).unwrap();
        assert_eq!(key_field.name, "id");
        assert_eq!(key_field.field_type, "Edm.String");
    }

    #[test]
    fn jira_schema_has_searchable_content() {
        let schema = jira_index_schema("test");
        let content = schema.fields.iter().find(|f| f.name == "content").unwrap();
        assert!(content.searchable);
        assert!(!content.filterable);
    }

    #[test]
    fn jira_schema_serializes_to_json() {
        let schema = jira_index_schema("test");
        let json = serde_json::to_string(&schema).unwrap();
        assert!(json.contains("\"key\":true"));
        assert!(json.contains("\"type\":\"Edm.String\""));
    }
}
```

- [ ] **Step 2: Create azure/mod.rs with the Search client**

Create `crates/quelch/src/azure/mod.rs`:

```rust
pub mod schema;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, warn};

use self::schema::IndexSchema;

const API_VERSION: &str = "2024-07-01";
const MAX_RETRY_ATTEMPTS: u32 = 3;

#[derive(Debug, Error)]
pub enum AzureError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Azure API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Client for Azure AI Search REST API.
pub struct SearchClient {
    client: Client,
    endpoint: String,
    api_key: String,
}

#[derive(Debug, Serialize)]
struct IndexBatch {
    value: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    value: Vec<serde_json::Value>,
    #[serde(rename = "@odata.nextLink")]
    #[allow(dead_code)]
    next_link: Option<String>,
}

impl SearchClient {
    pub fn new(endpoint: &str, api_key: &str) -> Self {
        let endpoint = endpoint.trim_end_matches('/').to_string();
        Self {
            client: Client::new(),
            endpoint,
            api_key: api_key.to_string(),
        }
    }

    /// Create an index if it doesn't already exist.
    pub async fn ensure_index(&self, schema: &IndexSchema) -> Result<(), AzureError> {
        // Check if index exists (GET returns 200 or 404)
        let url = format!(
            "{}/indexes/{}?api-version={}",
            self.endpoint, schema.name, API_VERSION
        );

        let resp = self
            .client
            .get(&url)
            .header("api-key", &self.api_key)
            .send()
            .await?;

        if resp.status().is_success() {
            debug!("Index '{}' already exists", schema.name);
            return Ok(());
        }

        // Create index (POST returns 201)
        let create_url = format!(
            "{}/indexes?api-version={}",
            self.endpoint, API_VERSION
        );

        let resp = self
            .client
            .post(&create_url)
            .header("api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(schema)
            .send()
            .await?;

        if resp.status().is_success() {
            debug!("Created index '{}'", schema.name);
            Ok(())
        } else {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            Err(AzureError::Api {
                status,
                message: body,
            })
        }
    }

    /// Push documents to an index using merge-or-upload action.
    pub async fn push_documents(
        &self,
        index_name: &str,
        documents: Vec<serde_json::Value>,
    ) -> Result<(), AzureError> {
        if documents.is_empty() {
            return Ok(());
        }

        let url = format!(
            "{}/indexes/{}/docs/index?api-version={}",
            self.endpoint, index_name, API_VERSION
        );

        let docs_with_action: Vec<serde_json::Value> = documents
            .into_iter()
            .map(|mut doc| {
                if let Some(obj) = doc.as_object_mut() {
                    obj.insert(
                        "@search.action".to_string(),
                        serde_json::Value::String("mergeOrUpload".to_string()),
                    );
                }
                doc
            })
            .collect();

        let batch = IndexBatch {
            value: docs_with_action,
        };

        let resp = self
            .request_with_retry(|| {
                self.client
                    .post(&url)
                    .header("api-key", &self.api_key)
                    .header("Content-Type", "application/json")
                    .json(&batch)
            })
            .await?;

        let status = resp.status();
        if status.is_success() || status.as_u16() == 207 {
            // 200 = all succeeded, 207 = partial success (check per-doc status)
            Ok(())
        } else {
            let status_code = status.as_u16();
            let body = resp.text().await.unwrap_or_default();
            Err(AzureError::Api {
                status: status_code,
                message: body,
            })
        }
    }

    /// Fetch all document IDs from an index (for delete detection).
    pub async fn fetch_all_ids(&self, index_name: &str) -> Result<Vec<String>, AzureError> {
        let mut ids = Vec::new();
        let mut skip: usize = 0;
        let top: usize = 1000;

        loop {
            let url = format!(
                "{}/indexes/{}/docs?api-version={}&search=*&$select=id&$top={}&$skip={}&$orderby=id",
                self.endpoint, index_name, API_VERSION, top, skip
            );

            let resp = self
                .client
                .get(&url)
                .header("api-key", &self.api_key)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                return Err(AzureError::Api {
                    status,
                    message: body,
                });
            }

            let search_resp: SearchResponse = resp.json().await?;
            let batch_len = search_resp.value.len();

            for doc in search_resp.value {
                if let Some(id) = doc.get("id").and_then(|v| v.as_str()) {
                    ids.push(id.to_string());
                }
            }

            if batch_len < top {
                break;
            }
            skip += top;
        }

        Ok(ids)
    }

    /// Delete documents by ID from an index.
    pub async fn delete_documents(
        &self,
        index_name: &str,
        ids: &[String],
    ) -> Result<(), AzureError> {
        if ids.is_empty() {
            return Ok(());
        }

        let url = format!(
            "{}/indexes/{}/docs/index?api-version={}",
            self.endpoint, index_name, API_VERSION
        );

        let docs: Vec<serde_json::Value> = ids
            .iter()
            .map(|id| {
                serde_json::json!({
                    "@search.action": "delete",
                    "id": id
                })
            })
            .collect();

        let batch = IndexBatch { value: docs };

        let resp = self
            .request_with_retry(|| {
                self.client
                    .post(&url)
                    .header("api-key", &self.api_key)
                    .header("Content-Type", "application/json")
                    .json(&batch)
            })
            .await?;

        if resp.status().is_success() {
            Ok(())
        } else {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            Err(AzureError::Api {
                status,
                message: body,
            })
        }
    }

    /// Execute a request with exponential backoff retry on 429/5xx.
    async fn request_with_retry<F>(
        &self,
        build_request: F,
    ) -> Result<reqwest::Response, AzureError>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let mut last_err = None;
        for attempt in 0..MAX_RETRY_ATTEMPTS {
            if attempt > 0 {
                let delay = std::time::Duration::from_secs(1 << attempt);
                warn!(
                    "Retrying after {:?} (attempt {}/{})",
                    delay,
                    attempt + 1,
                    MAX_RETRY_ATTEMPTS
                );
                tokio::time::sleep(delay).await;
            }

            match build_request().send().await {
                Ok(resp) if resp.status() == 429 || resp.status().is_server_error() => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    warn!("Request failed with {}: {}", status, body);
                    last_err = Some(AzureError::Api {
                        status: status.as_u16(),
                        message: body,
                    });
                }
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    warn!("Request error: {}", e);
                    last_err = Some(AzureError::Http(e));
                }
            }
        }
        Err(last_err.unwrap())
    }
}
```

- [ ] **Step 3: Add mod azure to main.rs**

Add `mod azure;` after `mod sources;` in main.rs.

- [ ] **Step 4: Run tests and quality checks**

Run: `cargo test -p quelch azure -- --nocapture`
Expected: 3 schema tests pass.

Run: `cargo fmt --all && cargo clippy --workspace -- -D warnings`

- [ ] **Step 5: Commit**

```bash
git add crates/quelch/src/azure/ crates/quelch/src/main.rs
git commit -m "Add Azure AI Search client with retry and index schema definitions"
```

---

### Task 4: Jira Connector

**Files:**
- Create: `crates/quelch/src/sources/jira.rs`

- [ ] **Step 1: Create jira.rs**

Create `crates/quelch/src/sources/jira.rs`:

```rust
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use tracing::debug;

use super::{FetchResult, SourceConnector, SourceDocument, SyncCursor};
use crate::config::JiraSourceConfig;

pub struct JiraConnector {
    client: Client,
    config: JiraSourceConfig,
}

#[derive(Debug, Deserialize)]
struct JiraSearchResponse {
    issues: Vec<JiraIssue>,
    total: u64,
    #[serde(rename = "startAt")]
    start_at: u64,
    #[serde(rename = "maxResults")]
    #[allow(dead_code)]
    max_results: u64,
}

#[derive(Debug, Deserialize)]
struct JiraIssue {
    key: String,
    fields: JiraFields,
}

#[derive(Debug, Deserialize)]
struct JiraFields {
    summary: Option<String>,
    description: Option<String>,
    status: Option<JiraNamedField>,
    priority: Option<JiraNamedField>,
    assignee: Option<JiraUser>,
    reporter: Option<JiraUser>,
    issuetype: Option<JiraNamedField>,
    labels: Option<Vec<String>>,
    created: Option<String>,
    updated: Option<String>,
    comment: Option<JiraCommentContainer>,
    project: Option<JiraProject>,
}

#[derive(Debug, Deserialize)]
struct JiraNamedField {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JiraUser {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

/// Jira comment field is a paginated sub-object, not a flat array.
#[derive(Debug, Deserialize)]
struct JiraCommentContainer {
    comments: Option<Vec<JiraComment>>,
}

#[derive(Debug, Deserialize)]
struct JiraComment {
    body: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JiraProject {
    key: Option<String>,
}

impl JiraConnector {
    pub fn new(config: &JiraSourceConfig) -> Self {
        Self {
            client: Client::new(),
            config: config.clone(),
        }
    }

    fn build_jql(&self, cursor: Option<&SyncCursor>) -> String {
        let project_clause = self
            .config
            .projects
            .iter()
            .map(|p| format!("project = {p}"))
            .collect::<Vec<_>>()
            .join(" OR ");

        let project_jql = if self.config.projects.len() > 1 {
            format!("({project_clause})")
        } else {
            project_clause
        };

        match cursor {
            Some(c) => {
                let ts = c.last_updated.format("%Y-%m-%d %H:%M");
                format!("{project_jql} AND updated >= \"{ts}\" ORDER BY updated ASC")
            }
            None => format!("{project_jql} ORDER BY updated ASC"),
        }
    }

    /// Parse Jira timestamp format: "2024-04-10T14:33:21.872+0000"
    fn parse_datetime(s: &str) -> Option<DateTime<Utc>> {
        DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.3f%z")
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    }

    fn issue_to_document(&self, issue: &JiraIssue) -> SourceDocument {
        let fields = &issue.fields;

        let summary = fields.summary.clone().unwrap_or_default();
        let description = fields.description.clone().unwrap_or_default();
        let comments_text = fields
            .comment
            .as_ref()
            .and_then(|c| c.comments.as_ref())
            .map(|comments| {
                comments
                    .iter()
                    .filter_map(|c| c.body.as_ref())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n\n")
            })
            .unwrap_or_default();

        let content = format!("{summary}\n\n{description}\n\n{comments_text}");
        let project_key = fields
            .project
            .as_ref()
            .and_then(|p| p.key.clone())
            .unwrap_or_default();

        let updated_at = fields
            .updated
            .as_ref()
            .and_then(|s| Self::parse_datetime(s))
            .unwrap_or_else(Utc::now);

        let created_at = fields
            .created
            .as_ref()
            .and_then(|s| Self::parse_datetime(s))
            .unwrap_or_else(Utc::now);

        let doc_id = format!("{}-{}", self.config.name, issue.key);

        let mut map = HashMap::new();
        map.insert("id".to_string(), serde_json::json!(doc_id));
        map.insert("source_name".to_string(), serde_json::json!(self.config.name));
        map.insert("project".to_string(), serde_json::json!(project_key));
        map.insert("issue_key".to_string(), serde_json::json!(issue.key));
        map.insert(
            "issue_type".to_string(),
            serde_json::json!(fields.issuetype.as_ref().and_then(|t| t.name.as_ref()).unwrap_or(&String::new())),
        );
        map.insert("summary".to_string(), serde_json::json!(summary));
        map.insert("description".to_string(), serde_json::json!(description));
        map.insert(
            "status".to_string(),
            serde_json::json!(fields.status.as_ref().and_then(|s| s.name.as_ref()).unwrap_or(&String::new())),
        );
        map.insert(
            "priority".to_string(),
            serde_json::json!(fields.priority.as_ref().and_then(|p| p.name.as_ref()).unwrap_or(&String::new())),
        );
        map.insert(
            "assignee".to_string(),
            serde_json::json!(fields.assignee.as_ref().and_then(|a| a.display_name.as_ref()).unwrap_or(&String::new())),
        );
        map.insert(
            "reporter".to_string(),
            serde_json::json!(fields.reporter.as_ref().and_then(|r| r.display_name.as_ref()).unwrap_or(&String::new())),
        );
        map.insert("labels".to_string(), serde_json::json!(fields.labels.clone().unwrap_or_default()));
        map.insert("comments".to_string(), serde_json::json!(comments_text));
        map.insert("content".to_string(), serde_json::json!(content));
        map.insert("created_at".to_string(), serde_json::json!(created_at.to_rfc3339()));
        map.insert("updated_at".to_string(), serde_json::json!(updated_at.to_rfc3339()));

        SourceDocument {
            id: doc_id,
            fields: map,
            updated_at,
        }
    }
}

impl SourceConnector for JiraConnector {
    fn source_type(&self) -> &str {
        "jira"
    }

    fn source_name(&self) -> &str {
        &self.config.name
    }

    fn index_name(&self) -> &str {
        &self.config.index
    }

    async fn fetch_changes(
        &self,
        cursor: Option<&SyncCursor>,
        batch_size: usize,
    ) -> Result<FetchResult> {
        let jql = self.build_jql(cursor);
        let url = format!("{}/rest/api/2/search", self.config.url.trim_end_matches('/'));
        let auth_header = self.config.auth.authorization_header();

        debug!(source = self.config.name, jql = jql, "Fetching Jira issues");

        let resp = self
            .client
            .get(&url)
            .header("Authorization", &auth_header)
            .query(&[
                ("jql", jql.as_str()),
                ("maxResults", &batch_size.to_string()),
                ("startAt", "0"),
                ("fields", "summary,description,status,priority,assignee,reporter,issuetype,labels,created,updated,comment,project"),
            ])
            .send()
            .await
            .context("failed to connect to Jira")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Jira API error ({}): {}", status, body);
        }

        let search_resp: JiraSearchResponse =
            resp.json().await.context("failed to parse Jira response")?;

        let documents: Vec<SourceDocument> = search_resp
            .issues
            .iter()
            .map(|issue| self.issue_to_document(issue))
            .collect();

        let new_cursor = documents
            .last()
            .map(|doc| SyncCursor { last_updated: doc.updated_at })
            .or_else(|| cursor.cloned())
            .unwrap_or(SyncCursor { last_updated: Utc::now() });

        let fetched = search_resp.start_at + search_resp.issues.len() as u64;
        let has_more = fetched < search_resp.total;

        debug!(
            source = self.config.name,
            count = documents.len(),
            total = search_resp.total,
            has_more = has_more,
            "Fetched Jira issues"
        );

        Ok(FetchResult {
            documents,
            cursor: new_cursor,
            has_more,
        })
    }

    async fn fetch_all_ids(&self) -> Result<Vec<String>> {
        let mut all_ids = Vec::new();
        let mut start_at: u64 = 0;
        let page_size: usize = 1000;
        let auth_header = self.config.auth.authorization_header();

        loop {
            let project_clause = self
                .config
                .projects
                .iter()
                .map(|p| format!("project = {p}"))
                .collect::<Vec<_>>()
                .join(" OR ");

            let jql = if self.config.projects.len() > 1 {
                format!("({project_clause})")
            } else {
                project_clause
            };

            let url = format!("{}/rest/api/2/search", self.config.url.trim_end_matches('/'));

            let resp = self
                .client
                .get(&url)
                .header("Authorization", &auth_header)
                .query(&[
                    ("jql", jql.as_str()),
                    ("maxResults", &page_size.to_string()),
                    ("startAt", &start_at.to_string()),
                    ("fields", "key"),
                ])
                .send()
                .await
                .context("failed to connect to Jira for ID fetch")?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("Jira API error ({}): {}", status, body);
            }

            let search_resp: JiraSearchResponse = resp.json().await?;
            let batch_len = search_resp.issues.len();

            for issue in &search_resp.issues {
                all_ids.push(format!("{}-{}", self.config.name, issue.key));
            }

            if batch_len == 0 || (start_at + batch_len as u64) >= search_resp.total {
                break;
            }
            start_at += batch_len as u64;
        }

        Ok(all_ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthConfig;

    fn test_config() -> JiraSourceConfig {
        JiraSourceConfig {
            name: "test-jira".to_string(),
            url: "https://jira.example.com".to_string(),
            auth: AuthConfig::DataCenter {
                pat: "fake-pat".to_string(),
            },
            projects: vec!["DO".to_string()],
            index: "jira-issues".to_string(),
        }
    }

    #[test]
    fn builds_jql_without_cursor() {
        let connector = JiraConnector::new(&test_config());
        let jql = connector.build_jql(None);
        assert_eq!(jql, "project = DO ORDER BY updated ASC");
    }

    #[test]
    fn builds_jql_with_cursor() {
        let connector = JiraConnector::new(&test_config());
        let cursor = SyncCursor {
            last_updated: DateTime::parse_from_rfc3339("2025-01-15T10:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
        };
        let jql = connector.build_jql(Some(&cursor));
        assert!(jql.contains("updated >= \"2025-01-15 10:30\""));
        assert!(jql.contains("project = DO"));
    }

    #[test]
    fn builds_jql_multiple_projects() {
        let mut config = test_config();
        config.projects = vec!["DO".to_string(), "HR".to_string()];
        let connector = JiraConnector::new(&config);
        let jql = connector.build_jql(None);
        assert_eq!(jql, "(project = DO OR project = HR) ORDER BY updated ASC");
    }

    #[test]
    fn parses_jira_datetime() {
        let dt = JiraConnector::parse_datetime("2025-01-15T10:30:00.000+0000").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-15T10:30:00+00:00");
    }

    #[test]
    fn parses_jira_datetime_with_offset() {
        let dt = JiraConnector::parse_datetime("2024-03-15T09:12:44.000+1100").unwrap();
        assert_eq!(dt.hour(), 22); // 09:12 +1100 = 22:12 UTC previous day
    }

    #[test]
    fn converts_issue_to_document() {
        let connector = JiraConnector::new(&test_config());
        let issue = JiraIssue {
            key: "DO-42".to_string(),
            fields: JiraFields {
                summary: Some("Fix the bug".to_string()),
                description: Some("It's broken".to_string()),
                status: Some(JiraNamedField { name: Some("Open".to_string()) }),
                priority: Some(JiraNamedField { name: Some("High".to_string()) }),
                assignee: Some(JiraUser { display_name: Some("Alice".to_string()) }),
                reporter: Some(JiraUser { display_name: Some("Bob".to_string()) }),
                issuetype: Some(JiraNamedField { name: Some("Bug".to_string()) }),
                labels: Some(vec!["backend".to_string()]),
                created: Some("2025-01-10T08:00:00.000+0000".to_string()),
                updated: Some("2025-01-15T10:30:00.000+0000".to_string()),
                comment: Some(JiraCommentContainer {
                    comments: Some(vec![JiraComment {
                        body: Some("Looking into it".to_string()),
                    }]),
                }),
                project: Some(JiraProject { key: Some("DO".to_string()) }),
            },
        };

        let doc = connector.issue_to_document(&issue);
        assert_eq!(doc.id, "test-jira-DO-42");
        assert_eq!(doc.fields["issue_key"], "DO-42");
        assert_eq!(doc.fields["status"], "Open");
        assert_eq!(doc.fields["assignee"], "Alice");
        assert!(doc.fields["content"].as_str().unwrap().contains("Fix the bug"));
        assert!(doc.fields["content"].as_str().unwrap().contains("Looking into it"));
    }

    #[test]
    fn handles_null_fields_gracefully() {
        let connector = JiraConnector::new(&test_config());
        let issue = JiraIssue {
            key: "DO-1".to_string(),
            fields: JiraFields {
                summary: None,
                description: None,
                status: None,
                priority: None,
                assignee: None,
                reporter: None,
                issuetype: None,
                labels: None,
                created: None,
                updated: None,
                comment: None,
                project: None,
            },
        };

        let doc = connector.issue_to_document(&issue);
        assert_eq!(doc.id, "test-jira-DO-1");
        assert_eq!(doc.fields["status"], "");
        assert_eq!(doc.fields["assignee"], "");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p quelch sources::jira -- --nocapture`
Expected: 7 tests pass.

- [ ] **Step 3: Quality checks and commit**

Run: `cargo fmt --all && cargo clippy --workspace -- -D warnings`

```bash
git add crates/quelch/src/sources/jira.rs
git commit -m "Add Jira connector with Cloud/DC auth and JQL-based incremental fetching"
```

---

### Task 5: Sync State Persistence

**Files:**
- Create: `crates/quelch/src/sync/state.rs`
- Create: `crates/quelch/src/sync/mod.rs`
- Modify: `crates/quelch/src/main.rs` (add mod)

- [ ] **Step 1: Create sync/state.rs**

Create `crates/quelch/src/sync/state.rs`:

```rust
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
pub struct SyncState {
    pub version: u32,
    pub sources: HashMap<String, SourceState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceState {
    pub last_cursor: Option<DateTime<Utc>>,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub documents_synced: u64,
    pub sync_count: u64,
}

impl Default for SyncState {
    fn default() -> Self {
        Self {
            version: 1,
            sources: HashMap::new(),
        }
    }
}

impl SyncState {
    /// Load state from a JSON file. Returns default state if file doesn't exist.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = std::fs::read_to_string(path).context("failed to read sync state file")?;
        let state: SyncState =
            serde_json::from_str(&data).context("failed to parse sync state file")?;
        Ok(state)
    }

    /// Save state to a JSON file atomically (write to temp, then rename).
    pub fn save(&self, path: &Path) -> Result<()> {
        let data =
            serde_json::to_string_pretty(self).context("failed to serialize sync state")?;
        let tmp_path = path.with_extension("tmp");
        std::fs::write(&tmp_path, &data).context("failed to write sync state temp file")?;
        std::fs::rename(&tmp_path, path).context("failed to rename sync state file")?;
        Ok(())
    }

    /// Get state for a source (returns default if not found).
    pub fn get_source(&self, name: &str) -> SourceState {
        self.sources.get(name).cloned().unwrap_or(SourceState {
            last_cursor: None,
            last_sync_at: None,
            documents_synced: 0,
            sync_count: 0,
        })
    }

    /// Update state for a source after a successful sync batch.
    pub fn update_source(&mut self, name: &str, cursor: DateTime<Utc>, docs_synced: u64) {
        let entry = self.sources.entry(name.to_string()).or_insert(SourceState {
            last_cursor: None,
            last_sync_at: None,
            documents_synced: 0,
            sync_count: 0,
        });
        entry.last_cursor = Some(cursor);
        entry.last_sync_at = Some(Utc::now());
        entry.documents_synced += docs_synced;
        entry.sync_count += 1;
    }

    pub fn reset_source(&mut self, name: &str) {
        self.sources.remove(name);
    }

    pub fn reset_all(&mut self) {
        self.sources.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn default_state_has_no_sources() {
        let state = SyncState::default();
        assert_eq!(state.version, 1);
        assert!(state.sources.is_empty());
    }

    #[test]
    fn load_returns_default_if_file_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");
        let state = SyncState::load(&path).unwrap();
        assert!(state.sources.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");

        let mut state = SyncState::default();
        state.update_source("test", Utc::now(), 42);
        state.save(&path).unwrap();

        let loaded = SyncState::load(&path).unwrap();
        assert_eq!(loaded.version, 1);
        let source = loaded.get_source("test");
        assert!(source.last_cursor.is_some());
        assert_eq!(source.documents_synced, 42);
        assert_eq!(source.sync_count, 1);
    }

    #[test]
    fn update_accumulates_counts() {
        let mut state = SyncState::default();
        state.update_source("s1", Utc::now(), 10);
        state.update_source("s1", Utc::now(), 20);
        let source = state.get_source("s1");
        assert_eq!(source.documents_synced, 30);
        assert_eq!(source.sync_count, 2);
    }

    #[test]
    fn reset_source_removes_it() {
        let mut state = SyncState::default();
        state.update_source("s1", Utc::now(), 10);
        state.reset_source("s1");
        let source = state.get_source("s1");
        assert!(source.last_cursor.is_none());
        assert_eq!(source.documents_synced, 0);
    }

    #[test]
    fn get_source_returns_default_for_unknown() {
        let state = SyncState::default();
        let source = state.get_source("unknown");
        assert!(source.last_cursor.is_none());
        assert_eq!(source.documents_synced, 0);
    }
}
```

- [ ] **Step 2: Create sync/mod.rs with the sync engine**

Create `crates/quelch/src/sync/mod.rs`:

```rust
pub mod state;

use anyhow::{Context, Result};
use std::path::Path;
use tracing::{error, info};

use crate::azure::schema::jira_index_schema;
use crate::azure::SearchClient;
use crate::config::{Config, SourceConfig};
use crate::sources::jira::JiraConnector;
use crate::sources::{SourceConnector, SyncCursor};

use self::state::SyncState;

/// Run a one-shot sync of all configured sources.
pub async fn run_sync(config: &Config, state_path: &Path) -> Result<()> {
    let azure = SearchClient::new(&config.azure.endpoint, &config.azure.api_key);
    let mut state = SyncState::load(state_path)?;

    for source_config in &config.sources {
        if let Err(e) =
            sync_source(&azure, source_config, config, &mut state, state_path).await
        {
            error!(source = source_config.name(), error = %e, "Sync failed for source");
        }
    }

    Ok(())
}

async fn sync_source(
    azure: &SearchClient,
    source_config: &SourceConfig,
    config: &Config,
    state: &mut SyncState,
    state_path: &Path,
) -> Result<()> {
    let connector: Box<dyn SourceConnector> = match source_config {
        SourceConfig::Jira(jira_config) => Box::new(JiraConnector::new(jira_config)),
        SourceConfig::Confluence(_) => {
            anyhow::bail!("Confluence connector not yet implemented");
        }
    };

    let index_name = connector.index_name();
    let source_name = connector.source_name();

    // Ensure index exists with correct schema
    let schema = match source_config {
        SourceConfig::Jira(_) => jira_index_schema(index_name),
        SourceConfig::Confluence(_) => unreachable!(),
    };
    azure
        .ensure_index(&schema)
        .await
        .context("failed to ensure index exists")?;

    // Get cursor from persisted state
    let source_state = state.get_source(source_name);
    let cursor = source_state
        .last_cursor
        .map(|ts| SyncCursor { last_updated: ts });

    let mut total_synced: u64 = 0;

    loop {
        let result = connector
            .fetch_changes(cursor.as_ref(), config.sync.batch_size)
            .await
            .context("failed to fetch changes from source")?;

        let doc_count = result.documents.len() as u64;
        if doc_count == 0 {
            info!(source = source_name, "No changes since last sync");
            break;
        }

        // Convert SourceDocuments to JSON values for Azure
        let azure_docs: Vec<serde_json::Value> = result
            .documents
            .iter()
            .map(|doc| {
                serde_json::Value::Object(
                    doc.fields
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                )
            })
            .collect();

        // Push to Azure AI Search
        azure
            .push_documents(index_name, azure_docs)
            .await
            .context("failed to push documents to Azure AI Search")?;

        total_synced += doc_count;

        // Persist state immediately after each batch (crash safety)
        state.update_source(source_name, result.cursor.last_updated, doc_count);
        state.save(state_path).context("failed to save sync state")?;

        info!(
            source = source_name,
            batch = doc_count,
            total = total_synced,
            "Pushed batch to Azure AI Search"
        );

        if !result.has_more {
            break;
        }
    }

    if total_synced > 0 {
        info!(source = source_name, total = total_synced, "Sync complete");
    }

    Ok(())
}
```

- [ ] **Step 3: Add mod sync to main.rs**

Add `mod sync;` after `mod azure;` in main.rs.

- [ ] **Step 4: Run tests and quality checks**

Run: `cargo test -p quelch sync::state -- --nocapture`
Expected: 6 tests pass.

Run: `cargo fmt --all && cargo clippy --workspace -- -D warnings`

- [ ] **Step 5: Commit**

```bash
git add crates/quelch/src/sync/ crates/quelch/src/main.rs
git commit -m "Add sync engine with state persistence and crash-safe cursor tracking"
```

---

### Task 6: CLI Wiring

**Files:**
- Create: `crates/quelch/src/cli.rs`
- Modify: `crates/quelch/src/main.rs` (complete rewrite)

- [ ] **Step 1: Create cli.rs**

Create `crates/quelch/src/cli.rs`:

```rust
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "quelch",
    version,
    about = "Ingest data directly into Azure AI Search"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Config file path
    #[arg(short, long, default_value = "quelch.yaml", global = true)]
    pub config: PathBuf,

    /// Increase verbosity (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Suppress TUI, only log errors
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Output logs as JSON
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(clap::Subcommand)]
pub enum Commands {
    /// Run a one-shot sync of all configured sources
    Sync,
    /// Run continuous sync (polls at configured interval)
    Watch,
    /// Show sync status for all sources
    Status,
    /// Reset sync state (force full re-sync on next run)
    Reset {
        /// Source name to reset (omit to reset all)
        source: Option<String>,
    },
    /// Validate config file without running
    Validate,
    /// Generate a starter quelch.yaml config
    Init,
}
```

- [ ] **Step 2: Rewrite main.rs**

Replace `crates/quelch/src/main.rs` entirely:

```rust
mod azure;
mod cli;
mod config;
mod sources;
mod sync;

use anyhow::Result;
use clap::Parser;
use std::path::Path;
use tracing::info;
use tracing_subscriber::EnvFilter;

use cli::{Cli, Commands};

fn setup_logging(verbose: u8, quiet: bool, json: bool) {
    let filter = match (quiet, verbose) {
        (true, _) => "error",
        (_, 0) => "quelch=info",
        (_, 1) => "quelch=debug",
        (_, 2) => "quelch=debug,reqwest=debug",
        _ => "trace",
    };

    let builder = tracing_subscriber::fmt().with_env_filter(EnvFilter::new(filter));

    if json {
        builder.json().init();
    } else {
        builder.init();
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    setup_logging(cli.verbose, cli.quiet, cli.json);

    match cli.command {
        Commands::Sync => cmd_sync(&cli.config).await,
        Commands::Watch => cmd_watch(&cli.config).await,
        Commands::Status => cmd_status(&cli.config),
        Commands::Reset { source } => cmd_reset(&cli.config, source.as_deref()),
        Commands::Validate => cmd_validate(&cli.config),
        Commands::Init => cmd_init(),
    }
}

async fn cmd_sync(config_path: &Path) -> Result<()> {
    let config = config::load_config(config_path)?;
    let state_path = Path::new(&config.sync.state_file).to_path_buf();
    info!("Starting one-shot sync");
    sync::run_sync(&config, &state_path).await?;
    info!("Sync complete");
    Ok(())
}

async fn cmd_watch(config_path: &Path) -> Result<()> {
    let config = config::load_config(config_path)?;
    let state_path = Path::new(&config.sync.state_file).to_path_buf();
    let interval = std::time::Duration::from_secs(config.sync.poll_interval);

    info!(poll_interval = config.sync.poll_interval, "Starting continuous sync");

    loop {
        if let Err(e) = sync::run_sync(&config, &state_path).await {
            tracing::error!(error = %e, "Sync cycle failed");
        }
        info!("Next sync in {} seconds", config.sync.poll_interval);
        tokio::time::sleep(interval).await;
    }
}

fn cmd_status(config_path: &Path) -> Result<()> {
    let config = config::load_config(config_path)?;
    let state_path = Path::new(&config.sync.state_file);
    let state = sync::state::SyncState::load(state_path)?;

    println!("Quelch Status");
    println!("{}", "─".repeat(50));
    println!("Config: {}", config_path.display());
    println!("Sources: {}", config.sources.len());
    println!();

    for source_config in &config.sources {
        let name = source_config.name();
        let source_state = state.get_source(name);

        let last_sync = source_state
            .last_sync_at
            .map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "never".to_string());

        println!("  {} ({})", name, source_config.index());
        println!("    Last sync:   {}", last_sync);
        println!("    Docs synced: {}", source_state.documents_synced);
        println!("    Sync count:  {}", source_state.sync_count);
        println!();
    }

    Ok(())
}

fn cmd_reset(config_path: &Path, source: Option<&str>) -> Result<()> {
    let config = config::load_config(config_path)?;
    let state_path = Path::new(&config.sync.state_file);
    let mut state = sync::state::SyncState::load(state_path)?;

    match source {
        Some(name) => {
            state.reset_source(name);
            println!("Reset sync state for source '{}'", name);
        }
        None => {
            state.reset_all();
            println!("Reset sync state for all sources");
        }
    }

    state.save(state_path)?;
    Ok(())
}

fn cmd_validate(config_path: &Path) -> Result<()> {
    let config = config::load_config(config_path)?;
    println!("Config is valid.");
    println!("  Azure endpoint: {}", config.azure.endpoint);
    println!("  Sources: {}", config.sources.len());
    for source in &config.sources {
        println!("    - {} -> index '{}'", source.name(), source.index());
    }
    Ok(())
}

fn cmd_init() -> Result<()> {
    let template = r#"# quelch.yaml

azure:
  endpoint: "https://your-search-service.search.windows.net"
  api_key: "${AZURE_SEARCH_API_KEY}"

sources:
  # Jira Cloud example (uses email + API token)
  - type: jira
    name: "my-jira-cloud"
    url: "https://your-company.atlassian.net"
    auth:
      email: "${JIRA_EMAIL}"
      api_token: "${JIRA_API_TOKEN}"
    projects:
      - "PROJ"
    index: "jira-issues"

  # Jira Data Center example (uses PAT)
  # - type: jira
  #   name: "my-jira-dc"
  #   url: "https://jira.internal.company.com"
  #   auth:
  #     pat: "${JIRA_PAT}"
  #   projects:
  #     - "HR"
  #   index: "jira-issues"

# Optional overrides (all have sensible defaults)
# sync:
#   poll_interval: 300
#   batch_size: 100
#   max_concurrent_per_credential: 3
#   state_file: ".quelch-state.json"
"#;

    let path = Path::new("quelch.yaml");
    if path.exists() {
        anyhow::bail!("quelch.yaml already exists — remove it first or edit it directly");
    }

    std::fs::write(path, template)?;
    println!("Created quelch.yaml — edit it with your Azure and source credentials");
    Ok(())
}
```

- [ ] **Step 3: Run all tests**

Run: `cargo test --workspace`
Expected: All tests pass (env: 4, config: 8, schema: 3, jira: 7, state: 6 = 28 tests).

- [ ] **Step 4: Verify CLI**

Run: `cargo run -p quelch -- --help`
Expected: Shows all commands with global options.

Run: `cargo run -p quelch -- init && cat quelch.yaml && rm quelch.yaml`
Expected: Creates config showing both Cloud and DC auth examples.

- [ ] **Step 5: Quality checks and commit**

Run: `cargo fmt --all && cargo clippy --workspace -- -D warnings`

```bash
git add crates/quelch/src/cli.rs crates/quelch/src/main.rs
git commit -m "Wire CLI commands to real implementations (sync, watch, status, reset, validate, init)"
```

---

### Task 7: Integration Tests with Mock Servers

**Files:**
- Create: `crates/quelch/tests/integration_test.rs`

- [ ] **Step 1: Create the integration test file with Jira and Azure mocks**

Create `crates/quelch/tests/integration_test.rs`:

```rust
use std::io::Write;
use tempfile::{NamedTempFile, TempDir};
use wiremock::matchers::{header, method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Helper: create a config file pointing at mock servers.
fn write_test_config(jira_url: &str, azure_url: &str) -> (NamedTempFile, TempDir) {
    let state_dir = TempDir::new().unwrap();
    let state_path = state_dir.path().join("state.json");

    let yaml = format!(
        r#"
azure:
  endpoint: "{azure_url}"
  api_key: "test-api-key"
sources:
  - type: jira
    name: "mock-jira"
    url: "{jira_url}"
    auth:
      pat: "test-pat"
    projects:
      - "TEST"
    index: "jira-issues"
sync:
  batch_size: 50
  state_file: "{state_file}"
"#,
        state_file = state_path.display()
    );

    let mut f = NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();
    (f, state_dir)
}

/// Build a realistic Jira search response matching the real API format.
fn jira_search_response(issues: Vec<serde_json::Value>) -> serde_json::Value {
    let total = issues.len();
    serde_json::json!({
        "expand": "schema,names",
        "startAt": 0,
        "maxResults": 50,
        "total": total,
        "issues": issues
    })
}

/// Build a single Jira issue matching the real API response format.
fn jira_issue(key: &str, summary: &str, updated: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "10001",
        "self": format!("https://jira.example.com/rest/api/2/issue/{key}"),
        "key": key,
        "fields": {
            "summary": summary,
            "description": "Test description",
            "status": {
                "name": "Open",
                "id": "1",
                "statusCategory": { "id": 2, "key": "new", "name": "To Do" }
            },
            "priority": { "name": "High", "id": "2" },
            "issuetype": { "name": "Bug", "id": "10001" },
            "assignee": {
                "displayName": "Test User",
                "accountId": "abc123"
            },
            "reporter": {
                "displayName": "Reporter User",
                "accountId": "def456"
            },
            "project": { "key": "TEST", "name": "Test Project" },
            "labels": ["test-label"],
            "created": "2025-01-10T08:00:00.000+0000",
            "updated": updated,
            "comment": {
                "startAt": 0,
                "maxResults": 5,
                "total": 1,
                "comments": [
                    {
                        "id": "10100",
                        "body": "Test comment body",
                        "author": { "displayName": "Commenter" },
                        "created": "2025-01-11T09:00:00.000+0000",
                        "updated": "2025-01-11T09:00:00.000+0000"
                    }
                ]
            }
        }
    })
}

/// Azure index docs/index success response.
fn azure_index_response(keys: &[&str]) -> serde_json::Value {
    let value: Vec<serde_json::Value> = keys
        .iter()
        .map(|k| {
            serde_json::json!({
                "key": k,
                "status": true,
                "errorMessage": null,
                "statusCode": 201
            })
        })
        .collect();
    serde_json::json!({ "value": value })
}

#[tokio::test]
async fn full_sync_jira_to_azure() {
    // Start mock Jira server
    let jira_server = MockServer::start().await;

    // Mock Jira search endpoint
    Mock::given(method("GET"))
        .and(path("/rest/api/2/search"))
        .and(header("Authorization", "Bearer test-pat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jira_search_response(vec![
            jira_issue("TEST-1", "First issue", "2025-01-15T10:30:00.000+0000"),
            jira_issue("TEST-2", "Second issue", "2025-01-15T11:00:00.000+0000"),
        ])))
        .expect(1)
        .mount(&jira_server)
        .await;

    // Start mock Azure server
    let azure_server = MockServer::start().await;

    // Mock Azure GET index (404 = doesn't exist yet)
    Mock::given(method("GET"))
        .and(path_regex(r"/indexes/jira-issues"))
        .and(query_param("api-version", "2024-07-01"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "error": { "code": "IndexNotFound", "message": "Index not found" }
        })))
        .expect(1)
        .mount(&azure_server)
        .await;

    // Mock Azure POST create index (201)
    Mock::given(method("POST"))
        .and(path("/indexes"))
        .and(query_param("api-version", "2024-07-01"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "name": "jira-issues",
            "fields": []
        })))
        .expect(1)
        .mount(&azure_server)
        .await;

    // Mock Azure POST docs/index (200 = all docs accepted)
    Mock::given(method("POST"))
        .and(path("/indexes/jira-issues/docs/index"))
        .and(query_param("api-version", "2024-07-01"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(azure_index_response(&["mock-jira-TEST-1", "mock-jira-TEST-2"])),
        )
        .expect(1)
        .mount(&azure_server)
        .await;

    // Load config and run sync
    let (config_file, state_dir) =
        write_test_config(&jira_server.uri(), &azure_server.uri());

    let config = quelch::config::load_config(config_file.path()).unwrap();
    let state_path = state_dir.path().join("state.json");

    quelch::sync::run_sync(&config, &state_path).await.unwrap();

    // Verify state was persisted
    let state = quelch::sync::state::SyncState::load(&state_path).unwrap();
    let source_state = state.get_source("mock-jira");
    assert!(source_state.last_cursor.is_some());
    assert_eq!(source_state.documents_synced, 2);
    assert_eq!(source_state.sync_count, 1);
}

#[tokio::test]
async fn incremental_sync_uses_cursor() {
    let jira_server = MockServer::start().await;
    let azure_server = MockServer::start().await;

    // Jira returns no new issues (empty result)
    Mock::given(method("GET"))
        .and(path("/rest/api/2/search"))
        .and(query_param("jql", "project = TEST AND updated >= \"2025-01-15 11:00\" ORDER BY updated ASC"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(jira_search_response(vec![])),
        )
        .expect(1)
        .mount(&jira_server)
        .await;

    // Azure index already exists
    Mock::given(method("GET"))
        .and(path_regex(r"/indexes/jira-issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "jira-issues", "fields": []
        })))
        .expect(1)
        .mount(&azure_server)
        .await;

    let (config_file, state_dir) =
        write_test_config(&jira_server.uri(), &azure_server.uri());

    // Pre-populate state with a cursor
    let state_path = state_dir.path().join("state.json");
    let mut state = quelch::sync::state::SyncState::default();
    state.update_source(
        "mock-jira",
        chrono::DateTime::parse_from_rfc3339("2025-01-15T11:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
        2,
    );
    state.save(&state_path).unwrap();

    let config = quelch::config::load_config(config_file.path()).unwrap();
    quelch::sync::run_sync(&config, &state_path).await.unwrap();

    // Docs synced should still be 2 (no new docs)
    let state = quelch::sync::state::SyncState::load(&state_path).unwrap();
    assert_eq!(state.get_source("mock-jira").documents_synced, 2);
}
```

- [ ] **Step 2: Make modules public for integration tests**

The integration tests need access to `quelch::config`, `quelch::sync`, etc. Add a `lib.rs` or make the modules public. The simplest approach: create `crates/quelch/src/lib.rs`:

```rust
pub mod azure;
pub mod config;
pub mod sources;
pub mod sync;
```

And update `main.rs` to use the library:

```rust
// At the top of main.rs, replace the mod declarations with:
use quelch::{azure, config, sources, sync};

mod cli;
```

Wait — since it's a binary crate, we need to restructure slightly. Add a `[lib]` section to `crates/quelch/Cargo.toml`:

```toml
[lib]
name = "quelch"
path = "src/lib.rs"
```

- [ ] **Step 3: Run integration tests**

Run: `cargo test --workspace -- --nocapture`
Expected: All unit tests pass + 2 integration tests pass.

- [ ] **Step 4: Quality checks and commit**

Run: `cargo fmt --all && cargo clippy --workspace -- -D warnings`

```bash
git add crates/quelch/tests/ crates/quelch/src/lib.rs crates/quelch/src/main.rs crates/quelch/Cargo.toml
git commit -m "Add integration tests with wiremock Jira and Azure AI Search mock servers"
```

---

### Task 8: Final Verification

- [ ] **Step 1: Run the full test suite**

Run: `cargo test --workspace`
Expected: All ~30 tests pass.

- [ ] **Step 2: Run full quality checks**

Run: `cargo fmt --all -- --check && cargo clippy --workspace -- -D warnings`
Expected: Clean.

- [ ] **Step 3: Verify CLI end-to-end**

Run: `cargo run -p quelch -- --version`
Expected: `quelch 0.1.0`

Run: `cargo run -p quelch -- --help`
Expected: All commands with global flags.

Run: `cargo run -p quelch -- init && cargo run -p quelch -- validate -c quelch.yaml 2>&1; rm quelch.yaml`
Expected: Init creates file, validate fails on env vars (correct behavior).

- [ ] **Step 4: Commit any fixes**

Only if needed. Message: `Fix final compilation and test issues`
