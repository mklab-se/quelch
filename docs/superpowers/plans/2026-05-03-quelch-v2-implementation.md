# Quelch v2 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Transform Quelch from v1 (direct-to-Azure-AI-Search ingest tool) into v2 (Cosmos-DB-primary platform with rigg-managed AI Search, an MCP server agents talk to, and an operator CLI that reconciles Azure to a single declarative config).

**Architecture:** One Rust binary, three runtime roles (`quelch ingest`, `quelch mcp`, the operator CLI), Cosmos as the system of record, Azure AI Search via rigg for hybrid semantic retrieval, MCP Streamable HTTP for agents, Bicep + rigg-as-library for provisioning. See [`/docs/`](../../../docs/) for the canonical spec.

**Tech Stack:** Rust 2024, tokio, clap, serde + serde_yaml, reqwest, axum (for the MCP HTTP server), rigg-core + rigg-client (library deps), Azure CLI (`az` shell-outs), Bicep generation (Rust `handlebars` for template rendering).

---

## How to read this plan

Eleven phases. Each phase is **internally shippable** — when you stop at the end of a phase, the project compiles, tests pass, and the work done so far is meaningful. Recommended execution: subagent-driven, one phase at a time, review at phase boundaries.

Each phase has a "Files" section listing every path that gets touched, and a sequence of tasks. Each task is 2–5 minutes of focused work, written test-first wherever possible.

Cross-references:

- **Spec**: [`/docs/superpowers/specs/2026-04-30-quelch-rearchitecture-design.md`](../specs/2026-04-30-quelch-rearchitecture-design.md)
- **Architecture**: [`/docs/architecture.md`](../../architecture.md)
- **Configuration**: [`/docs/configuration.md`](../../configuration.md)
- **Sync correctness**: [`/docs/sync.md`](../../sync.md)
- **MCP API**: [`/docs/mcp-api.md`](../../mcp-api.md)
- **Deployment**: [`/docs/deployment.md`](../../deployment.md)
- **CLI**: [`/docs/cli.md`](../../cli.md)
- **Agent generation**: [`/docs/agent-generation.md`](../../agent-generation.md)

Whenever a task says "see X", that's a directive to actually open X — the plan does not duplicate the spec.

## Pre-push gate (every commit)

Per `CLAUDE.md`:

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

All three must pass before `git push`. If you're committing locally and not pushing, run them anyway as a sanity check — clippy in particular catches things you'd otherwise have to undo later.

---

# Phase 0 — Workspace and dependency setup

**Goal:** Get the workspace into shape for the new modules, with rigg + ailloy dependencies wired in. No behaviour change yet.

**Files:**

- Modify: `Cargo.toml` (workspace dependencies)
- Modify: `crates/quelch/Cargo.toml` (crate dependencies)
- Modify: `CHANGELOG.md` (start v2 section)
- Modify: `/Users/kristofer/repos/rigg/Cargo.toml` (bump ailloy)

### Task 0.1: Bump rigg's ailloy dependency

Rigg currently pins `ailloy = "0.5"`; quelch is on `0.7.2`. We need them to agree before quelch can depend on rigg-core/rigg-client.

**Files:**

- Modify: `/Users/kristofer/repos/rigg/Cargo.toml`

- [ ] **Step 1: Update the ailloy version in rigg's workspace deps**

```toml
# in /Users/kristofer/repos/rigg/Cargo.toml [workspace.dependencies]
ailloy = { version = "0.7", default-features = false, features = ["config-tui"] }
```

- [ ] **Step 2: Run rigg's full pre-push gate**

```bash
cd /Users/kristofer/repos/rigg
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

If anything fails, fix the API drift before continuing — usually a renamed type or a moved module. The user owns ailloy so this is mechanical, not a compatibility battle.

- [ ] **Step 3: Bump rigg's version (patch bump)**

```bash
cd /Users/kristofer/repos/rigg
# Manually bump version in Cargo.toml [workspace.package] from 0.16.0 → 0.16.1
```

- [ ] **Step 4: Commit in rigg repo**

```bash
cd /Users/kristofer/repos/rigg
git add Cargo.toml Cargo.lock
git commit -m "deps: bump ailloy to 0.7 for quelch interop"
```

- [ ] **Step 5: Verify rigg-core and rigg-client publish cleanly (dry run only)**

```bash
cd /Users/kristofer/repos/rigg
cargo publish --dry-run -p rigg-core
cargo publish --dry-run -p rigg-client
```

Expected: both pass. If they fail, fix and re-run.

### Task 0.2: Add rigg-core, rigg-client, and a few new deps to quelch

**Files:**

- Modify: `Cargo.toml` (workspace deps)
- Modify: `crates/quelch/Cargo.toml`

- [ ] **Step 1: Add the new workspace dependencies**

In `/Users/kristofer/repos/quelch/Cargo.toml` `[workspace.dependencies]`, add:

```toml
# rigg as a library — manages Azure AI Search and Foundry config
rigg-core = "0.16.1"
rigg-client = "0.16.1"

# MCP server transport
axum = { version = "0.8", features = ["macros"] }
tower = "0.5"
tower-http = { version = "0.6", features = ["trace", "cors"] }

# Cosmos DB client (azure_data_cosmos is the official azure-sdk-for-rust crate)
azure_data_cosmos = "0.21"
azure_identity = "0.21"

# Templating for Bicep / on-prem artefacts
handlebars = "6"

# Toml for codex-mcp.toml output
toml = "0.8"

# Bigger reqwest features for retry/backoff
reqwest-retry = "0.6"
reqwest-middleware = "0.3"

# JSON Schema for MCP tool descriptions
schemars = "1"
```

- [ ] **Step 2: Add them to the quelch crate**

In `/Users/kristofer/repos/quelch/crates/quelch/Cargo.toml`:

```toml
[dependencies]
rigg-core = { workspace = true }
rigg-client = { workspace = true }
axum = { workspace = true }
tower = { workspace = true }
tower-http = { workspace = true }
azure_data_cosmos = { workspace = true }
azure_identity = { workspace = true }
handlebars = { workspace = true }
toml = { workspace = true }
reqwest-retry = { workspace = true }
reqwest-middleware = { workspace = true }
schemars = { workspace = true }
# (existing deps continue)
```

- [ ] **Step 3: Verify it builds**

```bash
cd /Users/kristofer/repos/quelch
cargo build --workspace
```

Expected: PASS. New deps download and compile.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/quelch/Cargo.toml
git commit -m "deps: add rigg, axum, cosmos sdk, and bicep-templating deps"
```

### Task 0.3: Begin v2 changelog entry

**Files:**

- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add a v2.0.0-alpha section header**

At the top of `CHANGELOG.md`:

```markdown
## [2.0.0-alpha] - Unreleased

### Breaking

This is a complete re-architecture; v1 configs are not compatible. See
[/docs/](docs/) for the new architecture and [migration guide TBD] for
upgrade steps.

### In progress

(implementation in progress; see docs/superpowers/plans/2026-05-03-quelch-v2-implementation.md)
```

- [ ] **Step 2: Commit**

```bash
git add CHANGELOG.md
git commit -m "chore: open v2.0.0-alpha changelog section"
```

---

# Phase 1 — Configuration v2

**Goal:** A working `quelch validate` against the new YAML schema. v1's `config/` is rewritten to deserialise the v2 sections from [`/docs/configuration.md`](../../configuration.md) (`azure`, `cosmos`, `search`, `openai`, `sources`, `ingest`, `deployments`, `mcp`, `rigg`, `state`).

**Files:**

- Replace: `crates/quelch/src/config/mod.rs`
- Keep: `crates/quelch/src/config/env.rs`
- New: `crates/quelch/src/config/schema.rs` (the new structs)
- New: `crates/quelch/src/config/validate.rs`
- New: `crates/quelch/src/config/slice.rs` (effective-config slicing)
- New: `crates/quelch/src/config/data_sources.rs` (auto-derived `mcp.data_sources` defaults)
- Modify: `crates/quelch/src/lib.rs` (re-export new types)

### Task 1.1: Define the v2 schema structs

**Files:**

- New: `crates/quelch/src/config/schema.rs`

- [ ] **Step 1: Write the failing test** in `crates/quelch/src/config/schema.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_v2_config() {
        let yaml = r#"
azure:
  subscription_id: "sub-123"
  resource_group: "rg-test"
  region: "swedencentral"
cosmos:
  database: "quelch"
search:
  sku: "basic"
openai:
  endpoint: "https://test.openai.azure.com"
  embedding_deployment: "text-embedding-3-large"
  embedding_dimensions: 3072
sources:
  - type: jira
    name: jira-cloud
    url: "https://example.atlassian.net"
    auth:
      email: "u@example.com"
      api_token: "tok"
    projects: ["DO"]
deployments:
  - name: ingest
    role: ingest
    target: azure
    sources:
      - source: jira-cloud
  - name: mcp
    role: mcp
    target: azure
    expose: ["jira_issues"]
    auth:
      mode: "api_key"
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.azure.region, "swedencentral");
        assert_eq!(config.deployments.len(), 2);
    }
}
```

- [ ] **Step 2: Run the test — expect compile errors**

```bash
cargo test -p quelch config::schema::tests::parses_minimal_v2_config
```

Expected: FAIL with "Config not defined".

- [ ] **Step 3: Define the structs**

Write the full type tree in `crates/quelch/src/config/schema.rs` matching every section of [configuration.md](../../configuration.md):

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub azure: AzureConfig,
    pub cosmos: CosmosConfig,
    #[serde(default)]
    pub search: SearchConfig,
    pub openai: OpenAiConfig,
    pub sources: Vec<SourceConfig>,
    #[serde(default)]
    pub ingest: IngestConfig,
    pub deployments: Vec<DeploymentConfig>,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub rigg: RiggConfig,
    #[serde(default)]
    pub state: StateConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AzureConfig {
    pub subscription_id: String,
    pub resource_group: String,
    pub region: String,
    #[serde(default)]
    pub naming: NamingConfig,
    #[serde(default)]
    pub skip_role_assignments: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct NamingConfig {
    pub prefix: Option<String>,
    pub environment: Option<String>,
}

// ... (the full struct tree — see configuration.md for every field)
```

Write all of: `CosmosConfig`, `CosmosContainersDefaults`, `CosmosThroughput`, `SearchConfig`, `IndexerSchedule`, `OpenAiConfig`, `SourceConfig` (untagged enum: `Jira` | `Confluence`), `JiraSourceConfig`, `ConfluenceSourceConfig`, `AuthConfig` (untagged enum), `CompanionContainersConfig`, `IngestConfig`, `DeploymentConfig`, `DeploymentRole` (enum `Ingest` | `Mcp`), `DeploymentTarget` (enum `Azure` | `Onprem`), `DeploymentSources`, `DeploymentAzureConfig`, `ContainerAppSpec`, `McpAuthMode`, `McpConfig`, `McpDataSource`, `BackedBy`, `McpSearchConfig`, `RiggConfig`, `StateConfig`.

Use `#[serde(rename_all = "snake_case")]` on enum tags and `#[serde(untagged)]` for `SourceConfig` and `AuthConfig` (matches existing v1 convention).

- [ ] **Step 4: Run the test again — expect PASS**

```bash
cargo test -p quelch config::schema::tests::parses_minimal_v2_config
```

- [ ] **Step 5: Commit**

```bash
git add crates/quelch/src/config/schema.rs
git commit -m "feat(config): v2 YAML schema"
```

### Task 1.2: Wire env-var substitution into the new schema

The v1 `config/env.rs` already does `${VAR}` substitution and is keepable as-is. Just make sure the new loader uses it.

**Files:**

- New: `crates/quelch/src/config/mod.rs` (replaces v1)

- [ ] **Step 1: Write the failing test** in `crates/quelch/src/config/mod.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn loads_with_env_substitution() {
        unsafe { std::env::set_var("Q_TEST_SUB", "subA"); }
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
}
```

- [ ] **Step 2: Run, expect FAIL**

- [ ] **Step 3: Implement `load_config`**

```rust
pub mod env;
pub mod schema;
pub mod validate;
pub mod slice;
pub mod data_sources;

pub use schema::*;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file '{path}': {source}")]
    ReadFile { path: String, source: std::io::Error },
    #[error("invalid YAML: {0}")]
    InvalidYaml(#[from] serde_yaml::Error),
    #[error("env var error: {0}")]
    EnvVar(#[from] env::EnvVarError),
    #[error("validation: {0}")]
    Validation(String),
}

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
```

- [ ] **Step 4: Make `validate::run` a stub for now**

```rust
// crates/quelch/src/config/validate.rs
use super::{Config, ConfigError};

pub fn run(_config: &Config) -> Result<(), ConfigError> {
    Ok(())
}
```

- [ ] **Step 5: Run, expect PASS, commit**

```bash
cargo test -p quelch config::tests::loads_with_env_substitution
git add crates/quelch/src/config/
git commit -m "feat(config): v2 loader with env substitution"
```

### Task 1.3: Implement validation rules

The validation rules in [configuration.md](../../configuration.md) state:

- Every `(source, subsource)` pair appears in at most one ingest deployment.
- Every name in any `expose:` list is defined in `mcp.data_sources` (or auto-derivable).
- Every source referenced in a deployment exists in `sources`.

**Files:**

- Modify: `crates/quelch/src/config/validate.rs`

- [ ] **Step 1: Write three failing tests**, one per rule. Each test loads a config that violates exactly one rule and asserts the error message contains the relevant identifier.

Example (the disjoint check):

```rust
#[test]
fn rejects_overlapping_subsources() {
    let yaml = include_str!("../../tests/fixtures/config_overlapping.yaml");
    let cfg: Config = serde_yaml::from_str(yaml).unwrap();
    let err = run(&cfg).unwrap_err();
    assert!(err.to_string().contains("DO"));
    assert!(err.to_string().contains("appears in"));
}
```

Stage three fixture files under `crates/quelch/tests/fixtures/` (or a `config/fixtures/` module in the source).

- [ ] **Step 2: Run, expect all FAIL**

- [ ] **Step 3: Implement `validate::run`**

```rust
pub fn run(config: &Config) -> Result<(), ConfigError> {
    validate_sources_referenced(config)?;
    validate_disjoint_subsources(config)?;
    validate_expose_resolves(config)?;
    Ok(())
}

fn validate_sources_referenced(config: &Config) -> Result<(), ConfigError> { /* ... */ }
fn validate_disjoint_subsources(config: &Config) -> Result<(), ConfigError> { /* ... */ }
fn validate_expose_resolves(config: &Config) -> Result<(), ConfigError> { /* ... */ }
```

Each helper iterates the relevant collections and constructs `ConfigError::Validation(...)` with the specific name.

- [ ] **Step 4: Run, expect PASS**

- [ ] **Step 5: Commit**

```bash
git add crates/quelch/src/config/validate.rs crates/quelch/tests/fixtures/
git commit -m "feat(config): validation rules"
```

### Task 1.4: Auto-derive `mcp.data_sources` defaults

Per [configuration.md "Auto-derived data_sources"](../../configuration.md#auto-derived-data_sources): if `mcp.data_sources` is omitted, derive one entry per `kind` from configured `sources` and the `cosmos.containers` defaults.

**Files:**

- New: `crates/quelch/src/config/data_sources.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn derives_jira_issues_from_two_sources() {
    let cfg = test_helpers::config_with_two_jira_sources();
    let resolved = resolve(&cfg);
    let jira_issues = resolved.get("jira_issues").unwrap();
    assert_eq!(jira_issues.kind, "jira_issue");
    assert_eq!(jira_issues.backed_by.len(), 2);
}
```

- [ ] **Step 2: Run, expect FAIL**

- [ ] **Step 3: Implement `resolve`**

```rust
use super::{Config, McpDataSource};
use std::collections::HashMap;

pub fn resolve(config: &Config) -> HashMap<String, ResolvedDataSource> {
    if !config.mcp.data_sources.is_empty() {
        return config.mcp.data_sources.iter()
            .map(|(name, src)| (name.clone(), ResolvedDataSource::from_explicit(src)))
            .collect();
    }
    derive_defaults(config)
}

fn derive_defaults(config: &Config) -> HashMap<String, ResolvedDataSource> { /* ... */ }
```

Implement `derive_defaults` to walk `config.sources` and emit one entry per source-type kind with `backed_by` pointing at each source's primary or companion container.

- [ ] **Step 4: Add tests for explicit override path** (when `mcp.data_sources` is populated, that wins).

- [ ] **Step 5: Run all tests in `data_sources::tests`, expect PASS, commit**

```bash
cargo test -p quelch config::data_sources
git add crates/quelch/src/config/data_sources.rs
git commit -m "feat(config): auto-derive mcp.data_sources defaults"
```

### Task 1.5: Implement effective-config slicing

Per [configuration.md "Slicing per deployment"](../../configuration.md#slicing-per-deployment): `slice::for_deployment(&config, "mcp-azure")` returns a sub-Config containing only what that one deployment needs.

**Files:**

- New: `crates/quelch/src/config/slice.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn slice_for_mcp_excludes_other_sources() {
    let cfg = test_helpers::config_with_jira_and_confluence();
    let sliced = slice::for_deployment(&cfg, "mcp").unwrap();
    // mcp deployment exposes jira_issues only; sources should be filtered to those
    // that contribute to jira_issues
    assert!(sliced.sources.iter().any(|s| s.name() == "jira-cloud"));
    assert!(sliced.sources.iter().all(|s| matches!(s, SourceConfig::Jira(_))));
    assert_eq!(sliced.deployments.len(), 1);
    assert_eq!(sliced.deployments[0].name, "mcp");
}
```

- [ ] **Step 2: Run, expect FAIL**

- [ ] **Step 3: Implement `for_deployment`**

The shape: keep `azure`, `cosmos`, `search`, `openai`, `ingest`, `rigg`, `state` (all global). Filter `deployments` to the one named. Filter `sources` to those referenced by that deployment (for ingest) or backing exposed data sources (for mcp). Filter `mcp.data_sources` to the exposed names.

- [ ] **Step 4: Add tests for the ingest-deployment case** and for "deployment not found" returning a typed error.

- [ ] **Step 5: Run, PASS, commit**

```bash
cargo test -p quelch config::slice
git add crates/quelch/src/config/slice.rs
git commit -m "feat(config): effective-config slicing"
```

### Task 1.6: Wire `quelch validate` and `quelch effective-config` into the CLI

**Files:**

- Modify: `crates/quelch/src/cli.rs`
- Modify: `crates/quelch/src/main.rs`

- [ ] **Step 1: Add the new clap subcommands**

In `cli.rs`, add `Validate` and `EffectiveConfig { name: String }` to the `Commands` enum (you'll be removing many old commands later — for now just add).

- [ ] **Step 2: Add the handlers in `main.rs`**

```rust
Commands::Validate => {
    let _config = quelch::config::load_config(&cli.config)?;
    println!("Config is valid.");
    Ok(())
}
Commands::EffectiveConfig { name } => {
    let config = quelch::config::load_config(&cli.config)?;
    let sliced = quelch::config::slice::for_deployment(&config, &name)?;
    let yaml = serde_yaml::to_string(&sliced)?;
    print!("{yaml}");
    Ok(())
}
```

- [ ] **Step 3: Add an integration test**

```rust
// crates/quelch/tests/cli_validate.rs
use assert_cmd::Command;

#[test]
fn validate_succeeds_on_minimal_config() {
    Command::cargo_bin("quelch").unwrap()
        .arg("--config").arg("tests/fixtures/quelch.minimal.yaml")
        .arg("validate")
        .assert()
        .success();
}
```

Drop a minimal valid `tests/fixtures/quelch.minimal.yaml` matching Task 1.1's example.

- [ ] **Step 4: Run, expect PASS, commit**

```bash
cargo test -p quelch --test cli_validate
git add crates/quelch/src/cli.rs crates/quelch/src/main.rs crates/quelch/tests/
git commit -m "feat(cli): quelch validate and effective-config"
```

### Phase 1 acceptance

```bash
cargo test -p quelch config
cargo run -p quelch -- validate -c tests/fixtures/quelch.minimal.yaml
cargo run -p quelch -- effective-config mcp -c tests/fixtures/quelch.minimal.yaml
```

All three exit 0. Phase 1 ships.

---

# Phase 2 — Cosmos client and sync state

**Goal:** A working `cosmos/` module that can upsert documents, point-read, run SQL queries, and read/write `quelch-meta` cursor state. Doubles as the storage backend for tests.

**Files:**

- New: `crates/quelch/src/cosmos/mod.rs` (public re-exports)
- New: `crates/quelch/src/cosmos/client.rs` (CosmosClient wrapping azure_data_cosmos)
- New: `crates/quelch/src/cosmos/meta.rs` (quelch-meta cursor doc CRUD)
- New: `crates/quelch/src/cosmos/in_memory.rs` (test/`quelch dev` backend)
- New: `crates/quelch/src/cosmos/error.rs`
- New: `crates/quelch/src/cosmos/document.rs` (the SourceDocument-to-Cosmos-doc envelope)
- Replace: `crates/quelch/src/sync/state.rs` (becomes a thin wrapper over `cosmos::meta`)

### Task 2.1: Define `CosmosBackend` trait

A trait so production code uses `azure_data_cosmos` and tests use `in_memory.rs`.

**Files:**

- New: `crates/quelch/src/cosmos/mod.rs`

- [ ] **Step 1: Write the trait**

```rust
// crates/quelch/src/cosmos/mod.rs
pub mod client;
pub mod meta;
pub mod in_memory;
pub mod error;
pub mod document;

pub use error::CosmosError;
pub use document::CosmosDocument;
use async_trait::async_trait;
use serde_json::Value;

#[async_trait]
pub trait CosmosBackend: Send + Sync {
    /// Upsert a document by id; partition key extracted from `_partition_key`.
    async fn upsert(&self, container: &str, doc: Value) -> Result<(), CosmosError>;

    /// Bulk upsert (single transaction per partition where the SDK supports it).
    async fn bulk_upsert(&self, container: &str, docs: Vec<Value>) -> Result<(), CosmosError>;

    /// Point-read by id and partition key. Returns None if not found.
    async fn get(&self, container: &str, id: &str, partition_key: &str)
        -> Result<Option<Value>, CosmosError>;

    /// Run a SQL query, returning all results as a stream of pages.
    async fn query(&self, container: &str, sql: &str, params: Vec<(String, Value)>)
        -> Result<QueryStream, CosmosError>;
}

pub struct QueryStream { /* internal cursor + page buffer */ }
impl QueryStream {
    pub async fn next_page(&mut self) -> Result<Option<Vec<Value>>, CosmosError> { /* ... */ }
    pub fn continuation_token(&self) -> Option<&str> { /* ... */ }
}
```

`async_trait` may need to be added to `Cargo.toml` if not already there (we have `trait-variant`, but `async_trait` is more ergonomic for object-safe traits).

- [ ] **Step 2: Sanity-build**

```bash
cargo build -p quelch
```

Expected: PASS (lots of `unimplemented!()` is OK at this stage).

- [ ] **Step 3: Commit**

```bash
git add crates/quelch/src/cosmos/
git commit -m "feat(cosmos): backend trait scaffold"
```

### Task 2.2: Implement the in-memory backend

Used for `quelch dev` and unit tests.

**Files:**

- New: `crates/quelch/src/cosmos/in_memory.rs`

- [ ] **Step 1: Write a test fixture**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn upsert_and_get_round_trip() {
        let backend = InMemoryCosmos::new();
        let doc = serde_json::json!({
            "id": "j-DO-1",
            "_partition_key": "DO",
            "key": "DO-1",
            "summary": "Test"
        });
        backend.upsert("jira-issues", doc.clone()).await.unwrap();
        let fetched = backend.get("jira-issues", "j-DO-1", "DO").await.unwrap();
        assert_eq!(fetched, Some(doc));
    }

    #[tokio::test]
    async fn point_read_with_wrong_partition_returns_none() { /* ... */ }

    #[tokio::test]
    async fn upsert_overwrites_existing() { /* ... */ }
}
```

- [ ] **Step 2: Run, expect FAIL**

- [ ] **Step 3: Implement `InMemoryCosmos`**

```rust
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct InMemoryCosmos {
    // container_name → ((id, partition_key) → doc)
    state: Arc<Mutex<HashMap<String, HashMap<(String, String), Value>>>>,
}

#[async_trait]
impl CosmosBackend for InMemoryCosmos {
    async fn upsert(&self, container: &str, doc: Value) -> Result<(), CosmosError> {
        let id = doc.get("id").and_then(Value::as_str)
            .ok_or_else(|| CosmosError::Validation("doc missing id".into()))?
            .to_string();
        let pk = doc.get("_partition_key").and_then(Value::as_str)
            .ok_or_else(|| CosmosError::Validation("doc missing _partition_key".into()))?
            .to_string();
        let mut s = self.state.lock().unwrap();
        s.entry(container.to_string())
            .or_default()
            .insert((id, pk), doc);
        Ok(())
    }
    // ...
}
```

Implement `bulk_upsert`, `get`, and `query` (the last one parses a tiny SQL subset enough for tests).

For `query`: since you only need it to support enough for the cursor-state tests in Task 2.3, you can implement an extremely minimal SQL parser — `WHERE id = @id` and `WHERE _partition_key = @pk` are enough for now.

- [ ] **Step 4: Run, expect PASS**

- [ ] **Step 5: Commit**

```bash
git add crates/quelch/src/cosmos/in_memory.rs
git commit -m "feat(cosmos): in-memory backend"
```

### Task 2.3: Implement quelch-meta cursor CRUD

Per [sync.md "State stored per (source, subsource)"](../../sync.md#state-stored-per-source-subsource), the cursor doc has a fixed schema. Build typed `Cursor::load` / `Cursor::save` that hides the JSON details.

**Files:**

- New: `crates/quelch/src/cosmos/meta.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosmos::in_memory::InMemoryCosmos;

    #[tokio::test]
    async fn save_and_load_cursor_round_trip() {
        let cosmos = InMemoryCosmos::new();
        let key = CursorKey {
            deployment_name: "ingest-az".into(),
            source_name: "jira-cloud".into(),
            subsource: "DO".into(),
        };
        let cursor = Cursor {
            last_complete_minute: Some(chrono::Utc::now()),
            documents_synced_total: 12,
            backfill_in_progress: false,
            ..Default::default()
        };
        save(&cosmos, "quelch-meta", &key, &cursor).await.unwrap();
        let loaded = load(&cosmos, "quelch-meta", &key).await.unwrap();
        assert_eq!(loaded.documents_synced_total, 12);
    }

    #[tokio::test]
    async fn load_returns_default_when_missing() {
        let cosmos = InMemoryCosmos::new();
        let key = CursorKey { /* ... */ };
        let loaded = load(&cosmos, "quelch-meta", &key).await.unwrap();
        assert!(loaded.last_complete_minute.is_none());
    }
}
```

- [ ] **Step 2: Implement the `Cursor` struct + `CursorKey` + `load`/`save`**

The Cursor struct mirrors the JSON schema in sync.md. `CursorKey::id()` returns `"{deployment}::{source}::{subsource}"`; `_partition_key` is `deployment_name`.

- [ ] **Step 3: Run, expect PASS, commit**

```bash
cargo test -p quelch cosmos::meta
git add crates/quelch/src/cosmos/meta.rs
git commit -m "feat(cosmos): cursor state CRUD against quelch-meta"
```

### Task 2.4: Implement the real Cosmos client

Production backend wrapping `azure_data_cosmos`.

**Files:**

- New: `crates/quelch/src/cosmos/client.rs`

- [ ] **Step 1: Wire `azure_data_cosmos::CosmosClient`**

```rust
use azure_data_cosmos::CosmosClient as AzCosmosClient;
use azure_identity::DefaultAzureCredential;

pub struct CosmosClient {
    inner: AzCosmosClient,
    database_name: String,
}

impl CosmosClient {
    pub async fn new(account_endpoint: &str, database_name: &str) -> Result<Self, CosmosError> {
        let credential = DefaultAzureCredential::new()?;
        let inner = AzCosmosClient::new(account_endpoint, credential, None)?;
        Ok(Self { inner, database_name: database_name.into() })
    }
}

#[async_trait]
impl CosmosBackend for CosmosClient { /* implement against the SDK */ }
```

- [ ] **Step 2: Add an integration test guarded by env var**

```rust
#[tokio::test]
#[ignore = "requires Azure Cosmos; set QUELCH_COSMOS_E2E_ENDPOINT to enable"]
async fn e2e_upsert_and_get() {
    let endpoint = std::env::var("QUELCH_COSMOS_E2E_ENDPOINT").unwrap();
    let client = CosmosClient::new(&endpoint, "quelch-test").await.unwrap();
    // ... round trip
}
```

This is `#[ignore]` so CI doesn't try to run it without credentials. We'll wire it into the release CI conditionally later.

- [ ] **Step 3: Commit**

```bash
git add crates/quelch/src/cosmos/client.rs
git commit -m "feat(cosmos): azure-sdk CosmosClient backend"
```

### Phase 2 acceptance

```bash
cargo test -p quelch cosmos
```

All non-ignored tests pass. Phase 2 ships — the cosmos layer is ready to be used by the ingest engine.

---

# Phase 3 — Ingest engine (the heart)

**Goal:** Replace the v1 sync engine with the new minute-resolution algorithm from [sync.md](../../sync.md). Concretely: a worker process that reads a sliced config, advances `last_complete_minute` cycle-by-cycle, performs full backfill on first run, and runs periodic reconciliation for deletions.

**Files:**

- Keep: `crates/quelch/src/sources/jira.rs` (extend)
- Keep: `crates/quelch/src/sources/confluence.rs` (extend)
- Keep: `crates/quelch/src/sources/mod.rs` (extend the trait)
- New: `crates/quelch/src/ingest/mod.rs` (the engine)
- New: `crates/quelch/src/ingest/cycle.rs` (per-cycle algorithm)
- New: `crates/quelch/src/ingest/backfill.rs` (backfill resume protocol)
- New: `crates/quelch/src/ingest/reconcile.rs` (deletion detection)
- New: `crates/quelch/src/ingest/window.rs` (minute-window math)
- New: `crates/quelch/src/ingest/rate_limit.rs` (Retry-After + 5xx backoff)
- Delete: `crates/quelch/src/sync/embedder.rs`
- Delete: `crates/quelch/src/sync/mod.rs`, `phases.rs` (will be migrated piece by piece)
- Replace: `crates/quelch/src/sync/state.rs` → re-exports from `cosmos::meta` for compatibility, eventually removed

### Task 3.1: Extend the SourceConnector trait

The v1 trait (`sources/mod.rs`) returns `SourceDocument` from `fetch_changes(subsource, cursor, batch_size)`. v2 needs:
- `fetch_window(subsource, window_start, window_end, batch_size, page_token)` — accepts a closed minute interval.
- `fetch_resumable(subsource, target, last_seen, batch_size)` — for backfill resume.
- `list_all_ids(subsource)` — returns the full id set for reconciliation.
- Companion-container fetches: `fetch_sprints`, `fetch_fix_versions`, `fetch_projects`, `fetch_spaces`.

**Files:**

- Modify: `crates/quelch/src/sources/mod.rs`

- [ ] **Step 1: Define the new trait shape**

```rust
#[trait_variant::make(Send)]
pub trait SourceConnector: Sync {
    fn source_type(&self) -> &str;
    fn source_name(&self) -> &str;
    fn subsources(&self) -> &[String];

    /// Fetch a closed minute-resolution window of issues/pages.
    async fn fetch_window(
        &self,
        subsource: &str,
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
        batch_size: usize,
        page_token: Option<&str>,
    ) -> anyhow::Result<FetchPage>;

    /// Fetch backfill page resuming after `last_seen` (None = start fresh).
    async fn fetch_backfill_page(
        &self,
        subsource: &str,
        backfill_target: DateTime<Utc>,
        last_seen: Option<&BackfillCheckpoint>,
        batch_size: usize,
    ) -> anyhow::Result<FetchPage>;

    /// List all ids currently in the source for a subsource (deletion reconciliation).
    async fn list_all_ids(&self, subsource: &str) -> anyhow::Result<Vec<String>>;

    /// Companion-container fetches; default impls return empty for sources that don't have them.
    async fn fetch_companions(&self, subsource: &str) -> anyhow::Result<Companions> {
        Ok(Companions::default())
    }
}

pub struct FetchPage {
    pub documents: Vec<SourceDocument>,
    pub next_page_token: Option<String>,
    pub last_seen: Option<BackfillCheckpoint>,  // populated by fetch_backfill_page
}

#[derive(Clone, Debug)]
pub struct BackfillCheckpoint {
    pub updated: DateTime<Utc>,
    pub key: String,
}

#[derive(Default)]
pub struct Companions {
    pub sprints: Vec<SourceDocument>,
    pub fix_versions: Vec<SourceDocument>,
    pub projects: Vec<SourceDocument>,
    pub spaces: Vec<SourceDocument>,
}
```

- [ ] **Step 2: Build the workspace; expect breakage in `sources/jira.rs` and `sources/confluence.rs`**

These will need re-implementation in tasks 3.2 and 3.3. For now, stub their old methods to compile (`unimplemented!()`).

- [ ] **Step 3: Commit the trait change with stubbed connector impls**

```bash
cargo build -p quelch
git add crates/quelch/src/sources/
git commit -m "refactor(sources): v2 connector trait — windows, backfill, reconciliation"
```

### Task 3.2: Re-implement Jira connector

The existing Jira connector logic (auth, paging) is keepable — just rewrap the methods.

**Files:**

- Modify: `crates/quelch/src/sources/jira.rs`

- [ ] **Step 1: Add unit tests using `wiremock`**

```rust
#[tokio::test]
async fn fetch_window_emits_correct_jql() {
    let server = wiremock::MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/search"))
        .and(query_param_contains("jql", "updated >= \"2026/04/30 14:23\""))
        .and(query_param_contains("jql", "updated <= \"2026/04/30 14:25\""))
        .and(query_param_contains("jql", "ORDER BY updated ASC, key ASC"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"issues":[]})))
        .mount(&server).await;

    let connector = JiraConnector::new(server.uri(), AuthConfig::DataCenter { pat: "x".into() }, /* ... */);
    let start = "2026-04-30T14:23:00Z".parse().unwrap();
    let end = "2026-04-30T14:25:00Z".parse().unwrap();
    connector.fetch_window("DO", start, end, 100, None).await.unwrap();
}
```

- [ ] **Step 2: Run, expect FAIL**

- [ ] **Step 3: Implement `fetch_window`**

Compose JQL exactly per [sync.md](../../sync.md#per-cycle-steps):

```
project = "{key}"
AND updated >= "{format(window_start, "yyyy/MM/dd HH:mm")}"
AND updated <= "{format(window_end,   "yyyy/MM/dd HH:mm")}"
ORDER BY updated ASC, key ASC
```

Map every field in [architecture.md "Jira issue"](../../architecture.md#jira-issue-jira_issues) — including `priority`, `resolution`, `resolved`, `status_category`, `parent`, `issuelinks`, `affects_versions`. Use the standard `?fields=*all` and post-process, or list the exact field names; either is fine.

Custom-field mapping comes from `JiraSourceConfig.fields` — read those, fetch via `?fields=customfield_10016`, project to friendly names like `story_points`.

- [ ] **Step 4: Implement `fetch_backfill_page`**

Same query but with `updated <= backfill_target` only, and the resume clause when `last_seen` is set:

```
AND ((updated > "{last_seen.updated, second precision}")
     OR (updated = "{last_seen.updated, second precision}" AND key > "{last_seen.key}"))
```

- [ ] **Step 5: Implement `list_all_ids`**

Paginate `project = "{key}"&fields=key`, return all keys.

- [ ] **Step 6: Implement `fetch_companions`** — only the Jira-flavoured ones (sprints, fix_versions, projects).

- [ ] **Step 7: Run all jira-connector tests, expect PASS, commit**

```bash
cargo test -p quelch sources::jira
git add crates/quelch/src/sources/jira.rs
git commit -m "feat(sources): jira connector — windows, backfill, companions"
```

### Task 3.3: Re-implement Confluence connector

Same shape as Task 3.2.

**Files:**

- Modify: `crates/quelch/src/sources/confluence.rs`

- [ ] **Step 1: Tests** (use wiremock; CQL filter assertions).

- [ ] **Step 2: Implement** `fetch_window`, `fetch_backfill_page`, `list_all_ids`, `fetch_companions` (just `spaces`).

Per [sync.md "Confluence specifics"](../../sync.md#confluence-specifics): CQL `space = "{key}" AND lastmodified >= "..." AND lastmodified <= "..." ORDER BY lastmodified ASC`. Secondary sort `id ASC`.

The id format is `confluence-{source_name}-{space_key}-{page_id}` per [architecture.md](../../architecture.md#confluence-page-confluence_pages) — important for the move-between-spaces case.

- [ ] **Step 3: Run, PASS, commit**

```bash
cargo test -p quelch sources::confluence
git add crates/quelch/src/sources/confluence.rs
git commit -m "feat(sources): confluence connector — windows, backfill, companions"
```

### Task 3.4: Implement window math

Pure functions — easy to unit-test.

**Files:**

- New: `crates/quelch/src/ingest/window.rs`

- [ ] **Step 1: Tests**

```rust
#[test]
fn next_window_returns_none_when_target_not_advanced() {
    let last = "2026-04-30T14:23:00Z".parse().unwrap();
    let now  = "2026-04-30T14:24:30Z".parse().unwrap();
    let lag = 2;
    assert!(plan_next_window(last, now, lag).is_none());
}

#[test]
fn next_window_starts_at_last_and_ends_at_now_minus_lag() {
    let last = "2026-04-30T14:20:00Z".parse().unwrap();
    let now  = "2026-04-30T14:30:30Z".parse().unwrap();
    let lag = 2;
    let win = plan_next_window(last, now, lag).unwrap();
    assert_eq!(win.start, last);
    assert_eq!(win.end, "2026-04-30T14:28:00Z".parse().unwrap());
}
```

- [ ] **Step 2: Implement `plan_next_window`**

Floor-to-minute, subtract lag minutes, compare to last_complete_minute.

- [ ] **Step 3: Run, PASS, commit**

```bash
cargo test -p quelch ingest::window
git add crates/quelch/src/ingest/window.rs
git commit -m "feat(ingest): minute-window planning"
```

### Task 3.5: Implement rate-limit handler

Wraps reqwest with `reqwest-retry`. Honours `Retry-After`. Exponential backoff on 5xx without `Retry-After`.

**Files:**

- New: `crates/quelch/src/ingest/rate_limit.rs`

- [ ] **Step 1: Test against a mock server that returns 429 with Retry-After**

```rust
#[tokio::test]
async fn honours_retry_after_seconds() {
    let server = wiremock::MockServer::start().await;
    let attempts = Arc::new(AtomicU32::new(0));
    Mock::given(method("GET")).and(path("/x"))
        .respond_with(move |_: &Request| {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                ResponseTemplate::new(429).insert_header("Retry-After", "1")
            } else {
                ResponseTemplate::new(200).set_body_string("ok")
            }
        })
        .mount(&server).await;

    let client = build_rate_limited_client();
    let start = std::time::Instant::now();
    let resp = client.get(format!("{}/x", server.uri())).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert!(start.elapsed() >= Duration::from_secs(1));
}
```

- [ ] **Step 2: Implement `build_rate_limited_client(config: &IngestConfig)`**

Returns `ClientWithMiddleware` that:
- Reads `Retry-After` and sleeps for that long.
- For 5xx without `Retry-After`: exponential 1s, 2s, 4s, 8s, capped at 60s.
- Caps total retries at `config.max_retries`.

- [ ] **Step 3: Run, PASS, commit**

```bash
cargo test -p quelch ingest::rate_limit
git add crates/quelch/src/ingest/rate_limit.rs
git commit -m "feat(ingest): rate-limit aware HTTP client"
```

### Task 3.6: Implement the per-cycle algorithm

This is the centrepiece. Coordinates: read cursor → plan window → fetch pages → upsert to Cosmos → advance cursor.

**Files:**

- New: `crates/quelch/src/ingest/cycle.rs`

- [ ] **Step 1: Write the integration test**

```rust
#[tokio::test]
async fn cycle_advances_cursor_after_full_window() {
    let cosmos = Arc::new(InMemoryCosmos::new());
    let connector = MockConnector::new()
        .with_window(window_a, vec![doc_1, doc_2])
        .with_no_more_pages();
    let key = CursorKey { /* ... */ };

    // initial: no cursor
    let result = cycle::run(&connector, &cosmos, &key, &cycle_config()).await.unwrap();

    assert_eq!(result.documents_written, 2);
    let cursor = meta::load(&*cosmos, "quelch-meta", &key).await.unwrap();
    assert_eq!(cursor.last_complete_minute, Some(window_a.end));
}

#[tokio::test]
async fn cycle_does_not_advance_cursor_on_failure() {
    // Simulate connector returning Err on the second page.
    // Assert cursor unchanged.
}
```

- [ ] **Step 2: Implement `cycle::run`**

```rust
pub async fn run(
    connector: &dyn SourceConnector,
    cosmos: &dyn CosmosBackend,
    key: &CursorKey,
    cfg: &CycleConfig,
) -> anyhow::Result<CycleOutcome> {
    let cursor = meta::load(cosmos, &cfg.meta_container, key).await?;
    if cursor.backfill_in_progress {
        return backfill::resume(connector, cosmos, key, cursor, cfg).await;
    }
    let now = Utc::now();
    let Some(window) = window::plan_next_window(
        cursor.last_complete_minute, now, cfg.safety_lag_minutes
    ) else {
        return Ok(CycleOutcome::NothingToDo);
    };
    let mut total_written = 0;
    let mut page_token = None;
    loop {
        let page = connector
            .fetch_window(&key.subsource, window.start, window.end, cfg.batch_size, page_token.as_deref())
            .await?;
        for doc in page.documents {
            cosmos.upsert(&cfg.target_container, doc.into_cosmos_doc()).await?;
        }
        total_written += page.documents.len();
        match page.next_page_token {
            Some(t) => page_token = Some(t),
            None => break,
        }
    }
    let mut new_cursor = cursor;
    new_cursor.last_complete_minute = Some(window.end);
    new_cursor.documents_synced_total += total_written as u64;
    new_cursor.last_sync_at = Some(Utc::now());
    new_cursor.last_error = None;
    meta::save(cosmos, &cfg.meta_container, key, &new_cursor).await?;
    Ok(CycleOutcome::Advanced { documents_written: total_written, window })
}
```

- [ ] **Step 3: Run all `cycle::tests`, PASS, commit**

```bash
cargo test -p quelch ingest::cycle
git add crates/quelch/src/ingest/cycle.rs
git commit -m "feat(ingest): per-cycle algorithm"
```

### Task 3.7: Implement backfill protocol

**Files:**

- New: `crates/quelch/src/ingest/backfill.rs`

- [ ] **Step 1: Test**

```rust
#[tokio::test]
async fn backfill_resumes_after_crash() {
    // Simulate: page 1 succeeds, page 2 errors; restart;
    // expect resume from last_seen of page 1.
}

#[tokio::test]
async fn backfill_completes_and_clears_flags() { /* ... */ }
```

- [ ] **Step 2: Implement `backfill::start` and `backfill::resume`** following [sync.md "Initial backfill"](../../sync.md#initial-backfill) literally.

- [ ] **Step 3: Run, PASS, commit**

```bash
cargo test -p quelch ingest::backfill
git add crates/quelch/src/ingest/backfill.rs
git commit -m "feat(ingest): initial backfill with crash-resume"
```

### Task 3.8: Implement deletion reconciliation

**Files:**

- New: `crates/quelch/src/ingest/reconcile.rs`

- [ ] **Step 1: Test**

```rust
#[tokio::test]
async fn reconcile_marks_missing_docs_deleted() {
    // Pre-populate cosmos with docs A,B,C.
    // Connector lists [A, C] only.
    // After reconcile, B has _deleted=true and _deleted_at set.
}

#[tokio::test]
async fn reconcile_does_not_touch_already_deleted_docs() { /* ... */ }
```

- [ ] **Step 2: Implement** per [sync.md "Deletions"](../../sync.md#deletions). The list-vs-list diff happens in memory; for very large containers this could be paginated, but the v1 baseline is single-pass.

- [ ] **Step 3: Run, PASS, commit**

```bash
cargo test -p quelch ingest::reconcile
git add crates/quelch/src/ingest/reconcile.rs
git commit -m "feat(ingest): deletion reconciliation with soft-delete"
```

### Task 3.9: Wire `quelch ingest` CLI command

**Files:**

- Modify: `crates/quelch/src/cli.rs`
- Modify: `crates/quelch/src/main.rs`
- New: `crates/quelch/src/ingest/worker.rs` (loop runner)

- [ ] **Step 1: Add the `Ingest` subcommand and the `--deployment` / `--once` / `--max-docs` flags** matching [cli.md](../../cli.md#quelch-ingest).

- [ ] **Step 2: Implement `worker::run(config, deployment_name, options)`**

```rust
pub async fn run(
    config: &Config,
    deployment_name: &str,
    options: WorkerOptions,
) -> anyhow::Result<()> {
    let sliced = config::slice::for_deployment(config, deployment_name)?;
    let cosmos = build_cosmos_backend(&sliced).await?;
    let connectors = build_connectors(&sliced)?;
    let cycle_config = cycle_config_from(&sliced);
    let mut cycle_n: u64 = 0;
    loop {
        cycle_n += 1;
        for (key, connector) in &connectors {
            cycle::run(connector.as_ref(), cosmos.as_ref(), key, &cycle_config).await
                .unwrap_or_else(|e| { tracing::error!(?e, %key, "cycle failed"); CycleOutcome::Failed });
            if cycle_n.is_multiple_of(cycle_config.reconcile_every) {
                reconcile::run(connector.as_ref(), cosmos.as_ref(), key, &cycle_config).await
                    .unwrap_or_else(|e| { tracing::error!(?e, %key, "reconcile failed"); 0 });
            }
        }
        if options.once { break; }
        tokio::time::sleep(cycle_config.poll_interval).await;
    }
    Ok(())
}
```

- [ ] **Step 3: Add an end-to-end test** that runs `worker::run(config, "ingest", WorkerOptions { once: true, .. })` against `MockConnector`s + `InMemoryCosmos` and asserts docs land in the right containers.

- [ ] **Step 4: Run, PASS, commit**

```bash
cargo test -p quelch ingest::worker
git add crates/quelch/src/ingest/worker.rs crates/quelch/src/cli.rs crates/quelch/src/main.rs
git commit -m "feat(cli): quelch ingest worker"
```

### Task 3.10: Delete v1 sync code

The v1 `sync/` engine is superseded.

**Files:**

- Delete: `crates/quelch/src/sync/embedder.rs`
- Delete: `crates/quelch/src/sync/mod.rs`
- Delete: `crates/quelch/src/sync/phases.rs`
- Delete: `crates/quelch/src/sync/state.rs` (after confirming no remaining imports)
- Modify: `crates/quelch/src/lib.rs` (remove `pub mod sync`)

- [ ] **Step 1: Confirm no live imports**

```bash
grep -r "use quelch::sync\|crate::sync" crates/quelch/src/ crates/quelch/tests/
```

If anything remains, fix it (almost certainly `main.rs` and `cli.rs` from Phase 0 leftovers).

- [ ] **Step 2: Delete the files**

```bash
git rm -r crates/quelch/src/sync/
```

- [ ] **Step 3: Run the full pre-push gate**

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

- [ ] **Step 4: Commit**

```bash
git commit -m "chore: remove v1 sync engine (superseded by ingest/)"
```

### Phase 3 acceptance

```bash
cargo test -p quelch ingest
cargo run -p quelch -- ingest --deployment ingest --once
```

Worker reads config, fetches mock data, writes to cosmos, advances cursor, exits cleanly. Phase 3 ships.

---

# Phase 4 — rigg integration (AI Search + Foundry config)

**Goal:** Generate a `rigg/` directory of resource files from `quelch.yaml`, then plan/diff/push them via the embedded `rigg-core` + `rigg-client` libraries.

**Files:**

- New: `crates/quelch/src/azure/rigg/mod.rs`
- New: `crates/quelch/src/azure/rigg/generate.rs` (quelch.yaml → rigg files)
- New: `crates/quelch/src/azure/rigg/plan.rs` (diff via rigg-client)
- New: `crates/quelch/src/azure/rigg/push.rs` (apply)
- New: `crates/quelch/src/azure/rigg/pull.rs` (pull live config back)
- New: `crates/quelch/src/azure/rigg/ownership.rs` (managed-by-user marker handling)
- New: templates for rigg-format YAML (under `crates/quelch/templates/rigg/`)

### Task 4.1: Generate index files

For every exposed data source, generate a rigg index file matching the canonical doc model.

**Files:**

- New: `crates/quelch/src/azure/rigg/generate.rs`
- New: `crates/quelch/templates/rigg/index.yaml.hbs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn generates_jira_issues_index_with_canonical_fields() {
    let cfg = test_helpers::config_with_jira_cloud();
    let files = generate::all(&cfg).unwrap();
    let idx = files.indexes.get("jira-issues").unwrap();
    let yaml: serde_yaml::Value = serde_yaml::from_str(idx).unwrap();
    let fields = yaml["fields"].as_sequence().unwrap();
    assert!(fields.iter().any(|f| f["name"] == "key"));
    assert!(fields.iter().any(|f| f["name"] == "status_category"));
    assert!(fields.iter().any(|f| f["name"] == "issuelinks"));
}
```

- [ ] **Step 2: Author the Handlebars template** at `crates/quelch/templates/rigg/index.yaml.hbs` for a Jira index. Reference [architecture.md "Jira issue"](../../architecture.md#jira-issue-jira_issues) for every field that needs to appear.

- [ ] **Step 3: Implement `generate::all(config) -> GeneratedRiggFiles`**

Walk `config.mcp.data_sources` (after `data_sources::resolve()`); for each, render the appropriate template into a string. Return a struct grouped by resource type.

- [ ] **Step 4: Run, PASS, commit**

```bash
cargo test -p quelch azure::rigg::generate
git add crates/quelch/src/azure/rigg/ crates/quelch/templates/rigg/
git commit -m "feat(rigg): generate index files from quelch.yaml"
```

### Task 4.2: Generate skillset, indexer, datasource, knowledge_source, knowledge_base files

Same pattern as 4.1, one file type at a time.

**Files:**

- New: `crates/quelch/templates/rigg/skillset.yaml.hbs`
- New: `crates/quelch/templates/rigg/indexer.yaml.hbs`
- New: `crates/quelch/templates/rigg/datasource.yaml.hbs`
- New: `crates/quelch/templates/rigg/knowledge_source.yaml.hbs`
- New: `crates/quelch/templates/rigg/knowledge_base.yaml.hbs`

For each:

- [ ] **Step N.1**: Author the template referencing `rigg/crates/rigg-core/src/resources/{type}.rs` for the exact field schema.

- [ ] **Step N.2**: Add a generator function in `generate.rs` calling Handlebars.

- [ ] **Step N.3**: Add a unit test asserting the generated content parses as the right `rigg-core` resource type.

- [ ] **Step N.4**: Commit per resource type.

The skillset template needs to reference `config.openai.endpoint` and `embedding_deployment` for the integrated vectoriser. The indexer points at the data source. Knowledge source wraps the index. Knowledge base groups knowledge sources per `mcp.expose:` set.

### Task 4.3: Implement `quelch.yaml → rigg/` writer

**Files:**

- Modify: `crates/quelch/src/azure/rigg/generate.rs`
- New: `crates/quelch/src/azure/rigg/ownership.rs`

- [ ] **Step 1: Test that `write_to_disk` honours managed-by-user markers**

```rust
#[test]
fn write_to_disk_preserves_managed_by_user_files() {
    let tmp = tempfile::tempdir().unwrap();
    // pre-create rigg/indexes/jira-issues.yaml with marker
    fs::create_dir_all(tmp.path().join("indexes")).unwrap();
    fs::write(tmp.path().join("indexes/jira-issues.yaml"),
              "# rigg:managed-by-user\nfields: [{name: foo, type: Edm.String}]").unwrap();

    let files = test_helpers::generated_files();
    write_to_disk(&files, tmp.path()).unwrap();

    // Existing file untouched
    let content = fs::read_to_string(tmp.path().join("indexes/jira-issues.yaml")).unwrap();
    assert!(content.contains("rigg:managed-by-user"));
    assert!(content.contains("name: foo"));
}
```

- [ ] **Step 2: Implement `ownership::is_managed_by_user(path) -> bool`** — reads first line, returns true if it matches `# rigg:managed-by-user`.

- [ ] **Step 3: Implement `write_to_disk(files, root)`** — for each file, skip if managed-by-user, else write/overwrite.

- [ ] **Step 4: Run, PASS, commit**

```bash
cargo test -p quelch azure::rigg::ownership azure::rigg::generate::write_to_disk
git add crates/quelch/src/azure/rigg/
git commit -m "feat(rigg): write generated files to disk with hand-takeover support"
```

### Task 4.4: Implement plan via rigg-client

**Files:**

- New: `crates/quelch/src/azure/rigg/plan.rs`

- [ ] **Step 1: Test against a mock rigg-client (or wiremock for the AI Search REST API)**

The exact call surface comes from `rigg-client/src/lib.rs`. You'll be using `AzureSearchClient::list_indexes` etc. against a mock and computing diffs locally.

- [ ] **Step 2: Implement `plan(rigg_dir, search_endpoint) -> PlanReport`**

```rust
pub struct PlanReport {
    pub creates: Vec<ResourceRef>,
    pub updates: Vec<(ResourceRef, Diff)>,
    pub deletes: Vec<ResourceRef>,
    pub unchanged: Vec<ResourceRef>,
}

pub async fn plan(rigg_dir: &Path, endpoint: &str) -> Result<PlanReport, PlanError> {
    let local = read_local_files(rigg_dir)?;
    let live = fetch_live(endpoint).await?;
    Ok(compute_diff(local, live))
}
```

- [ ] **Step 3: Run, PASS, commit**

```bash
cargo test -p quelch azure::rigg::plan
git add crates/quelch/src/azure/rigg/plan.rs
git commit -m "feat(rigg): plan/diff via rigg-client"
```

### Task 4.5: Implement push and pull

**Files:**

- New: `crates/quelch/src/azure/rigg/push.rs`
- New: `crates/quelch/src/azure/rigg/pull.rs`

For each:

- [ ] **Step N.1**: Test that `push(plan_report)` calls the right rigg-client methods.

- [ ] **Step N.2**: Implement.

- [ ] **Step N.3**: For pull, write into the local `rigg/` dir, *skipping managed-by-user files*. Add `--diff` mode that shows what would change without writing.

- [ ] **Step N.4**: Commit.

### Phase 4 acceptance

```bash
cargo test -p quelch azure::rigg
```

All tests pass. With a real Azure account env-gated, `quelch azure plan` should round-trip against an empty resource group.

---

# Phase 5 — Bicep generator and `quelch azure plan/deploy`

**Goal:** Generate `.quelch/azure/<deployment>.bicep` from the config, run `az deployment group what-if`, present the combined Bicep+rigg diff, apply on confirmation.

**Files:**

- New: `crates/quelch/src/azure/deploy/mod.rs`
- New: `crates/quelch/src/azure/deploy/bicep.rs` (generation)
- New: `crates/quelch/src/azure/deploy/whatif.rs` (parse `az deployment group what-if` output)
- New: `crates/quelch/src/azure/deploy/apply.rs`
- New: `crates/quelch/src/azure/deploy/diff_view.rs` (combined Bicep+rigg diff renderer)
- New: `crates/quelch/templates/bicep/main.bicep.hbs`
- New: `crates/quelch/templates/bicep/cosmos.bicep.hbs`
- New: `crates/quelch/templates/bicep/search.bicep.hbs`
- New: `crates/quelch/templates/bicep/keyvault.bicep.hbs`
- New: `crates/quelch/templates/bicep/containerapps.bicep.hbs`
- New: `crates/quelch/templates/bicep/identities.bicep.hbs`

### Task 5.1: Generate top-level Bicep

**Files:**

- New: `crates/quelch/src/azure/deploy/bicep.rs`

- [ ] **Step 1: Snapshot test**

```rust
#[test]
fn generates_minimal_bicep_for_two_deployments() {
    let cfg = test_helpers::config_with_two_deployments();
    let bicep = bicep::generate(&cfg, "mcp").unwrap();
    insta::assert_snapshot!(bicep);
}
```

(Add `insta = "1"` to dev-deps for golden-file testing.)

- [ ] **Step 2: Author each template** (`main.bicep.hbs` includes the others as `module` references). Reference [deployment.md "What gets created"](../../deployment.md#what-gets-created) for the exact resource set.

- [ ] **Step 3: Implement `bicep::generate(config, deployment_name)`**

- [ ] **Step 4: Run, review the snapshot, accept it**

```bash
cargo test -p quelch azure::deploy::bicep
cargo insta review
```

- [ ] **Step 5: Commit (including the accepted snapshot)**

```bash
git add crates/quelch/src/azure/deploy/bicep.rs crates/quelch/templates/bicep/ \
        crates/quelch/src/snapshots/
git commit -m "feat(bicep): generate top-level + module Bicep from config"
```

### Task 5.2: Wrap `az deployment group what-if`

**Files:**

- New: `crates/quelch/src/azure/deploy/whatif.rs`

- [ ] **Step 1: Test against a captured what-if JSON output** (Azure docs include sample output; capture into `tests/fixtures/whatif_sample.json`).

- [ ] **Step 2: Implement `whatif::run(rg, bicep_path) -> WhatIfReport`** that shells out to `az` and parses the JSON.

```rust
pub fn run(resource_group: &str, bicep_path: &Path) -> Result<WhatIfReport, WhatIfError> {
    let output = std::process::Command::new("az")
        .args(["deployment", "group", "what-if",
               "--resource-group", resource_group,
               "--template-file", bicep_path.to_str().unwrap(),
               "--no-pretty-print"])
        .output()?;
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    parse_whatif(&json)
}
```

- [ ] **Step 3: Run, PASS, commit**

```bash
git add crates/quelch/src/azure/deploy/whatif.rs
git commit -m "feat(bicep): wrap az deployment group what-if"
```

### Task 5.3: Combined diff view

Render Bicep what-if + rigg plan in a single human-readable summary per [deployment.md "Reading a combined diff"](../../deployment.md#reading-a-combined-diff).

**Files:**

- New: `crates/quelch/src/azure/deploy/diff_view.rs`

- [ ] **Step 1: Snapshot test** with mocked Bicep + rigg inputs.

- [ ] **Step 2: Implement `render(bicep_report, rigg_report) -> String`**.

- [ ] **Step 3: Commit.**

### Task 5.4: Wire `quelch azure plan` and `quelch azure deploy`

**Files:**

- Modify: `crates/quelch/src/cli.rs`
- Modify: `crates/quelch/src/main.rs`

- [ ] **Step 1: Add `Azure { command: AzureCommands }` to clap** with `Plan { deployment: Option<String> }`, `Deploy { deployment: Option<String>, yes: bool, dry_run: bool }`, etc.

- [ ] **Step 2: Implement the handlers** — `plan` runs Bicep generate + rigg generate + Bicep what-if + rigg plan, then renders. `deploy` does the same plus prompts and applies.

- [ ] **Step 3: Add an integration test that pins a known config and asserts the rendered plan**.

- [ ] **Step 4: Commit.**

### Task 5.5: `quelch azure indexer` and `quelch azure logs`

**Files:**

- New: `crates/quelch/src/azure/deploy/indexer.rs` (shells `az search ... indexer ...`)
- New: `crates/quelch/src/azure/deploy/logs.rs` (shells `az containerapp logs show`)

- [ ] **Step 1: Tests**

- [ ] **Step 2: Implement** — these are thin `az` wrappers per [cli.md](../../cli.md#quelch-azure-logs-deployment).

- [ ] **Step 3: Wire into CLI**.

- [ ] **Step 4: Commit.**

### Phase 5 acceptance

```bash
cargo test -p quelch azure::deploy
cargo run -p quelch -- azure plan --no-what-if  # synthesises Bicep + rigg only
```

Plans are emitted; rigg files written; `.quelch/azure/<deployment>.bicep` exists. Phase 5 ships.

---

# Phase 6 — MCP server

**Goal:** A working `quelch mcp` server that speaks Streamable HTTP, exposes the five tools, and routes to Cosmos / AI Search / Knowledge Base appropriately.

**Files:**

- New: `crates/quelch/src/mcp/mod.rs`
- New: `crates/quelch/src/mcp/server.rs` (axum app)
- New: `crates/quelch/src/mcp/transport.rs` (Streamable HTTP framing)
- New: `crates/quelch/src/mcp/auth.rs` (API key v1, Entra v1.x)
- New: `crates/quelch/src/mcp/tools/mod.rs`
- New: `crates/quelch/src/mcp/tools/search.rs`
- New: `crates/quelch/src/mcp/tools/query.rs`
- New: `crates/quelch/src/mcp/tools/get.rs`
- New: `crates/quelch/src/mcp/tools/list_sources.rs`
- New: `crates/quelch/src/mcp/tools/aggregate.rs`
- New: `crates/quelch/src/mcp/filter/mod.rs` (where-grammar parser)
- New: `crates/quelch/src/mcp/filter/cosmos_sql.rs` (where → Cosmos SQL)
- New: `crates/quelch/src/mcp/filter/odata.rs` (where → AI Search OData)
- New: `crates/quelch/src/mcp/expose.rs` (data-source resolution + 403 enforcement)

### Task 6.1: Implement the where-grammar parser

The unified `where` grammar from [mcp-api.md "Filter grammar"](../../mcp-api.md#filter-grammar) is the heart of the MCP layer.

**Files:**

- New: `crates/quelch/src/mcp/filter/mod.rs`

- [ ] **Step 1: Tests** — one per grammar shape (equality, membership, comparison, dates, like, not, nested, and/or, exists, array projection, array_match).

```rust
#[test]
fn parses_equality() {
    let ast = parse(json!({ "status": "Open" })).unwrap();
    assert_eq!(ast, Where::Field { path: "status".into(), op: Op::Eq, value: "Open".into() });
}

#[test]
fn parses_array_projection() {
    let ast = parse(json!({ "fix_versions[].name": "iXX-2.7.0" })).unwrap();
    // assert AST shape
}
```

- [ ] **Step 2: Implement `parse(serde_json::Value) -> Result<Where, FilterError>`**

The AST:

```rust
pub enum Where {
    Field { path: FieldPath, op: Op, value: Value },
    And(Vec<Where>),
    Or(Vec<Where>),
    Not(Box<Where>),
    Exists { path: FieldPath, present: bool },
    ArrayMatch { path: FieldPath, predicate: Box<Where> },
}
```

- [ ] **Step 3: Run, PASS, commit**

```bash
cargo test -p quelch mcp::filter
git add crates/quelch/src/mcp/filter/mod.rs
git commit -m "feat(mcp): unified where-grammar parser"
```

### Task 6.2: Translate where → Cosmos SQL

**Files:**

- New: `crates/quelch/src/mcp/filter/cosmos_sql.rs`

- [ ] **Step 1: Tests covering each AST node → Cosmos SQL fragment.** Include the array-projection case (`ARRAY_CONTAINS(c.fix_versions, {"name": "iXX-2.7.0"}, true)` style).

- [ ] **Step 2: Implement `to_cosmos_sql(where) -> (String, Vec<(String, Value)>)`** returning the SQL fragment and parameterised values.

- [ ] **Step 3: Layer in soft-delete default**: every translated query gets `(NOT IS_DEFINED(c._deleted) OR c._deleted = false)` AND-ed unless `include_deleted: true`.

- [ ] **Step 4: Run, PASS, commit**

```bash
cargo test -p quelch mcp::filter::cosmos_sql
git add crates/quelch/src/mcp/filter/cosmos_sql.rs
git commit -m "feat(mcp): where → Cosmos SQL translator"
```

### Task 6.3: Translate where → AI Search OData

**Files:**

- New: `crates/quelch/src/mcp/filter/odata.rs`

Similar to Task 6.2 but emits AI Search `$filter` syntax. Note: AI Search uses `/` for nested fields (not `.`) and `any()`/`all()` for array projection. Tests should cover the differences.

- [ ] Steps 1–4 mirror 6.2.

```bash
git commit -m "feat(mcp): where → AI Search OData translator"
```

### Task 6.4: Implement `query` tool

**Files:**

- New: `crates/quelch/src/mcp/tools/query.rs`

- [ ] **Step 1: Test against InMemoryCosmos**

```rust
#[tokio::test]
async fn query_returns_matching_docs() {
    let cosmos = build_cosmos_with_test_data().await;
    let result = query::run(&cosmos, &expose_config(),
        QueryRequest {
            data_source: "jira_issues".into(),
            where_: Some(json!({ "status": "Open" })),
            ..Default::default()
        }
    ).await.unwrap();
    assert_eq!(result.total, 3);
}

#[tokio::test]
async fn query_excludes_soft_deleted_by_default() { /* ... */ }

#[tokio::test]
async fn query_with_include_deleted_returns_tombstoned() { /* ... */ }
```

- [ ] **Step 2: Implement `query::run`** per [mcp-api.md `query`](../../mcp-api.md#query). Resolves `data_source` to backing containers via `expose::resolve_data_source`, fans out to each, merges with `order_by` semantics, returns the page.

- [ ] **Step 3: Implement cursor pagination** — encode the continuation tokens of all backing containers into a single base64 cursor string.

- [ ] **Step 4: Run, PASS, commit**

```bash
cargo test -p quelch mcp::tools::query
git add crates/quelch/src/mcp/tools/query.rs
git commit -m "feat(mcp): query tool"
```

### Task 6.5: Implement `get`, `aggregate`, `list_sources`

**Files:**

- New: `crates/quelch/src/mcp/tools/get.rs`
- New: `crates/quelch/src/mcp/tools/aggregate.rs`
- New: `crates/quelch/src/mcp/tools/list_sources.rs`

For each tool:

- [ ] **Step N.1**: Tests, including the soft-delete defaults for `get`, the array-field group_by semantics for `aggregate` (per [mcp-api.md "Aggregation over array fields"](../../mcp-api.md#aggregation-over-array-fields)), and the schema introspection for `list_sources`.

- [ ] **Step N.2**: Implement.

- [ ] **Step N.3**: Commit.

For `list_sources`: returns `data_sources[]` only — the agent never sees container names. For each data source, include schema (read from a static schema-table compiled per source-type at startup) and `source_instances` (computed from `mcp.data_sources.backed_by`).

### Task 6.6: Implement `search` tool

**Files:**

- New: `crates/quelch/src/mcp/tools/search.rs`

The trickiest tool — routes through Knowledge Base by default, falls back to direct hybrid search when `disable_agentic` is set, supports three `include_content` modes.

- [ ] **Step 1: Tests**

```rust
#[tokio::test]
async fn search_routes_through_knowledge_base() {
    // wiremock the AI Search Knowledge Base API; assert the request went there
}

#[tokio::test]
async fn search_disable_agentic_routes_through_index() { /* ... */ }

#[tokio::test]
async fn search_include_content_full_returns_body() { /* ... */ }

#[tokio::test]
async fn search_include_content_agentic_answer_returns_answer_field() { /* ... */ }
```

- [ ] **Step 2: Implement `search::run`** using `rigg-client::AzureSearchClient` for both KB and direct paths (rigg-client should already wrap both APIs; confirm and extend if needed).

- [ ] **Step 3: Run, PASS, commit**

```bash
cargo test -p quelch mcp::tools::search
git add crates/quelch/src/mcp/tools/search.rs
git commit -m "feat(mcp): search tool with KB routing"
```

### Task 6.7: HTTP transport (Streamable HTTP)

**Files:**

- New: `crates/quelch/src/mcp/transport.rs`
- New: `crates/quelch/src/mcp/server.rs`

- [ ] **Step 1: Read the latest MCP Streamable HTTP spec** ([modelcontextprotocol.io](https://modelcontextprotocol.io/) — verify the current transport name and message format).

- [ ] **Step 2: Implement axum routes** for `POST /mcp` (request), GET for SSE streaming when needed.

- [ ] **Step 3: Add an end-to-end test** using `axum::Server::serve` against a `TestClient`.

- [ ] **Step 4: Commit.**

### Task 6.8: Auth middleware

**Files:**

- New: `crates/quelch/src/mcp/auth.rs`

- [ ] **Step 1: API-key middleware** that compares `Authorization: Bearer <key>` against the configured key (read from env var, populated from Key Vault by Container Apps secret reference).

- [ ] **Step 2: Stub Entra middleware** behind a `cfg(feature = "entra")` so it can be turned on later.

- [ ] **Step 3: Tests**: missing/wrong key → 401, correct key → through.

- [ ] **Step 4: Commit.**

### Task 6.9: Wire `quelch mcp` CLI command

**Files:**

- Modify: `crates/quelch/src/cli.rs`
- Modify: `crates/quelch/src/main.rs`

- [ ] **Step 1: Add `Mcp { deployment, port, bind, api_key }` subcommand**.

- [ ] **Step 2: Implement handler** that loads sliced config, builds backends, wires axum, listens.

- [ ] **Step 3: End-to-end test**: spin up the server, hit it with reqwest, assert tool calls work.

- [ ] **Step 4: Commit.**

### Phase 6 acceptance

```bash
cargo test -p quelch mcp
cargo run -p quelch -- mcp --deployment mcp --port 8080
# in another shell:
curl -H "Authorization: Bearer dev" -X POST http://localhost:8080/mcp \
  -d '{"method": "tools/call", "params": {"name": "list_sources", "arguments": {}}}'
```

Returns the data-source list. Phase 6 ships.

---

# Phase 7 — Operator CLI completeness

**Goal:** Fill in every remaining `quelch ...` command from [cli.md](../../cli.md).

**Files:**

- Modify: `crates/quelch/src/cli.rs`
- Modify: `crates/quelch/src/main.rs`
- New: `crates/quelch/src/commands/{status, query, search, get, reset}.rs`

### Task 7.1: `quelch status` (and `--tui` later in Phase 10)

- [ ] Reads `quelch-meta` for the active config; prints a table per (source, subsource).

- [ ] Add `--deployment`, `--json`.

- [ ] Tests against InMemoryCosmos populated with mock cursors.

- [ ] Commit.

### Task 7.2: `quelch query`, `quelch search`, `quelch get`

These are CLI wrappers that load the sliced config for the active deployment (or use the first `mcp` deployment if `--deployment` is omitted), build the same backends the MCP server uses, and call the same tool functions.

**Files:**

- New: `crates/quelch/src/commands/query.rs`, `search.rs`, `get.rs`

- [ ] Tests: each command parses `--data-source`, `--where` (parses JSON), etc., and routes to the tool function correctly.

- [ ] Commit per command.

### Task 7.3: `quelch reset`

Per [cli.md "quelch reset"](../../cli.md#quelch-reset).

- [ ] Reads cursor docs in `quelch-meta` matching the source/subsource filter, deletes them (or sets `last_complete_minute = null`).

- [ ] `--yes` to skip prompt.

- [ ] Tests.

- [ ] Commit.

### Task 7.4: `quelch azure pull`

Per [deployment.md "quelch azure pull"](../../deployment.md#quelch-azure-pull).

- [ ] Wraps `azure::rigg::pull::run`.

- [ ] `[<resource>]` positional arg restricts to one resource type.

- [ ] `--diff` mode shows what would change without writing.

- [ ] Commit.

### Task 7.5: `quelch azure destroy`

- [ ] Removes a single Container App; leaves shared infra alone.

- [ ] Confirmation prompt.

- [ ] Tests against mocked `az`.

- [ ] Commit.

### Phase 7 acceptance

Every command listed in [cli.md command tree](../../cli.md#command-tree) responds to `--help` with the documented flags. Smoke tests pass.

---

# Phase 8 — Agent and skill bundle generator

**Goal:** Implement `quelch agent generate` for all six targets per [agent-generation.md](../../agent-generation.md).

**Files:**

- New: `crates/quelch/src/agent/mod.rs`
- New: `crates/quelch/src/agent/bundle.rs` (shared content: tool reference, schema cheatsheet, how-tos, prompts)
- New: `crates/quelch/src/agent/targets/copilot_studio.rs`
- New: `crates/quelch/src/agent/targets/claude_code.rs`
- New: `crates/quelch/src/agent/targets/copilot_cli.rs`
- New: `crates/quelch/src/agent/targets/vscode_copilot.rs`
- New: `crates/quelch/src/agent/targets/codex.rs`
- New: `crates/quelch/src/agent/targets/markdown.rs`
- Repurpose: `crates/quelch/src/copilot.rs` becomes the `copilot-studio` target's implementation
- New: templates under `crates/quelch/templates/agent/`

### Task 8.1: Build the shared bundle content

**Files:**

- New: `crates/quelch/src/agent/bundle.rs`

- [ ] **Step 1: Test that schema cheatsheet generation reflects exposed data sources only.**

- [ ] **Step 2: Implement** — read `mcp.expose:` for the active deployment, render the schema-cheatsheet markdown, render the tool-reference markdown, render the how-tos markdown (matching the patterns in [agent-generation.md "Domain how-tos"](../../agent-generation.md#4-domain-how-tos)), generate example prompts from `examples.md`.

- [ ] **Step 3: Commit.**

### Task 8.2: Implement each target

For each target file:

- [ ] **Step N.1**: Snapshot test of the bundle structure against `tests/fixtures/quelch.standard.yaml`.

- [ ] **Step N.2**: Implement — assembles the per-target file structure from [agent-generation.md "Per-target packaging"](../../agent-generation.md#per-target-packaging).

- [ ] **Step N.3**: Commit.

The Copilot Studio target reuses much of v1's `copilot.rs` but is reorganised to share bundle content with the others.

### Task 8.3: Wire `quelch agent generate` CLI

- [ ] Subcommand with `--target`, `--format [agent|skill|both]`, `--output`.

- [ ] Format auto-default per target.

- [ ] Integration test: run against a fixture config, assert the output directory contains the expected files.

- [ ] Commit.

### Phase 8 acceptance

```bash
quelch agent generate --target claude-code --output /tmp/bundle
ls -la /tmp/bundle
```

Bundle contains expected files. Phase 8 ships.

---

# Phase 9 — On-prem deploy generator + `quelch init` wizard

**Goal:** Two operator conveniences. `generate-deployment` produces docker-compose / systemd / k8s artefacts; `quelch init` runs an interactive scaffolder using `az` to discover existing Azure resources.

**Files:**

- New: `crates/quelch/src/onprem/mod.rs`
- New: `crates/quelch/src/onprem/docker.rs`
- New: `crates/quelch/src/onprem/systemd.rs`
- New: `crates/quelch/src/onprem/k8s.rs`
- New: `crates/quelch/templates/onprem/{docker-compose.yaml.hbs, *.service.hbs, deployment.yaml.hbs, ...}`
- New: `crates/quelch/src/init/mod.rs` (interactive wizard)

### Task 9.1: `quelch generate-deployment`

For each target (docker, systemd, k8s):

- [ ] Snapshot test of generated artefacts.

- [ ] Implement template rendering.

- [ ] Output directory always contains: artefact + `effective-config.yaml` + `.env.example` + `README.md`.

- [ ] Commit per target.

### Task 9.2: `quelch init` wizard

Per [cli.md "quelch init"](../../cli.md#quelch-init).

**Files:**

- New: `crates/quelch/src/init/mod.rs`

- [ ] **Step 1: Implement `discover::azure()`** — shells `az account list`, `az group list`, `az resource list` to discover Cosmos / AI Search / OpenAI accounts.

- [ ] **Step 2: Implement the prompt loop** — uses `dialoguer` (add to deps) for interactive selection.

- [ ] **Step 3: Implement source-credential testing** — for each configured source, attempt a no-op API call and verify auth.

- [ ] **Step 4: Render quelch.yaml** from the collected answers.

- [ ] **Step 5: Tests** with `dialoguer`'s test mode.

- [ ] **Step 6: Commit.**

### Phase 9 acceptance

```bash
quelch generate-deployment ingest-onprem --target docker --output /tmp/d
quelch init --force      # in an empty directory
```

Both work end-to-end.

---

# Phase 10 — TUI refocus, dev mode, release CI

**Goal:** Update v1's TUI to be the default UX of `quelch dev` and a fleet dashboard for `quelch status --tui`. Update `quelch dev` to use the in-memory backends from Phase 2. Update GitHub Actions release workflow to build and publish the container image.

**Files:**

- Modify: `crates/quelch/src/tui/*` (refocus events to the new architecture)
- New: `crates/quelch/src/dev/mod.rs` (the embedded sim+ingest+mcp runner)
- Modify: `.github/workflows/release.yml` (add ghcr.io build + push)
- New: `Dockerfile`

### Task 10.1: Refocus the TUI to read from `quelch-meta`

**Files:**

- Modify: `crates/quelch/src/tui/status.rs`
- Modify: `crates/quelch/src/tui/widgets/source_table.rs`

- [ ] **Step 1: Replace event sources** — instead of reading `tracing` events from a single ingest worker, the TUI now polls `cosmos::meta::list_all` every few seconds and renders the cursor states.

- [ ] **Step 2: Tests** — TUI snapshot tests using `ratatui::backend::TestBackend` and `insta`.

- [ ] **Step 3: Commit.**

### Task 10.2: `quelch dev` with in-memory backends

**Files:**

- New: `crates/quelch/src/dev/mod.rs`

- [ ] **Step 1: Build** — spawn the simulator, in-memory Cosmos, embedded MCP server, ingest worker, all in one process. Wire the TUI as the front-end.

- [ ] **Step 2: Add `--use-real-search`, `--use-cosmos-emulator`** flags.

- [ ] **Step 3: End-to-end test** — `quelch dev --once --no-tui` runs a finite simulation and asserts a doc made it through ingest → cosmos → mcp.

- [ ] **Step 4: Commit.**

### Task 10.3: Build and publish the Container image

**Files:**

- New: `Dockerfile`
- Modify: `.github/workflows/release.yml`

- [ ] **Step 1: Author Dockerfile** — multi-stage `cargo chef` build, distroless runtime image with the `quelch` binary.

- [ ] **Step 2: Test locally** — `docker build -t quelch:test . && docker run --rm quelch:test --version`.

- [ ] **Step 3: Add a release-workflow job** that builds and pushes to `ghcr.io/mklab-se/quelch:<version>` using the existing version-tag trigger.

- [ ] **Step 4: Commit.**

### Phase 10 acceptance

```bash
cargo run -p quelch -- dev --once --no-tui   # exits cleanly
docker build -t quelch:test .                # passes
```

---

# Phase 11 — External-project changes (rigg, ailloy)

These changes happen in their own repos but coordinate with quelch's release.

### Task 11.1: rigg — add anything quelch needs

If during Phase 4 you discovered missing rigg features, address them here.

**Files in `/Users/kristofer/repos/rigg/`:**

- Modify: `rigg-core/src/resources/*.rs` (any missing fields)
- Modify: `rigg-client/src/client.rs` (any missing API methods)

- [ ] **Step 1: Audit** — re-read [architecture.md](../../architecture.md), [deployment.md](../../deployment.md), and [mcp-api.md](../../mcp-api.md). For each rigg-managed resource type, confirm rigg can read/write/diff it.

- [ ] **Step 2: Add missing pieces** test-first.

- [ ] **Step 3: Bump rigg version, publish, and bump the dep in quelch.**

If audit finds nothing missing, this task is a no-op — skip and move on.

### Task 11.2: ailloy — no changes expected

Quelch v2 does *not* use ailloy on the ingest path (Azure AI Search owns embeddings). ailloy stays as a dependency for future AI features in Quelch itself.

Nothing to do unless audit during Phase 0 surfaced an issue.

### Task 11.3: Coordinated release notes

Once quelch v2 is ready to ship:

- [ ] Update CHANGELOG.md in all three repos.

- [ ] Tag rigg, ailloy, quelch versions in lockstep.

- [ ] Verify Homebrew tap formulas pin compatible versions.

- [ ] Commit and tag.

---

# Self-review checklist

Run through this before declaring the plan complete:

| Spec section | Phase(s) covering it |
|---|---|
| Decision 1 (embeddings in AI Search) | 4 (rigg generates skillsets), 6 (search routes through KB) |
| Decision 2 (per-source-type containers, overridable) | 1 (config schema), 4 (rigg index per container) |
| Decision 3 (config = source of truth; Bicep+rigg generated) | 5 (Bicep), 4 (rigg) |
| Decision 4 (5 MCP tools speaking data_sources) | 6 (all five tools) |
| Decision 5 (state in Cosmos) | 2 (cosmos::meta), 3 (cycle uses meta) |
| Decision 6 (Bicep + az shell-outs) | 5 (deploy, plan, indexer, logs) |
| Decision 7 (Container Apps from ghcr.io) | 5 (Bicep), 10 (Dockerfile, release CI) |
| Decision 8 (on-prem = generate, not manage) | 9 (generate-deployment) |
| Decision 9 (API key v1, Entra v1.x) | 6 (auth.rs) |
| Decision 10 (keep TUI/sim/mock) | 10 (TUI refocus, dev mode) |
| Decision 11 (agent generation first-class) | 8 (all six targets) |
| Decision 12 (sync correctness algorithm) | 3 (cycle, backfill, reconcile, window, rate_limit) |
| Decision 13 (rigg as embedded library) | 4 (entire phase) |
| Open: filter grammar locked down | 6 (Tasks 6.1–6.3 with concrete tests) |
| Open: MCP Streamable HTTP transport | 6 (Task 6.7) |
| Open: Bicep modules layout | 5 (Task 5.1, file layout) |
| Open: deferred queue-based ingestion | Not implemented; no task — design seam preserved by Task 3.6's connector→cosmos channel pattern |

Examples (from [examples.md](../../examples.md)):

| Example | Phase exercising it |
|---|---|
| 1 (all my user stories) | 6 (query) — covered by Task 6.4 tests |
| 2 (count by status) | 6 (aggregate) |
| 3 (camera connection issues) | 6 (search KB routing, paginate) |
| 4 (next sprint contents) | 6 (query against jira_sprints + jira_issues) |
| 5 (sprint goal summary) | 6 + agent synthesis (no MCP change) |
| 6 (work left in sprint) | 6 (aggregate sum) |
| 7 (top 3 risks) | 6 (query + query + search combined) |
| 8 (release notes lookup) | 6 (query fix_versions + search confluence_pages) |
| 9 (way of working) | 6 (search + get) |
| 10 (top 5 blockers, group_by labels) | 6 (aggregate array group_by — Task 6.5) |
| 11 (summarise across both) | 6 (search include_content=full) |
| 12 (recent activity) | 6 (query with updated >= "1h ago") |
| 13 (stalled work, status_category) | 6 (query) |
| 14 (release notes for iXX-2.7.0) | 6 (query with fix_versions[].name) |
| 15 (what's blocking DO-1182) | 6 (get + query with key:[]) |
| 16 (discovery) | 6 (list_sources + query metadata sources) |
| 17 (recently deleted) | 6 (query with include_deleted) |

Every example has at least one phase that implements the underlying capability.

---

## Execution recommendation

**Recommended execution: subagent-driven, one phase at a time, review at phase boundaries.**

Each phase is internally shippable. After Phase 3 you have an ingest worker that writes to Cosmos. After Phase 6 you have a working MCP server. After Phase 10 you have the full operator experience.

If a phase reveals an unexpected complication, stop and update the plan before continuing — better to revise the plan than to execute a wrong one.
