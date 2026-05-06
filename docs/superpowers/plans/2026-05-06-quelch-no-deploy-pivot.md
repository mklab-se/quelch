# Quelch no-deploy pivot — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Strip Quelch of all auto-deploy paths (Bicep, Container Apps, on-disk rigg files) and refocus it on configuring Azure resources + emitting per-instance config files the user runs themselves anywhere.

**Architecture:** One YAML schema (master + slimmed per-instance), `quelch azure apply` configures Cosmos containers via ARM REST and AI Search via rigg-as-library, `quelch instance config` emits per-instance files, the user hosts Q-Ingest / Q-MCP. Conflict prevention is static (validate) + dynamic (cursor ownership).

**Tech Stack:** Rust 2024, tokio, reqwest (rustls-tls-native-roots), serde / serde_yaml, clap, ratatui, tracing, rigg-core (workspace dep), azure_identity for `DefaultAzureCredential`.

**Spec:** `docs/superpowers/specs/2026-05-06-quelch-no-deploy-pivot-design.md`

---

## Working agreements

- **TDD:** every behaviour change starts with a failing test. Pure deletions and doc rewrites get a verify-by-build / verify-by-render step instead of a test.
- **Pre-commit gate:** before every commit run `cargo fmt --all`, then `cargo clippy --workspace -- -D warnings`, then `cargo test --workspace`. All three must pass. If any fails, fix before committing.
- **No backwards compatibility:** delete cleanly, don't deprecate.
- **Comments:** default to none. The spec is the design doc; the code shouldn't repeat it.
- **No `.unwrap()`** in library code; propagate errors with `?`.

---

## Phase 0 — Baseline

### Task 0.1: Establish baseline

**Files:** none.

- [ ] **Step 1: Verify the working tree is clean and on `main`**

```bash
git status            # expect: nothing to commit, working tree clean
git rev-parse --abbrev-ref HEAD  # expect: main
```

- [ ] **Step 2: Run the full pre-push gate to confirm we're starting green**

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

If anything is red on `main`, stop and report — the plan assumes a green starting point.

- [ ] **Step 3: Create a feature branch**

```bash
git checkout -b feat/no-deploy-pivot
```

---

## Phase 1 — Demolition

The goal of this phase is to delete every line of code and config that the spec says is gone, and end with a buildable tree. We accept temporary CLI gaps (commands that error with a clear "rewriting" message) — they get filled in by later phases.

### Task 1.1: Delete the `azure/deploy/` directory

**Files:**
- Delete: `crates/quelch/src/azure/deploy/` (whole directory, including `snapshots/`)

- [ ] **Step 1: Delete the directory**

```bash
git rm -r crates/quelch/src/azure/deploy/
```

- [ ] **Step 2: Update `crates/quelch/src/azure/mod.rs` to drop the `pub mod deploy;` line and any `pub use deploy::...` re-exports**

After the edit, the file should declare only the modules that survive: `pub mod rigg;` and (later) `pub mod apply;`, `pub mod plan;`, `pub mod cosmos_config;`, `pub mod indexer;`. For now keep just `pub mod rigg;`.

- [ ] **Step 3: Verify it compiles up to caller errors**

```bash
cargo build --workspace 2>&1 | head -100
```

Expected: errors in `cli.rs` / `commands/` / `main.rs` referencing deleted symbols. That's fine — the next tasks fix them.

- [ ] **Step 4: Commit (after later tasks make it build green)**

Don't commit yet — Phase 1 commits as a single unit at Task 1.7.

---

### Task 1.2: Delete on-disk rigg modules

**Files:**
- Delete: `crates/quelch/src/azure/rigg/write.rs`
- Delete: `crates/quelch/src/azure/rigg/pull.rs`
- Delete: `crates/quelch/src/azure/rigg/ownership.rs`

- [ ] **Step 1: Delete the files**

```bash
git rm crates/quelch/src/azure/rigg/write.rs \
       crates/quelch/src/azure/rigg/pull.rs \
       crates/quelch/src/azure/rigg/ownership.rs
```

- [ ] **Step 2: Update `crates/quelch/src/azure/rigg/mod.rs`**

Drop the `pub mod write;`, `pub mod pull;`, `pub mod ownership;` declarations and any re-exports. Surviving submodules: `pub mod generate;`, `pub mod plan;`, `pub mod push;`. (Their internals get refactored in Phase 4 — for now they likely don't compile because `generate.rs` may import from the deleted modules; comment out or `unimplemented!()` the offending calls so the file at least parses. Phase 4 fixes this.)

- [ ] **Step 3: Verify it parses**

```bash
cargo check -p quelch 2>&1 | head -60
```

Errors are expected; what we don't want is a parse error in `rigg/mod.rs` itself.

---

### Task 1.3: Delete `onprem/`

**Files:**
- Delete: `crates/quelch/src/onprem/` (whole directory)

- [ ] **Step 1: Delete the directory**

```bash
git rm -r crates/quelch/src/onprem/
```

- [ ] **Step 2: Drop `pub mod onprem;` from `crates/quelch/src/lib.rs` (or `main.rs`, wherever it's declared)**

```bash
grep -rn "pub mod onprem" crates/quelch/src/
```

Edit the matching line(s) to remove them. Also drop any `use crate::onprem::...` calls — they'll surface as build errors handled in Task 1.4.

---

### Task 1.4: Prune the CLI verbs

**Files:**
- Modify: `crates/quelch/src/cli.rs`
- Modify: `crates/quelch/src/main.rs` (or wherever the dispatch lives)
- Modify: `crates/quelch/src/commands/mod.rs`
- Delete: `crates/quelch/src/commands/mcp_key.rs`

The spec's "Verbs gone" list:

- `effective-config` → folded into `instance config` (added in Phase 6)
- `generate-deployment` → folded into `instance config` (added in Phase 6)
- `azure deploy` → renamed `azure apply` (added in Phase 5)
- `azure pull / logs / destroy` → gone outright
- `mcp-key set / rotate / show / generate` → all gone (no replacement; users do `openssl rand -base64 32` themselves)

For `azure plan`, leave the variant in place but make its handler return `bail!("rewrite in progress — replaced by Phase 5")` — Phase 5 wires it up.

- [ ] **Step 1: Edit `cli.rs`**

Delete these `Commands::` variants and their argument structs:
- `EffectiveConfig`
- `GenerateDeployment`
- `McpKey` (and the `McpKeyCommand` enum)
- The `Azure { ... }` subcommand: keep only the `Plan` and `Indexer` variants. Delete `Deploy`, `Pull`, `Logs`, `Destroy`.

Also drop the `AzureCommand` enum's `Pull`, `Logs`, `Destroy` variants. Keep `Plan` (rename from `Deploy`'s sibling) and `Indexer`.

- [ ] **Step 2: Update the dispatch in `main.rs`**

Remove match arms for deleted commands. For `Azure { command: AzureCommand::Plan { .. } }`, leave a stub:

```rust
AzureCommand::Plan { .. } => {
    anyhow::bail!("`quelch azure plan` is being rewritten in the no-deploy pivot — see docs/superpowers/plans/2026-05-06-quelch-no-deploy-pivot.md")
}
```

The `Indexer` arm continues to work via the existing `azure/deploy/indexer.rs`… which we just deleted. Move `indexer.rs` to `crates/quelch/src/azure/indexer.rs` (top-level under `azure/`) and update the `pub mod indexer;` line in `azure/mod.rs` accordingly.

```bash
git mv crates/quelch/src/azure/deploy/indexer.rs crates/quelch/src/azure/indexer.rs
```

If `indexer.rs` was already deleted as part of Task 1.1, restore it:

```bash
git checkout HEAD -- crates/quelch/src/azure/deploy/indexer.rs
git mv crates/quelch/src/azure/deploy/indexer.rs crates/quelch/src/azure/indexer.rs
```

- [ ] **Step 3: Delete `commands/mcp_key.rs` and any usages**

```bash
git rm crates/quelch/src/commands/mcp_key.rs
```

Remove `pub mod mcp_key;` from `crates/quelch/src/commands/mod.rs`.

- [ ] **Step 4: Build**

```bash
cargo build --workspace 2>&1 | tail -50
```

Fix any remaining unresolved imports or references with the minimum patch needed (e.g. `unimplemented!()` stubs in handlers that depend on Phase 2+ work).

---

### Task 1.5: Drop deployment-target plumbing in `commands/`

**Files:**
- Modify: `crates/quelch/src/commands/{status,reset,query,search,get}.rs`

These commands today take a `--deployment <name>` argument. After the pivot, deployment is gone — but the same handlers still need to know which **instance** they're operating on (e.g. `quelch reset --instance jira-internal`). That rewiring happens in Phase 7. For now:

- [ ] **Step 1: Replace `deployment` argument with `instance` everywhere it appears in command structs**

```bash
grep -rn "deployment" crates/quelch/src/commands/
```

For each handler, do a textual rename `deployment` → `instance` in field names and CLI flags. Logic stays the same for now (`Phase 7` makes it correct against the new schema).

- [ ] **Step 2: Build**

```bash
cargo build --workspace
```

---

### Task 1.6: Verify Phase 1 builds green

- [ ] **Step 1: Run the pre-push gate**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

Several tests will fail because they reference deleted/renamed types. For each failing test:
- If it tests a deleted feature (Bicep generation, on-disk rigg, on-prem artefacts, mcp-key flows): **delete the test**.
- Otherwise: temporarily mark `#[ignore]` with a comment `// re-enable in Phase X` (e.g. Phase 7 for ingest tests). Track these in your head — they get re-enabled or rewritten as you proceed.

After this scrub, all three commands must pass.

- [ ] **Step 2: Commit**

```bash
git add -A
git commit -m "refactor: remove auto-deploy paths (Bicep, on-disk rigg, onprem) — Phase 1

Demolition phase of the no-deploy pivot. Quelch no longer:
  - generates Bicep
  - writes rigg files to disk
  - emits per-target deployment artefacts (Docker / systemd / k8s)
  - manages Q-MCP API keys via Key Vault

CLI verbs gone: effective-config, generate-deployment, azure
deploy/pull/logs/destroy, mcp-key set/rotate/show/generate.

Spec: docs/superpowers/specs/2026-05-06-quelch-no-deploy-pivot-design.md
Plan: docs/superpowers/plans/2026-05-06-quelch-no-deploy-pivot.md"
```

---

## Phase 2 — New schema

We replace `deployments[]` with `instances[]` and `source_connections[]`, simplify the `azure:` block per the spec, and add static conflict validation.

### Task 2.1: Test the new schema parses

**Files:**
- Test: `crates/quelch/src/config/schema.rs` (inline `#[cfg(test)]` module)

- [ ] **Step 1: Write the failing test**

Add to the existing `#[cfg(test)] mod tests` block in `schema.rs` (or create one):

```rust
#[test]
fn parses_master_yaml_with_instances_and_connections() {
    let yaml = r#"
azure:
  cosmos:
    subscription_id: "00000000-0000-0000-0000-000000000000"
    resource_group: rg-quelch
    account: acct
    endpoint: https://acct.documents.azure.com
    database: quelch
    containers:
      jira_issues: jira-issues
      jira_sprints: jira-sprints
      jira_fix_versions: jira-fix-versions
      jira_projects: jira-projects
      confluence_pages: confluence-pages
      confluence_spaces: confluence-spaces
    meta_container: quelch-meta
  search:
    endpoint: https://srv.search.windows.net
  ai:
    provider: foundry
    endpoint: https://ai.example
    embedding: { deployment: text-embedding-3-large, dimensions: 3072 }
    chat: { deployment: gpt-5-mini, model_name: gpt-5-mini }

source_connections:
  - name: jira-x
    type: jira
    base_url: https://jira.internal/
    auth: { kind: pat, token: "T1" }
    projects: [DO]
  - name: jira-y
    type: jira
    base_url: https://jira.internal/
    auth: { kind: pat, token: "T2" }
    projects: [EMMA]

instances:
  - name: ingest-internal
    kind: ingest
    connections: [jira-x, jira-y]
    cycle_interval: 5m
  - name: mcp-prod
    kind: mcp
    expose: [jira_issues]
    api_key: "K"
    knowledge_base: kb
    listen: 0.0.0.0:8080
"#;
    let cfg: super::Config = serde_yaml::from_str(yaml).expect("parses");
    assert_eq!(cfg.source_connections.len(), 2);
    assert_eq!(cfg.instances.len(), 2);
    let ingest = cfg
        .instances
        .iter()
        .find(|i| i.name == "ingest-internal")
        .unwrap();
    assert!(matches!(ingest.kind, super::InstanceKind::Ingest));
    let connections = match &ingest.spec {
        super::InstanceSpec::Ingest(i) => &i.connections,
        _ => panic!("wrong variant"),
    };
    assert_eq!(connections, &vec!["jira-x".to_string(), "jira-y".to_string()]);
}
```

- [ ] **Step 2: Run it — expect failure**

```bash
cargo test -p quelch config::schema::tests::parses_master_yaml_with_instances_and_connections 2>&1 | tail -30
```

Expected: compile or runtime failure because the new types don't exist yet.

- [ ] **Step 3: Implement the new schema in `crates/quelch/src/config/schema.rs`**

Replace the entire `Config` struct and its sub-structs. Keep this exact shape:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub azure: AzureConfig,
    pub source_connections: Vec<SourceConnection>,
    pub instances: Vec<InstanceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AzureConfig {
    pub cosmos: CosmosConfig,
    #[serde(default)]
    pub search: Option<SearchConfig>,
    #[serde(default)]
    pub ai: Option<AiConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CosmosConfig {
    // Control-plane fields (None when this is a per-instance slice).
    #[serde(default)]
    pub subscription_id: Option<String>,
    #[serde(default)]
    pub resource_group: Option<String>,
    #[serde(default)]
    pub account: Option<String>,
    // Data-plane fields (always present).
    pub endpoint: String,
    pub database: String,
    #[serde(default)]
    pub containers: ContainerLayout,
    #[serde(default = "default_meta_container")]
    pub meta_container: String,
}

fn default_meta_container() -> String {
    "quelch-meta".to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ContainerLayout {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jira_issues: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jira_sprints: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jira_fix_versions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jira_projects: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confluence_pages: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confluence_spaces: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SearchConfig {
    pub endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiConfig {
    pub provider: AiProvider,
    pub endpoint: String,
    pub embedding: AiEmbedding,
    pub chat: AiChat,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiProvider {
    Foundry,
    AzureOpenai,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiEmbedding {
    pub deployment: String,
    pub dimensions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiChat {
    pub deployment: String,
    pub model_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SourceConnection {
    pub name: String,
    #[serde(rename = "type")]
    pub source_type: SourceType,
    pub base_url: String,
    pub auth: SourceAuth,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spaces: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    Jira,
    Confluence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceAuth {
    Pat { token: String },
    Basic { email: String, token: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstanceConfig {
    pub name: String,
    #[serde(flatten)]
    pub spec: InstanceSpec,
}

impl InstanceConfig {
    pub fn kind(&self) -> InstanceKind {
        match self.spec {
            InstanceSpec::Ingest(_) => InstanceKind::Ingest,
            InstanceSpec::Mcp(_) => InstanceKind::Mcp,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstanceKind {
    Ingest,
    Mcp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstanceSpec {
    Ingest(IngestInstance),
    Mcp(McpInstance),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct IngestInstance {
    pub connections: Vec<String>,
    #[serde(with = "humantime_serde")]
    pub cycle_interval: std::time::Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct McpInstance {
    pub expose: Vec<String>,
    pub api_key: String,
    pub knowledge_base: String,
    pub listen: String,
}
```

Add `humantime_serde` to `crates/quelch/Cargo.toml` if it isn't already there:

```bash
cargo add -p quelch humantime-serde
```

- [ ] **Step 4: Run the test — expect pass**

```bash
cargo test -p quelch config::schema::tests::parses_master_yaml_with_instances_and_connections
```

- [ ] **Step 5: Run the full clippy + test gate**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

Many existing tests will be broken because they construct old types. For each broken test:
- If the test exercises something that survives the pivot (e.g. ingest cycle logic, MCP tool routing): port the test fixtures to the new schema.
- If it exercises a deleted feature: delete the test.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor: replace deployments[] with instances[] + source_connections[]

Single YAML schema serves both as master config and as per-instance
slice. azure: block simplified — control-plane fields on Cosmos are
optional, search/ai blocks are optional (omitted in slimmed slices)."
```

---

### Task 2.2: Slicing logic — emit per-instance config

**Files:**
- Replace: `crates/quelch/src/config/slice.rs`
- Test: `crates/quelch/src/config/slice.rs` (inline tests)

The current `slice.rs` slices by deployment name. Rewrite it to slice by instance name, applying the spec's slimming rules.

- [ ] **Step 1: Write the failing tests**

Replace the existing `slice.rs` test module wholesale with these:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Config {
        serde_yaml::from_str(include_str!("slice_test_fixture.yaml"))
            .expect("fixture parses")
    }

    #[test]
    fn slices_ingest_instance_strips_search_and_ai() {
        let cfg = fixture();
        let slice = slice_for_instance(&cfg, "ingest-internal").expect("slice");
        assert!(slice.azure.search.is_none(), "search must be stripped");
        assert!(slice.azure.ai.is_none(), "ai must be stripped");
        assert!(slice.azure.cosmos.subscription_id.is_none());
        assert!(slice.azure.cosmos.resource_group.is_none());
        assert!(slice.azure.cosmos.account.is_none());
        assert_eq!(slice.instances.len(), 1);
        assert_eq!(slice.instances[0].name, "ingest-internal");
    }

    #[test]
    fn slices_ingest_instance_keeps_only_referenced_connections() {
        let cfg = fixture();
        let slice = slice_for_instance(&cfg, "ingest-internal").expect("slice");
        let names: Vec<_> = slice.source_connections.iter().map(|c| &c.name).collect();
        assert!(names.contains(&&"jira-x".to_string()));
        assert!(names.contains(&&"jira-y".to_string()));
        assert!(!names.iter().any(|n| n.as_str() == "confluence-internal"));
    }

    #[test]
    fn slices_mcp_instance_strips_source_connections_and_ai() {
        let cfg = fixture();
        let slice = slice_for_instance(&cfg, "mcp-prod").expect("slice");
        assert!(slice.source_connections.is_empty());
        assert!(slice.azure.ai.is_none());
        assert!(slice.azure.search.is_some(), "search kept for MCP");
    }

    #[test]
    fn unknown_instance_returns_error() {
        let cfg = fixture();
        let err = slice_for_instance(&cfg, "ghost").unwrap_err();
        assert!(err.to_string().contains("ghost"));
    }
}
```

Create `crates/quelch/src/config/slice_test_fixture.yaml` with the same fixture from Task 2.1's test plus a `confluence-internal` connection that ingest-internal does NOT reference.

- [ ] **Step 2: Run the tests — expect failure**

```bash
cargo test -p quelch config::slice
```

- [ ] **Step 3: Implement `slice_for_instance`**

Replace the body of `crates/quelch/src/config/slice.rs`:

```rust
use crate::config::schema::{Config, InstanceKind, InstanceSpec};

pub fn slice_for_instance(cfg: &Config, instance_name: &str) -> anyhow::Result<Config> {
    let instance = cfg
        .instances
        .iter()
        .find(|i| i.name == instance_name)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "instance '{}' not found in config (have: {})",
                instance_name,
                cfg.instances.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(", ")
            )
        })?
        .clone();

    let mut sliced = cfg.clone();

    // Strip control-plane Cosmos fields — runtime is data-plane only.
    sliced.azure.cosmos.subscription_id = None;
    sliced.azure.cosmos.resource_group = None;
    sliced.azure.cosmos.account = None;
    // ai is wired into the KB at apply-time, never needed at runtime.
    sliced.azure.ai = None;

    match instance.kind() {
        InstanceKind::Ingest => {
            // Q-Ingest doesn't read from Search.
            sliced.azure.search = None;

            // Keep only connections this instance uses.
            let connections = match &instance.spec {
                InstanceSpec::Ingest(i) => i.connections.clone(),
                _ => unreachable!(),
            };
            sliced.source_connections.retain(|c| connections.contains(&c.name));
        }
        InstanceKind::Mcp => {
            // Q-MCP doesn't pull from sources.
            sliced.source_connections.clear();
        }
    }

    sliced.instances = vec![instance];
    Ok(sliced)
}
```

- [ ] **Step 4: Run the tests — expect pass**

```bash
cargo test -p quelch config::slice
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(config): slice_for_instance applies per-kind slimming rules"
```

---

### Task 2.3: Static conflict validation

**Files:**
- Modify: `crates/quelch/src/config/validate.rs`
- Test: same file, inline.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn rejects_overlapping_subsource_claims_across_ingest_instances() {
    let yaml = r#"
azure:
  cosmos:
    endpoint: https://x
    database: quelch
source_connections:
  - { name: jira-a, type: jira, base_url: https://jira.example/,
      auth: { kind: pat, token: T },
      projects: [DO, ANNA] }
  - { name: jira-b, type: jira, base_url: https://jira.example/,
      auth: { kind: pat, token: T2 },
      projects: [ANNA, SARA] }     # conflicts with jira-a on ANNA
instances:
  - { name: ingest-1, kind: ingest, connections: [jira-a], cycle_interval: 5m }
  - { name: ingest-2, kind: ingest, connections: [jira-b], cycle_interval: 5m }
"#;
    let cfg: Config = serde_yaml::from_str(yaml).unwrap();
    let errs = validate(&cfg).unwrap_err();
    let msg = errs.to_string();
    assert!(msg.contains("ingest-1"), "names ingest-1: {}", msg);
    assert!(msg.contains("ingest-2"), "names ingest-2: {}", msg);
    assert!(msg.contains("ANNA"),     "names the conflicting subsource ANNA: {}", msg);
    assert!(msg.contains("https://jira.example/"));
}

#[test]
fn accepts_disjoint_ingest_instances() {
    let yaml = r#"
azure:
  cosmos:
    endpoint: https://x
    database: quelch
source_connections:
  - { name: jira-a, type: jira, base_url: https://jira.example/,
      auth: { kind: pat, token: T }, projects: [DO] }
  - { name: jira-b, type: jira, base_url: https://jira.example/,
      auth: { kind: pat, token: T2 }, projects: [SARA] }
instances:
  - { name: a, kind: ingest, connections: [jira-a], cycle_interval: 5m }
  - { name: b, kind: ingest, connections: [jira-b], cycle_interval: 5m }
"#;
    let cfg: Config = serde_yaml::from_str(yaml).unwrap();
    validate(&cfg).expect("disjoint claims must validate");
}
```

- [ ] **Step 2: Run the tests — expect failure**

```bash
cargo test -p quelch config::validate
```

- [ ] **Step 3: Implement the conflict check**

Add to `validate.rs`:

```rust
use std::collections::BTreeMap;

use crate::config::schema::{Config, InstanceSpec, SourceConnection, SourceType};

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("conflicting subsource claims:\n{0}")]
    Conflicts(String),
    #[error("instance '{instance}' references unknown connection '{connection}'")]
    UnknownConnection { instance: String, connection: String },
    // (existing variants kept)
}

pub fn validate(cfg: &Config) -> Result<(), ValidationError> {
    validate_connection_refs(cfg)?;
    validate_no_overlapping_claims(cfg)?;
    Ok(())
}

fn validate_connection_refs(cfg: &Config) -> Result<(), ValidationError> {
    let names: std::collections::HashSet<_> =
        cfg.source_connections.iter().map(|c| &c.name).collect();
    for inst in &cfg.instances {
        if let InstanceSpec::Ingest(spec) = &inst.spec {
            for c in &spec.connections {
                if !names.contains(c) {
                    return Err(ValidationError::UnknownConnection {
                        instance: inst.name.clone(),
                        connection: c.clone(),
                    });
                }
            }
        }
    }
    Ok(())
}

/// One element per claimed subsource by a given ingest instance.
type ClaimKey = (SourceType, String, String); // (type, base_url, subsource)

fn claims_for_connection(c: &SourceConnection) -> impl Iterator<Item = ClaimKey> + '_ {
    let subsources: Vec<&String> = match c.source_type {
        SourceType::Jira => c.projects.iter().collect(),
        SourceType::Confluence => c.spaces.iter().collect(),
    };
    subsources
        .into_iter()
        .map(move |s| (c.source_type, c.base_url.clone(), s.clone()))
}

fn validate_no_overlapping_claims(cfg: &Config) -> Result<(), ValidationError> {
    let conn_by_name: BTreeMap<&str, &SourceConnection> =
        cfg.source_connections.iter().map(|c| (c.name.as_str(), c)).collect();

    let mut claimed: BTreeMap<ClaimKey, Vec<&str>> = BTreeMap::new();

    for inst in &cfg.instances {
        let InstanceSpec::Ingest(spec) = &inst.spec else { continue };
        for conn_name in &spec.connections {
            let Some(conn) = conn_by_name.get(conn_name.as_str()) else { continue };
            for key in claims_for_connection(conn) {
                claimed.entry(key).or_default().push(inst.name.as_str());
            }
        }
    }

    let mut conflicts = String::new();
    for ((kind, base, sub), claimers) in &claimed {
        if claimers.len() > 1 {
            use std::fmt::Write;
            let _ = writeln!(
                conflicts,
                "  - ({:?}, {}, {}) claimed by {}",
                kind,
                base,
                sub,
                claimers.join(", ")
            );
        }
    }
    if conflicts.is_empty() {
        Ok(())
    } else {
        Err(ValidationError::Conflicts(conflicts))
    }
}
```

- [ ] **Step 4: Run the tests — expect pass**

```bash
cargo test -p quelch config::validate
```

- [ ] **Step 5: Wire `validate` into the existing CLI `validate` command**

```bash
grep -rn "fn validate" crates/quelch/src/commands/
```

Update the handler to call `crate::config::validate::validate(&cfg)?;` and exit non-zero on error.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(config): static conflict check rejects overlapping subsource claims"
```

---

## Phase 3 — Cosmos control-plane via ARM REST

`quelch azure apply` needs to create/update Cosmos databases and containers. We do this via ARM REST against `management.azure.com`, authenticated by `DefaultAzureCredential`. No Bicep, no SDK detours.

### Task 3.1: ARM REST client

**Files:**
- Create: `crates/quelch/src/azure/cosmos_config/mod.rs`
- Create: `crates/quelch/src/azure/cosmos_config/arm_client.rs`
- Create: `crates/quelch/src/azure/cosmos_config/diff.rs`
- Modify: `crates/quelch/src/azure/mod.rs` (add `pub mod cosmos_config;`)

- [ ] **Step 1: Add azure-identity dep**

```bash
cargo add -p quelch azure_identity azure_core
```

- [ ] **Step 2: Write the diff-logic test (no network needed)**

Create `crates/quelch/src/azure/cosmos_config/diff.rs`:

```rust
use crate::config::schema::CosmosConfig;

#[derive(Debug, Clone, PartialEq)]
pub struct ContainerSpec {
    pub name: String,
    pub partition_key: String,
}

/// Computes the desired container set from the master config.
/// Includes only containers actually referenced (defaults + meta).
pub fn desired_containers(cosmos: &CosmosConfig) -> Vec<ContainerSpec> {
    let layout = &cosmos.containers;
    let mut out = vec![];
    for (name_opt, partition_key) in [
        (layout.jira_issues.as_deref(),       "/id"),
        (layout.jira_sprints.as_deref(),      "/id"),
        (layout.jira_fix_versions.as_deref(), "/id"),
        (layout.jira_projects.as_deref(),     "/id"),
        (layout.confluence_pages.as_deref(),  "/id"),
        (layout.confluence_spaces.as_deref(), "/id"),
    ] {
        if let Some(n) = name_opt {
            out.push(ContainerSpec {
                name: n.to_string(),
                partition_key: partition_key.to_string(),
            });
        }
    }
    // Meta container partitioned by source_name (see Phase 7).
    out.push(ContainerSpec {
        name: cosmos.meta_container.clone(),
        partition_key: "/source_name".to_string(),
    });
    out
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContainerDiff {
    Create(ContainerSpec),
    Match { name: String },
    PartitionKeyMismatch { name: String, want: String, have: String },
}

pub fn diff_containers(want: &[ContainerSpec], have: &[ContainerSpec]) -> Vec<ContainerDiff> {
    let mut out = vec![];
    for w in want {
        match have.iter().find(|h| h.name == w.name) {
            Some(h) if h.partition_key == w.partition_key => {
                out.push(ContainerDiff::Match { name: w.name.clone() });
            }
            Some(h) => out.push(ContainerDiff::PartitionKeyMismatch {
                name: w.name.clone(),
                want: w.partition_key.clone(),
                have: h.partition_key.clone(),
            }),
            None => out.push(ContainerDiff::Create(w.clone())),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_creates_missing_and_matches_existing() {
        let want = vec![
            ContainerSpec { name: "a".into(), partition_key: "/id".into() },
            ContainerSpec { name: "b".into(), partition_key: "/id".into() },
        ];
        let have = vec![
            ContainerSpec { name: "a".into(), partition_key: "/id".into() },
        ];
        assert_eq!(
            diff_containers(&want, &have),
            vec![
                ContainerDiff::Match { name: "a".into() },
                ContainerDiff::Create(ContainerSpec { name: "b".into(), partition_key: "/id".into() }),
            ]
        );
    }

    #[test]
    fn diff_flags_partition_key_mismatch() {
        let want = vec![ContainerSpec { name: "a".into(), partition_key: "/id".into() }];
        let have = vec![ContainerSpec { name: "a".into(), partition_key: "/wrong".into() }];
        assert_eq!(
            diff_containers(&want, &have),
            vec![ContainerDiff::PartitionKeyMismatch {
                name: "a".into(),
                want: "/id".into(),
                have: "/wrong".into(),
            }]
        );
    }
}
```

- [ ] **Step 3: Run the tests — expect pass (or implement until they pass)**

```bash
cargo test -p quelch azure::cosmos_config::diff
```

- [ ] **Step 4: Implement the ARM client**

Create `crates/quelch/src/azure/cosmos_config/arm_client.rs`:

```rust
//! Minimal ARM REST client for Cosmos DB control-plane operations.

use anyhow::{Context, Result};
use azure_core::auth::TokenCredential;
use serde::{Deserialize, Serialize};

use super::diff::ContainerSpec;

const ARM: &str = "https://management.azure.com";
const COSMOS_API_VERSION: &str = "2024-05-15";
const ARM_SCOPE: &str = "https://management.azure.com/.default";

pub struct ArmCosmosClient {
    pub credential: std::sync::Arc<dyn TokenCredential>,
    pub http: reqwest::Client,
    pub subscription_id: String,
    pub resource_group: String,
    pub account: String,
}

impl ArmCosmosClient {
    async fn token(&self) -> Result<String> {
        let token = self.credential.get_token(&[ARM_SCOPE]).await?;
        Ok(token.token.secret().to_string())
    }

    pub async fn ensure_database(&self, db: &str) -> Result<()> {
        let url = format!(
            "{ARM}/subscriptions/{}/resourceGroups/{}/providers/Microsoft.DocumentDB/databaseAccounts/{}/sqlDatabases/{}?api-version={COSMOS_API_VERSION}",
            self.subscription_id, self.resource_group, self.account, db
        );
        let body = serde_json::json!({
            "properties": { "resource": { "id": db } }
        });
        let token = self.token().await?;
        let resp = self.http.put(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("PUT database {db}: {} — {}", resp.status(), resp.text().await.unwrap_or_default());
        }
        Ok(())
    }

    pub async fn list_containers(&self, db: &str) -> Result<Vec<ContainerSpec>> {
        let url = format!(
            "{ARM}/subscriptions/{}/resourceGroups/{}/providers/Microsoft.DocumentDB/databaseAccounts/{}/sqlDatabases/{}/containers?api-version={COSMOS_API_VERSION}",
            self.subscription_id, self.resource_group, self.account, db
        );
        let token = self.token().await?;
        let resp = self.http.get(&url).bearer_auth(&token).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("LIST containers: {} — {}", resp.status(), resp.text().await.unwrap_or_default());
        }
        #[derive(Deserialize)]
        struct ListResp { value: Vec<ContainerArmRecord> }
        #[derive(Deserialize)]
        struct ContainerArmRecord {
            name: String,
            properties: ContainerArmProps,
        }
        #[derive(Deserialize)]
        struct ContainerArmProps {
            resource: ContainerArmResource,
        }
        #[derive(Deserialize)]
        struct ContainerArmResource {
            #[serde(rename = "partitionKey")]
            partition_key: PartitionKey,
        }
        #[derive(Deserialize)]
        struct PartitionKey { paths: Vec<String> }
        let parsed: ListResp = resp.json().await.context("parse container list")?;
        Ok(parsed.value.into_iter().map(|r| ContainerSpec {
            name: r.name,
            partition_key: r.properties.resource.partition_key.paths.into_iter().next()
                .unwrap_or_else(|| "/id".to_string()),
        }).collect())
    }

    pub async fn create_container(&self, db: &str, spec: &ContainerSpec) -> Result<()> {
        let url = format!(
            "{ARM}/subscriptions/{}/resourceGroups/{}/providers/Microsoft.DocumentDB/databaseAccounts/{}/sqlDatabases/{}/containers/{}?api-version={COSMOS_API_VERSION}",
            self.subscription_id, self.resource_group, self.account, db, spec.name
        );
        let body = serde_json::json!({
            "properties": {
                "resource": {
                    "id": spec.name,
                    "partitionKey": { "paths": [spec.partition_key], "kind": "Hash" }
                }
            }
        });
        let token = self.token().await?;
        let resp = self.http.put(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("PUT container {}: {} — {}", spec.name, resp.status(), resp.text().await.unwrap_or_default());
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
struct _SerdeAnchor;     // keeps the imports honest if we don't end up using Serialize
```

(Drop the `_SerdeAnchor` once you wire actual Serialize usages, or remove the import.)

- [ ] **Step 5: Implement the orchestrator in `cosmos_config/mod.rs`**

```rust
pub mod arm_client;
pub mod diff;

use anyhow::{Context, Result};

use crate::config::schema::CosmosConfig;

pub use arm_client::ArmCosmosClient;
pub use diff::{desired_containers, diff_containers, ContainerDiff, ContainerSpec};

pub struct CosmosPlan {
    pub diffs: Vec<ContainerDiff>,
}

pub async fn plan(client: &ArmCosmosClient, cosmos: &CosmosConfig) -> Result<CosmosPlan> {
    let want = desired_containers(cosmos);
    let have = client.list_containers(&cosmos.database).await
        .context("list existing containers")
        .unwrap_or_default(); // empty list when DB doesn't exist yet
    Ok(CosmosPlan { diffs: diff_containers(&want, &have) })
}

pub async fn apply(client: &ArmCosmosClient, cosmos: &CosmosConfig) -> Result<()> {
    client.ensure_database(&cosmos.database).await?;
    let plan = plan(client, cosmos).await?;
    for d in plan.diffs {
        match d {
            ContainerDiff::Match { .. } => continue,
            ContainerDiff::Create(spec) => client.create_container(&cosmos.database, &spec).await?,
            ContainerDiff::PartitionKeyMismatch { name, want, have } => {
                anyhow::bail!(
                    "container '{name}' exists with partition key '{have}', want '{want}'. \
                     Quelch will not auto-recreate (would lose data). Resolve manually then retry."
                );
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 6: Build + commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test --workspace
git add -A
git commit -m "feat(azure): cosmos control-plane via ARM REST (no Bicep)

`quelch azure apply` now PUTs Cosmos databases and containers
directly via management.azure.com using DefaultAzureCredential.
Mismatched partition keys are surfaced as a hard error rather than
silently recreated."
```

---

## Phase 4 — In-memory rigg

Today, `azure/rigg/{generate,plan,push}.rs` reads/writes a `rigg/` directory on disk. Refactor them to operate purely on in-memory rigg structs.

### Task 4.1: Inventory the surviving rigg API surface

- [ ] **Step 1: Read each file end-to-end**

```bash
wc -l crates/quelch/src/azure/rigg/{generate,plan,push,mod}.rs
```

For each, identify:
- Public functions called from outside the module (likely `generate`, `plan`, `push`, plus the high-level orchestrator).
- Functions that read/write the `rigg/` directory.
- Functions that compute pure in-memory state.

- [ ] **Step 2: Sketch the new public surface**

The new `azure::rigg` module exposes:

```rust
pub fn generate(cfg: &Config) -> RiggDesiredState;
pub async fn plan(state: &RiggDesiredState, client: &RiggClient) -> Result<RiggDiff>;
pub async fn apply(state: &RiggDesiredState, client: &RiggClient) -> Result<()>;
```

Where `RiggDesiredState` is whatever struct rigg-core's `apply` accepts in-memory (likely a `Vec<RiggResource>` or similar — read `rigg-core`'s docs / source).

No file paths, no `rigg/` directory.

---

### Task 4.2: Refactor `rigg/generate.rs` to return in-memory state

**Files:**
- Modify: `crates/quelch/src/azure/rigg/generate.rs`

- [ ] **Step 1: Locate every `std::fs::write` / `tokio::fs::write` / `Path::new("rigg/")` reference**

```bash
grep -n "fs::write\|fs::create_dir\|\"rigg" crates/quelch/src/azure/rigg/generate.rs
```

- [ ] **Step 2: Replace each file write with an in-memory push**

Convert the function from:

```rust
pub fn generate_rigg_files(cfg: &Config, root: &Path) -> Result<()> {
    fs::write(root.join("indexes/jira-issues.yaml"), serde_yaml::to_string(&index)?)?;
    // ...
}
```

to:

```rust
pub fn generate(cfg: &Config) -> Result<RiggDesiredState> {
    let mut state = RiggDesiredState::default();
    state.indexes.push(build_index_for_jira_issues(cfg));
    // ...
    Ok(state)
}
```

- [ ] **Step 3: Update the type that callers use**

Define `RiggDesiredState` in `azure/rigg/mod.rs`:

```rust
#[derive(Debug, Default)]
pub struct RiggDesiredState {
    pub indexes: Vec<rigg_core::Index>,
    pub indexers: Vec<rigg_core::Indexer>,
    pub skillsets: Vec<rigg_core::Skillset>,
    pub data_sources: Vec<rigg_core::DataSource>,
    pub knowledge_sources: Vec<rigg_core::KnowledgeSource>,
    pub knowledge_bases: Vec<rigg_core::KnowledgeBase>,
}
```

(Match the actual rigg-core type names — verify with `cargo doc -p rigg-core --open` or by reading the rigg-core source.)

- [ ] **Step 4: Tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::*;

    #[test]
    fn generates_resources_for_each_exposed_data_source() {
        let yaml = r#"
azure:
  cosmos:
    endpoint: https://x
    database: quelch
    containers:
      jira_issues: jira-issues
      confluence_pages: confluence-pages
    meta_container: quelch-meta
  search:
    endpoint: https://srv.search.windows.net
  ai:
    provider: foundry
    endpoint: https://ai
    embedding: { deployment: e, dimensions: 3072 }
    chat: { deployment: c, model_name: c }
source_connections: []
instances:
  - { name: mcp-prod, kind: mcp,
      expose: [jira_issues, confluence_pages],
      api_key: K, knowledge_base: kb, listen: 0.0.0.0:8080 }
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        let state = generate(&cfg).unwrap();
        assert_eq!(state.indexes.len(), 2,           "one index per exposed data source");
        assert_eq!(state.indexers.len(), 2,          "one indexer per index");
        assert_eq!(state.skillsets.len(), 2,         "one skillset per index");
        assert_eq!(state.data_sources.len(), 2,     "one cosmos data source per index");
        assert_eq!(state.knowledge_sources.len(), 2, "one KS per index");
        assert_eq!(state.knowledge_bases.len(), 1,  "one KB per MCP instance");
    }

    #[test]
    fn generates_nothing_when_no_mcp_instance() {
        let yaml = r#"
azure: { cosmos: { endpoint: https://x, database: quelch, meta_container: quelch-meta } }
source_connections: []
instances: []
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        let state = generate(&cfg).unwrap();
        assert!(state.indexes.is_empty());
        assert!(state.knowledge_bases.is_empty());
    }
}
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor(rigg): generate() returns in-memory desired state"
```

---

### Task 4.3: Refactor `rigg/plan.rs` and `rigg/push.rs` to operate on in-memory state

**Files:**
- Modify: `crates/quelch/src/azure/rigg/plan.rs`
- Modify: `crates/quelch/src/azure/rigg/push.rs`

- [ ] **Step 1: Rewrite `plan` to take `&RiggDesiredState` and a `RiggClient`**

```rust
pub async fn plan(
    state: &RiggDesiredState,
    client: &rigg_core::Client,
) -> Result<RiggDiff> {
    // For each resource type, fetch the live state from `client`,
    // diff against `state`, accumulate into `RiggDiff`.
    // Return diff.
}
```

`RiggDiff` is whatever shape is convenient for rendering — a `Vec<RiggChange>` enum (`Create`, `Update`, `Delete`, `Match`) per resource is straightforward.

- [ ] **Step 2: Rewrite `push` (rename to `apply`) similarly**

```rust
pub async fn apply(
    state: &RiggDesiredState,
    client: &rigg_core::Client,
) -> Result<()> {
    // For each resource type: create-or-update against the live API.
}
```

- [ ] **Step 3: Update `rigg/mod.rs` re-exports**

Drop any references to file-IO helpers; re-export only `generate`, `plan`, `apply`, and the public types.

- [ ] **Step 4: Build + test + commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test --workspace
git add -A
git commit -m "refactor(rigg): plan/apply work on in-memory state, no on-disk files"
```

---

## Phase 5 — `quelch azure plan` and `quelch azure apply`

### Task 5.1: Top-level orchestrators

**Files:**
- Create: `crates/quelch/src/azure/plan.rs`
- Create: `crates/quelch/src/azure/apply.rs`
- Modify: `crates/quelch/src/azure/mod.rs`

- [ ] **Step 1: Implement `azure/plan.rs`**

```rust
//! `quelch azure plan` — compute and render the diff for cosmos + AI Search.

use anyhow::Result;

use crate::azure::{cosmos_config, rigg};
use crate::config::schema::Config;

pub struct AzurePlan {
    pub cosmos: cosmos_config::CosmosPlan,
    pub rigg: rigg::RiggDiff,
}

pub async fn compute(
    cfg: &Config,
    cosmos_client: &cosmos_config::ArmCosmosClient,
    rigg_client: &rigg_core::Client,
) -> Result<AzurePlan> {
    let cosmos_plan = cosmos_config::plan(cosmos_client, &cfg.azure.cosmos).await?;
    let desired = rigg::generate(cfg)?;
    let rigg_diff = rigg::plan(&desired, rigg_client).await?;
    Ok(AzurePlan { cosmos: cosmos_plan, rigg: rigg_diff })
}

pub fn render(plan: &AzurePlan) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    writeln!(&mut out, "Cosmos DB:").unwrap();
    for d in &plan.cosmos.diffs {
        match d {
            cosmos_config::ContainerDiff::Match { name } =>
                writeln!(&mut out, "  = {name}  (no change)").unwrap(),
            cosmos_config::ContainerDiff::Create(spec) =>
                writeln!(&mut out, "  + {} (partition key {})", spec.name, spec.partition_key).unwrap(),
            cosmos_config::ContainerDiff::PartitionKeyMismatch { name, want, have } =>
                writeln!(&mut out, "  ! {name}  (have pk={have}, want pk={want}) — manual fix required").unwrap(),
        }
    }
    writeln!(&mut out, "\nAI Search:").unwrap();
    write!(&mut out, "{}", plan.rigg.render()).unwrap();
    out
}
```

(`RiggDiff::render` returns a string formatted similarly.)

- [ ] **Step 2: Implement `azure/apply.rs`**

```rust
use anyhow::Result;

use crate::azure::{cosmos_config, rigg, plan as plan_mod};
use crate::config::schema::Config;

pub async fn apply(
    cfg: &Config,
    cosmos_client: &cosmos_config::ArmCosmosClient,
    rigg_client: &rigg_core::Client,
) -> Result<()> {
    cosmos_config::apply(cosmos_client, &cfg.azure.cosmos).await?;
    let desired = rigg::generate(cfg)?;
    rigg::apply(&desired, rigg_client).await?;
    Ok(())
}
```

- [ ] **Step 3: Update `azure/mod.rs`**

```rust
pub mod apply;
pub mod cosmos_config;
pub mod indexer;
pub mod plan;
pub mod rigg;
```

- [ ] **Step 4: Wire into the CLI**

In `cli.rs`, ensure `Commands::Azure` has only:

```rust
Azure {
    #[command(subcommand)]
    command: AzureCommand,
},

#[derive(Subcommand, Debug)]
pub enum AzureCommand {
    Plan,
    Apply,
    Indexer { ... },
}
```

In the dispatcher (`main.rs` or equivalent), wire each arm:

```rust
AzureCommand::Plan => {
    let cfg = load_config(&cli.config)?;
    let cosmos_client = build_cosmos_client(&cfg).await?;
    let rigg_client = build_rigg_client(&cfg).await?;
    let plan = azure::plan::compute(&cfg, &cosmos_client, &rigg_client).await?;
    print!("{}", azure::plan::render(&plan));
    Ok(())
}
AzureCommand::Apply => {
    let cfg = load_config(&cli.config)?;
    let cosmos_client = build_cosmos_client(&cfg).await?;
    let rigg_client = build_rigg_client(&cfg).await?;
    let plan = azure::plan::compute(&cfg, &cosmos_client, &rigg_client).await?;
    print!("{}", azure::plan::render(&plan));
    let _ = ask_yes_no("Apply these changes? [y/N] ")?;
    azure::apply::apply(&cfg, &cosmos_client, &rigg_client).await?;
    println!("done.");
    Ok(())
}
```

`build_cosmos_client` and `build_rigg_client` take the `Config`'s azure block + `DefaultAzureCredential` and return ready clients. Co-locate these constructors in `azure/mod.rs`.

- [ ] **Step 5: Test the render path with synthetic plans**

Add a unit test in `azure/plan.rs`:

```rust
#[test]
fn renders_human_readable_diff() {
    let plan = AzurePlan {
        cosmos: cosmos_config::CosmosPlan {
            diffs: vec![
                cosmos_config::ContainerDiff::Create(cosmos_config::ContainerSpec {
                    name: "jira-issues".into(),
                    partition_key: "/id".into(),
                }),
                cosmos_config::ContainerDiff::Match { name: "quelch-meta".into() },
            ],
        },
        rigg: rigg::RiggDiff::default(),
    };
    let out = render(&plan);
    assert!(out.contains("+ jira-issues"));
    assert!(out.contains("= quelch-meta"));
}
```

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test --workspace
git add -A
git commit -m "feat(azure): top-level plan/apply orchestrators (cosmos + rigg)"
```

---

## Phase 6 — `quelch instance list` and `quelch instance config`

### Task 6.1: Instance subcommands

**Files:**
- Create: `crates/quelch/src/commands/instance.rs`
- Modify: `crates/quelch/src/cli.rs`
- Modify: `crates/quelch/src/main.rs` dispatcher

- [ ] **Step 1: CLI shape**

In `cli.rs`:

```rust
#[derive(Subcommand, Debug)]
pub enum Commands {
    // ...
    /// Manage named instances declared in quelch.yaml.
    Instance {
        #[command(subcommand)]
        command: InstanceCommand,
    },
    // ...
}

#[derive(Subcommand, Debug)]
pub enum InstanceCommand {
    /// List instances declared in the master config.
    List,

    /// Emit a per-instance config file (slimmed slice).
    Config {
        /// Instance name from quelch.yaml.
        name: String,
        /// Sanity-check that the instance has the expected kind.
        #[arg(long, value_enum)]
        kind: InstanceKindArg,
        /// Write to this path instead of stdout.
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum InstanceKindArg { Ingest, Mcp }
```

- [ ] **Step 2: Implement `commands/instance.rs`**

```rust
use std::path::Path;

use anyhow::{anyhow, Result};

use crate::cli::InstanceKindArg;
use crate::config::schema::{Config, InstanceKind};
use crate::config::slice::slice_for_instance;

pub fn list(cfg: &Config) -> Result<()> {
    if cfg.instances.is_empty() {
        println!("(no instances declared)");
        return Ok(());
    }
    let name_w = cfg.instances.iter().map(|i| i.name.len()).max().unwrap_or(0);
    for inst in &cfg.instances {
        let kind = match inst.kind() {
            InstanceKind::Ingest => "ingest",
            InstanceKind::Mcp => "mcp",
        };
        println!("  {:<name_w$}  {kind}", inst.name);
    }
    Ok(())
}

pub fn config(
    cfg: &Config,
    instance_name: &str,
    declared_kind: InstanceKindArg,
    output: Option<&Path>,
) -> Result<()> {
    let slice = slice_for_instance(cfg, instance_name)?;
    let actual_kind = slice.instances[0].kind();
    let expected = match declared_kind {
        InstanceKindArg::Ingest => InstanceKind::Ingest,
        InstanceKindArg::Mcp => InstanceKind::Mcp,
    };
    if actual_kind != expected {
        return Err(anyhow!(
            "instance '{}' is {:?} but --kind says {:?}",
            instance_name, actual_kind, expected
        ));
    }
    let yaml = serde_yaml::to_string(&slice)?;
    match output {
        Some(p) => std::fs::write(p, yaml)?,
        None => print!("{yaml}"),
    }
    Ok(())
}
```

- [ ] **Step 3: Tests**

```rust
#[test]
fn config_kind_mismatch_errors() {
    let cfg: Config = serde_yaml::from_str(MASTER_FIXTURE).unwrap();
    let err = config(&cfg, "ingest-internal", InstanceKindArg::Mcp, None).unwrap_err();
    assert!(err.to_string().contains("ingest"));
    assert!(err.to_string().contains("Mcp"));
}

#[test]
fn config_ingest_emits_slimmed_yaml() {
    let cfg: Config = serde_yaml::from_str(MASTER_FIXTURE).unwrap();
    let mut buf = vec![];
    {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        config(&cfg, "ingest-internal", InstanceKindArg::Ingest, Some(tmp.path())).unwrap();
        buf = std::fs::read(tmp.path()).unwrap();
    }
    let s = String::from_utf8(buf).unwrap();
    assert!(!s.contains("subscription_id"), "control-plane fields stripped");
    assert!(!s.contains("ai:"),              "ai stripped from ingest");
    assert!(!s.contains("search:"),          "search stripped from ingest");
    assert!( s.contains("source_connections:"));
}
```

- [ ] **Step 4: Wire into the dispatcher**

```rust
Commands::Instance { command } => match command {
    InstanceCommand::List => commands::instance::list(&cfg),
    InstanceCommand::Config { name, kind, output } =>
        commands::instance::config(&cfg, &name, kind, output.as_deref()),
}
```

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test --workspace
git add -A
git commit -m "feat(cli): \`quelch instance list\` and \`quelch instance config\`"
```

---

## Phase 7 — Cursor ownership

### Task 7.1: Add `owner_instance` and rekey

**Files:**
- Modify: `crates/quelch/src/cosmos/meta.rs`
- Modify: every caller of `CursorKey` (find with grep)

- [ ] **Step 1: Add owner_instance to Cursor and rekey CursorKey**

```rust
#[derive(Debug, Clone)]
pub struct CursorKey {
    pub source_name: String,
    pub subsource: String,
}

impl CursorKey {
    pub fn id(&self) -> String {
        format!("{}::{}", self.source_name, self.subsource)
    }
    pub fn partition_key(&self) -> &str {
        &self.source_name
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cursor {
    pub owner_instance: Option<String>,
    pub last_complete_minute: Option<DateTime<Utc>>,
    pub documents_synced_total: u64,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub backfill_in_progress: bool,
    pub backfill_target: Option<DateTime<Utc>>,
    pub backfill_last_seen: Option<BackfillCheckpoint>,
    pub last_reconciliation_at: Option<DateTime<Utc>>,
    pub last_reconciliation_deleted: u64,
}
```

- [ ] **Step 2: Build, fix every caller**

```bash
cargo build --workspace 2>&1 | tail -80
```

For each caller, drop the `deployment_name` parameter and replace any `deployment_name` references with the new instance name from the runtime config.

- [ ] **Step 3: Add a test that round-trips a cursor with owner_instance set**

In `cosmos/meta.rs`:

```rust
#[test]
fn cursor_serializes_with_owner_instance() {
    let c = Cursor {
        owner_instance: Some("ingest-internal".into()),
        documents_synced_total: 42,
        ..Default::default()
    };
    let json = serde_json::to_string(&c).unwrap();
    assert!(json.contains("\"owner_instance\":\"ingest-internal\""));
    let back: Cursor = serde_json::from_str(&json).unwrap();
    assert_eq!(back.owner_instance.as_deref(), Some("ingest-internal"));
}
```

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test --workspace
git add -A
git commit -m "feat(cosmos): cursor docs carry owner_instance, partitioned by source_name"
```

---

### Task 7.2: Q-Ingest claims + refusal

**Files:**
- Modify: `crates/quelch/src/ingest/worker.rs` (or wherever startup lives — `grep -rn "fn run" crates/quelch/src/ingest/`)
- Test: same module.

- [ ] **Step 1: Write the failing test (in-memory cosmos backend)**

```rust
#[tokio::test]
async fn ingest_refuses_when_cursor_owned_by_other_instance() {
    let mut backend = InMemoryCosmosBackend::new();
    // Pre-existing cursor owned by "instance-a".
    backend.put_cursor(
        &CursorKey { source_name: "jira-x".into(), subsource: "DO".into() },
        &Cursor { owner_instance: Some("instance-a".into()), ..Default::default() },
    ).await.unwrap();

    let cfg = ingest_config_for_instance("instance-b", &[("jira-x", &["DO"])]);
    let result = run_one_cycle(&backend, &cfg).await;
    let err = result.unwrap_err().to_string();
    assert!(err.contains("instance-a"));
    assert!(err.contains("instance-b"));
    assert!(err.contains("DO"));
}

#[tokio::test]
async fn ingest_claims_unowned_cursor_on_first_run() {
    let backend = InMemoryCosmosBackend::new();
    let cfg = ingest_config_for_instance("instance-b", &[("jira-x", &["DO"])]);
    run_one_cycle(&backend, &cfg).await.unwrap();
    let stored = backend.get_cursor(&CursorKey {
        source_name: "jira-x".into(), subsource: "DO".into(),
    }).await.unwrap();
    assert_eq!(stored.owner_instance.as_deref(), Some("instance-b"));
}
```

- [ ] **Step 2: Implement claim-or-fail**

In the ingest worker, before any per-cycle work, run:

```rust
async fn ensure_claim<B: CosmosBackend>(
    backend: &B,
    instance_name: &str,
    key: &CursorKey,
) -> Result<()> {
    match backend.get_cursor(key).await? {
        Some(existing) if existing.owner_instance.as_deref() == Some(instance_name) => Ok(()),
        Some(existing) if existing.owner_instance.is_some() => {
            anyhow::bail!(
                "cursor {} is already owned by instance '{}'; this instance is '{}'. \
                 Run `quelch reset --instance {} --source {} --subsource {} --take-ownership` \
                 to transfer.",
                key.id(),
                existing.owner_instance.unwrap(),
                instance_name,
                instance_name,
                key.source_name,
                key.subsource
            )
        }
        _ => {
            // Unowned (None) or doesn't exist — claim it.
            let mut c = backend.get_cursor(key).await?.unwrap_or_default();
            c.owner_instance = Some(instance_name.to_string());
            backend.put_cursor(key, &c).await?;
            Ok(())
        }
    }
}
```

Call `ensure_claim` for every (source_name, subsource) tuple at worker startup. Failure exits with the error message and a non-zero status — no retry.

- [ ] **Step 3: Run tests**

```bash
cargo test -p quelch ingest::worker
```

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat(ingest): refuse cursors claimed by another instance, claim unowned ones"
```

---

### Task 7.3: `quelch reset --take-ownership`

**Files:**
- Modify: `crates/quelch/src/commands/reset.rs`
- Modify: `crates/quelch/src/cli.rs`

- [ ] **Step 1: Add the flag**

In the `Reset` variant in `cli.rs`:

```rust
Reset {
    #[arg(long)]
    instance: String,
    #[arg(long)]
    source: String,
    #[arg(long)]
    subsource: String,
    /// Rewrite the cursor's owner_instance to this instance even if currently held by another.
    #[arg(long)]
    take_ownership: bool,
},
```

- [ ] **Step 2: Implement the behaviour**

```rust
pub async fn reset<B: CosmosBackend>(
    backend: &B,
    instance: &str,
    source: &str,
    subsource: &str,
    take_ownership: bool,
) -> Result<()> {
    let key = CursorKey { source_name: source.into(), subsource: subsource.into() };
    let mut cursor = backend.get_cursor(&key).await?.unwrap_or_default();
    match (&cursor.owner_instance, take_ownership) {
        (Some(owner), false) if owner != instance => {
            anyhow::bail!(
                "cursor owned by '{owner}', refusing to reset from '{instance}'. \
                 Pass --take-ownership to override."
            )
        }
        _ => {
            cursor = Cursor { owner_instance: Some(instance.to_string()), ..Default::default() };
            backend.put_cursor(&key, &cursor).await?;
            println!("reset cursor {}::{} (owner: {})", source, subsource, instance);
            Ok(())
        }
    }
}
```

- [ ] **Step 3: Tests for both paths**

```rust
#[tokio::test]
async fn reset_without_take_ownership_refuses_other_instance_cursor() {
    let backend = InMemoryCosmosBackend::new();
    backend.put_cursor(
        &CursorKey { source_name: "jira-x".into(), subsource: "DO".into() },
        &Cursor { owner_instance: Some("instance-a".into()), ..Default::default() },
    ).await.unwrap();
    let err = reset(&backend, "instance-b", "jira-x", "DO", false).await.unwrap_err();
    assert!(err.to_string().contains("instance-a"));
    assert!(err.to_string().contains("--take-ownership"));
}

#[tokio::test]
async fn reset_with_take_ownership_rewrites_owner() {
    let backend = InMemoryCosmosBackend::new();
    backend.put_cursor(
        &CursorKey { source_name: "jira-x".into(), subsource: "DO".into() },
        &Cursor {
            owner_instance: Some("instance-a".into()),
            documents_synced_total: 999,
            ..Default::default()
        },
    ).await.unwrap();
    reset(&backend, "instance-b", "jira-x", "DO", true).await.unwrap();
    let stored = backend.get_cursor(&CursorKey {
        source_name: "jira-x".into(), subsource: "DO".into(),
    }).await.unwrap().unwrap();
    assert_eq!(stored.owner_instance.as_deref(), Some("instance-b"));
    assert_eq!(stored.documents_synced_total, 0, "reset clears progress");
}
```

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test --workspace
git add -A
git commit -m "feat(cli): \`quelch reset --take-ownership\` for cursor ownership transfer"
```

---

## Phase 8 — `quelch ingest` / `quelch mcp` runtime launch

### Task 8.1: Auto-detect `--instance`

**Files:**
- Modify: `crates/quelch/src/cli.rs` (the `Ingest` and `Mcp` variants)
- Modify: `crates/quelch/src/main.rs` (dispatch helpers)
- Test: a new module `crates/quelch/src/cli_helpers.rs` (or wherever resolution logic lives)

- [ ] **Step 1: Helper to resolve instance name with auto-detect**

```rust
use anyhow::{anyhow, Result};

use crate::config::schema::{Config, InstanceKind};

pub fn resolve_instance<'a>(
    cfg: &'a Config,
    explicit: Option<&str>,
    want_kind: InstanceKind,
) -> Result<&'a str> {
    if let Some(name) = explicit {
        let inst = cfg.instances.iter().find(|i| i.name == name)
            .ok_or_else(|| anyhow!("instance '{name}' not found"))?;
        if inst.kind() != want_kind {
            return Err(anyhow!(
                "instance '{name}' is {:?}, expected {:?}", inst.kind(), want_kind
            ));
        }
        return Ok(&inst.name);
    }
    let candidates: Vec<&str> = cfg.instances.iter()
        .filter(|i| i.kind() == want_kind)
        .map(|i| i.name.as_str())
        .collect();
    match candidates.as_slice() {
        [single] => Ok(single),
        [] => Err(anyhow!("no {:?} instances declared in config", want_kind)),
        many => Err(anyhow!(
            "multiple {:?} instances ({}); pass --instance to disambiguate",
            want_kind,
            many.join(", ")
        )),
    }
}
```

- [ ] **Step 2: Tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::*;

    fn cfg_with_instances(specs: &[(&str, InstanceKind)]) -> Config {
        let instances = specs.iter().map(|(name, kind)| InstanceConfig {
            name: name.to_string(),
            spec: match kind {
                InstanceKind::Ingest => InstanceSpec::Ingest(IngestInstance {
                    connections: vec![],
                    cycle_interval: std::time::Duration::from_secs(60),
                }),
                InstanceKind::Mcp => InstanceSpec::Mcp(McpInstance {
                    expose: vec![],
                    api_key: "k".into(),
                    knowledge_base: "kb".into(),
                    listen: "127.0.0.1:8080".into(),
                }),
            },
        }).collect();
        Config {
            azure: AzureConfig {
                cosmos: CosmosConfig {
                    subscription_id: None, resource_group: None, account: None,
                    endpoint: "https://x".into(), database: "quelch".into(),
                    containers: ContainerLayout::default(),
                    meta_container: "quelch-meta".into(),
                },
                search: None, ai: None,
            },
            source_connections: vec![],
            instances,
        }
    }

    #[test]
    fn resolve_picks_single_ingest_when_no_flag() {
        let cfg = cfg_with_instances(&[("only", InstanceKind::Ingest)]);
        assert_eq!(resolve_instance(&cfg, None, InstanceKind::Ingest).unwrap(), "only");
    }

    #[test]
    fn resolve_requires_flag_when_multiple_match() {
        let cfg = cfg_with_instances(&[("a", InstanceKind::Ingest), ("b", InstanceKind::Ingest)]);
        let err = resolve_instance(&cfg, None, InstanceKind::Ingest).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("a") && msg.contains("b"));
        assert!(msg.contains("--instance"));
    }

    #[test]
    fn resolve_errors_on_kind_mismatch() {
        let cfg = cfg_with_instances(&[("only", InstanceKind::Mcp)]);
        let err = resolve_instance(&cfg, Some("only"), InstanceKind::Ingest).unwrap_err();
        assert!(err.to_string().contains("expected") || err.to_string().contains("Ingest"));
    }

    #[test]
    fn resolve_errors_when_no_matching_kind() {
        let cfg = cfg_with_instances(&[("a", InstanceKind::Mcp)]);
        let err = resolve_instance(&cfg, None, InstanceKind::Ingest).unwrap_err();
        assert!(err.to_string().contains("no") && err.to_string().contains("Ingest"));
    }
}
```

- [ ] **Step 3: Wire into `quelch ingest` and `quelch mcp` dispatch arms**

```rust
Commands::Ingest { config, instance } => {
    let cfg = load_config(&config)?;
    let name = resolve_instance(&cfg, instance.as_deref(), InstanceKind::Ingest)?;
    ingest::worker::run(&cfg, name).await
}
Commands::Mcp { config, instance } => {
    let cfg = load_config(&config)?;
    let name = resolve_instance(&cfg, instance.as_deref(), InstanceKind::Mcp)?;
    mcp::server::run(&cfg, name).await
}
```

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test --workspace
git add -A
git commit -m "feat(cli): auto-detect --instance when only one of the right kind is declared"
```

---

## Phase 9 — `init` wizard rewrite

### Task 9.1: Rewrite prompts for new schema

**Files:**
- Modify: `crates/quelch/src/init/prompts.rs`
- Modify: `crates/quelch/src/init/mod.rs`
- Possibly delete: `crates/quelch/src/init/templates.rs` if templates reference the old schema; replace with new templates referencing the new schema.

- [ ] **Step 1: Audit current prompts and trim**

```bash
grep -n "deployment\|target\|key_vault\|container_apps_env\|application_insights" crates/quelch/src/init/prompts.rs
```

Every match is a prompt that goes away. The new prompt set:

1. Subscription / resource group / region (kept).
2. Cosmos account name + endpoint (kept).
3. AI Search service endpoint (kept; no service-name prompt).
4. AI provider (Foundry vs Azure OpenAI), endpoint, embedding deployment, chat deployment (kept).
5. Source connection — **looped**: name, type (jira/confluence), base URL, auth (PAT or basic), projects/spaces. User can add as many as they want.
6. Instance — **looped**: name, kind (ingest/mcp), then either:
   - For ingest: which connections to use (multi-select), cycle interval.
   - For mcp: which data sources to expose (multi-select), API key env var name, listen address, KB name.

Drop entirely:
- "Where will deployments run? (Azure / on-prem)" prompt.
- Container Apps env / App Insights / Key Vault prompts.
- "Skip role assignments?" prompt.

- [ ] **Step 2: Rewrite the wizard top-level in `init/mod.rs`**

Single function that builds a `Config` value by calling out to small per-section prompt helpers, then `serde_yaml::to_string` writes it out.

- [ ] **Step 3: Tests**

Wizard tests typically use mocked prompt responses. If the existing test scaffolding works, port one or two key tests (e.g., "user adds two connections and one ingest instance referencing both" → resulting Config has the right shape).

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test --workspace
git add -A
git commit -m "refactor(init): wizard rewrite for instances + source_connections schema"
```

---

## Phase 10 — Documentation

Each docs file is its own task because each is a distinct rewrite. Where possible, work from the spec section that describes the new behaviour.

### Task 10.1: `docs/getting-started.md`

**Files:**
- Modify: `docs/getting-started.md` (full rewrite)

- [ ] **Step 1: Rewrite end-to-end** with section headings that literally mirror the user-described 11-step journey. The doc reads as a follow-along — each section is one step the user is currently taking.

Required section structure:

```
# Getting started

(intro paragraph: who this is for, what you'll have at the end, that
 you'll be hosting Q-Ingest / Q-MCP yourself rather than Quelch deploying them)

## 1. Install Quelch
   - brew / cargo install commands
   - verify: `quelch --version`

## 2. Create the Azure resources Quelch depends on
   - resource group, Cosmos account, AI Search service (Basic+ with semantic ranker enabled),
     Foundry project or Azure OpenAI account holding embedding + chat deployments
   - exact `az` commands (copy-pasteable)
   - "you can use Bicep / Terraform / portal — Quelch doesn't care" note
   - explicit list of required RBAC roles for the operator running `quelch azure apply`
     (Cosmos DB Operator on the account, Search Service Contributor on the service,
     Cognitive Services User on the AI provider — verify exact role names in the impl)

## 3. Configure Azure resources with Q-CLI
   - `quelch init` (interactive wizard — what it asks, what it writes)
   - `quelch validate`
   - `quelch azure plan`  → read the diff
   - `quelch azure apply` → apply

## 4. Test Q-Ingest locally against one source
   - export the env vars referenced in quelch.yaml
   - `quelch ingest --config quelch.yaml --instance ingest-jira-internal`
   - `quelch status` in another terminal — confirm document count climbs

## 5. Test Q-MCP locally
   - export QUELCH_MCP_API_KEY
   - `quelch mcp --config quelch.yaml --instance mcp-prod`
   - `curl` the `tools/list` JSON-RPC, confirm 5 tools come back

## 6. Add additional sources
   - edit quelch.yaml (more source_connections; add to existing instance OR add a new instance)
   - re-run `quelch validate` (notice the static conflict check if you accidentally overlap)
   - re-run `quelch azure apply` (notice it's idempotent — only diffs apply)
   - test the new ingest locally

## 7. Move Q-Ingest to production
   - `quelch instance config ingest-jira-internal --kind ingest --output q-ingest.yaml`
   - copy q-ingest.yaml to your host
   - set credential env vars in the host's secret store
   - run `quelch ingest --config q-ingest.yaml` — pointer to docs/hosting.md for
     concrete Docker / systemd / k8s / Container App snippets

## 8. Verify the deployed Q-Ingest is working
   - `quelch status` from your laptop reads quelch-meta in Cosmos
   - `quelch status --tui` for the live dashboard
   - cursor ownership: hint that running a second instance against the same source
     fails fast — link to docs/hosting.md "ownership transfer" subsection if needed

## 9. Move Q-MCP to production
   - `quelch instance config mcp-prod --kind mcp --output q-mcp.yaml`
   - same hosting choice as Q-Ingest, link to docs/hosting.md
   - generate the API key per docs/api-key.md, store it in the host's secret store

## 10. Monitor and configure the deployed instances
   - `quelch status [--tui]` — read sync state
   - `quelch query | search | get` — operator queries against the data
   - `quelch reset --instance NAME --source ... --subsource ...` — reset a stuck cursor
   - `quelch azure indexer run|reset|status` — nudge AI Search indexers
   - to change config: edit quelch.yaml, validate, `quelch azure apply`,
     re-emit per-instance configs, restart hosts

## 11. Add more sources later
   - same as step 6, then 7 if a new ingest instance is needed
   - `quelch instance list` shows current instances

---

## Try it offline first with `quelch dev`
   - run the all-in-one local sandbox (mock Jira/Confluence + in-memory Cosmos + ingest + mcp)
   - useful before step 2 if you want to evaluate Quelch with no Azure spend
```

Each section corresponds directly to one step of the user's stated workflow — section 4 is "test Q-Ingest locally", step 4 in the user journey is "Testing the first Q-Ingest instance locally", etc. Anyone reading the doc top-to-bottom is following the exact path the new model is designed for.

The "Plan / Deploy" sections from the old doc collapse into section 3 (`azure plan` / `azure apply`). The `azure deploy` → `azure apply` terminology change must be applied throughout. Any reference to Container Apps, Key Vault, Application Insights, or "Quelch deploys" is gone — section 7 / 9 explicitly hand off hosting to the user with a link to `docs/hosting.md`.

- [ ] **Step 2: Verify rendered markdown reads cleanly**

```bash
glow docs/getting-started.md  # if installed
# or open in a viewer of choice
```

- [ ] **Step 3: Commit**

```bash
git add docs/getting-started.md
git commit -m "docs: rewrite getting-started for no-deploy pivot"
```

---

### Task 10.2: `docs/deployment.md` → `docs/hosting.md`

**Files:**
- Move: `docs/deployment.md` → `docs/hosting.md`
- Modify: rewrite contents.

- [ ] **Step 1: Rename**

```bash
git mv docs/deployment.md docs/hosting.md
```

- [ ] **Step 2: Rewrite as "host Q-Ingest / Q-MCP yourself"**

Sections:
- "What Quelch generates" — only the per-instance config file (`quelch instance config`). Quelch generates no Docker / systemd / k8s artefacts. (One paragraph.)
- "Running Q-Ingest as a Docker container" — copy-paste `docker run` snippet using the `ghcr.io/mklab-se/quelch:<version>` image, mounting the per-instance config, setting env vars.
- "Running Q-Ingest under systemd" — copy-paste unit file.
- "Running Q-Ingest in Kubernetes" — copy-paste Deployment + ConfigMap + Secret YAML.
- "Running Q-Ingest as an Azure Container App" — copy-paste `az containerapp create` snippet (or ARM JSON / Bicep that the user owns).
- Repeat for Q-MCP (or a single combined section if the snippets are nearly identical).
- "Generating an API key" — `openssl rand -base64 32`, where to store it, what env var the per-instance config references.

The snippets are illustrative — Quelch does not generate them. State this explicitly at the top.

- [ ] **Step 3: Update cross-references**

```bash
grep -rn "deployment.md" docs/ CLAUDE.md crates/quelch/src/ 2>/dev/null
```

Replace `deployment.md` → `hosting.md` everywhere.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "docs: rename deployment.md → hosting.md, rewrite as user-side hosting guide"
```

---

### Task 10.3: `docs/architecture.md`

**Files:**
- Modify: `docs/architecture.md`

- [ ] **Step 1: Drop the "Provisioning split: Bicep vs rigg" section**

Search for that section header (or whatever it's called) and delete it.

- [ ] **Step 2: Drop Container Apps framing throughout**

Replace any wording that says "Quelch deploys X to Container Apps" with "Quelch generates per-instance config; the user runs Q-Ingest / Q-MCP wherever they like".

- [ ] **Step 3: Update the module map at the bottom**

Replace with the spec's Module map section.

- [ ] **Step 4: Commit**

```bash
git add docs/architecture.md
git commit -m "docs(architecture): drop deploy/provisioning framing"
```

---

### Task 10.4: `docs/configuration.md`

**Files:**
- Modify: `docs/configuration.md` (full rewrite)

- [ ] **Step 1: Rewrite covering** the master schema, per-instance slimming rules, and all field semantics. Use the spec's schema sections as the source of truth.

- [ ] **Step 2: Commit**

```bash
git add docs/configuration.md
git commit -m "docs(configuration): rewrite for new schema"
```

---

### Task 10.5: `docs/cli.md`

**Files:**
- Modify: `docs/cli.md` (full rewrite)

- [ ] **Step 1: One section per top-level command in the spec's CLI surface table**

For each command, document: synopsis, what it does, every flag, one example, what it errors on.

- [ ] **Step 2: Commit**

```bash
git add docs/cli.md
git commit -m "docs(cli): rewrite for new verb set"
```

---

### Task 10.6: Light edits

**Files:**
- Modify: `docs/sync.md`
- Modify: `docs/mcp-api.md`
- Modify: `docs/agent-generation.md`
- Modify: `docs/examples.md`

- [ ] **Step 1: Search each for deployment-shaped wording**

```bash
for f in docs/sync.md docs/mcp-api.md docs/agent-generation.md docs/examples.md; do
  echo "=== $f ==="
  grep -nE "deploy(ment)?|Container App|Bicep|Key Vault|Application Insights|rigg/|generate-deployment" "$f" || true
done
```

For each match, decide: rephrase to use the new vocabulary (instances, hosting, `quelch instance config`), or delete the paragraph entirely.

- [ ] **Step 2: Commit**

```bash
git add -A
git commit -m "docs: scrub deployment-era wording from sync/mcp-api/agent-generation/examples"
```

---

### Task 10.7: New `docs/api-key.md`

**Files:**
- Create: `docs/api-key.md`

- [ ] **Step 1: Short doc (~80 lines)** covering:
  - Why Q-MCP needs an API key (auth between agent and MCP).
  - How to generate one (`openssl rand -base64 32`).
  - Where to put it (env var on the host that runs Q-MCP, e.g. `QUELCH_MCP_API_KEY`).
  - How to reference it in the per-instance config (`api_key: ${QUELCH_MCP_API_KEY}`).
  - How to rotate (generate new value, update host secret store, restart Q-MCP).

- [ ] **Step 2: Link from `docs/getting-started.md` and `docs/hosting.md` and `docs/mcp-api.md`**

- [ ] **Step 3: Commit**

```bash
git add docs/api-key.md docs/getting-started.md docs/hosting.md docs/mcp-api.md
git commit -m "docs: add api-key.md (replaces mcp-key set/rotate/show flow)"
```

---

### Task 10.8: `CLAUDE.md`

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Update the architecture paragraph and module map** to match the spec's Module map section.

- [ ] **Step 2: Drop any reference to** `azure/deploy/`, `onprem/`, `generate-deployment`, "deploys the Container App", "Bicep generator". Add `azure/cosmos_config/`.

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs(CLAUDE.md): align with no-deploy pivot module map"
```

---

## Phase 11 — Verification

### Task 11.1: Spec acceptance signals

Walk through every line of the spec's "Acceptance signals" section and verify each.

- [ ] **Step 1: Pre-push gate green**

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

- [ ] **Step 2: New-user end-to-end**

Run through the rewritten `docs/getting-started.md` against a real Azure subscription with a fresh resource group. (If you don't have one, simulate by checking each `quelch` command runs without crashing on a hand-crafted `quelch.yaml`.)

- [ ] **Step 3: Conflict validation**

Hand-craft a `quelch.yaml` with two ingest instances claiming overlapping subsources. Run `quelch validate`. Expect a precise error naming both instances and the conflicting subsource.

- [ ] **Step 4: Cursor ownership refusal**

With `quelch dev` running (or in-memory backend test), pre-seed a cursor with `owner_instance="A"` and start a Q-Ingest configured as instance B against the same source. Confirm B exits hard with the ownership error message.

- [ ] **Step 5: `azure apply` idempotence**

Against an empty AI Search service and a Cosmos account with no `quelch` database: `quelch azure apply` creates everything. A second run reports zero changes.

- [ ] **Step 6: String scrub**

```bash
git grep -nE "Bicep|Container App|Key Vault|Application Insights|naming\.prefix|skip_role_assignments|rigg\.ownership" \
  -- ':!docs/superpowers/specs/' ':!docs/superpowers/plans/'
```

Expected: no matches.

- [ ] **Step 7: Final commit if anything changed during verification**

```bash
git status
# if dirty:
git add -A
git commit -m "chore: post-verification cleanups"
```

- [ ] **Step 8: Push the branch**

```bash
git push -u origin feat/no-deploy-pivot
```

---

## Done

The pivot is complete when Phase 11 verification all passes. From here, the user reviews the branch, opens a PR (if desired), or merges directly to `main`.
