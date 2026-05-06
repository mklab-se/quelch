# Quelch — no-deploy pivot design spec

**Date:** 2026-05-06
**Status:** Approved during brainstorming. Awaiting written-spec review.
**Supersedes:** parts of [2026-04-30-quelch-rearchitecture-design.md](2026-04-30-quelch-rearchitecture-design.md) related to provisioning, Bicep, on-disk rigg files, and Container App deployment. The high-level data-flow architecture from that spec stays intact.
**No back-compat:** Quelch is unreleased. Cut cleanly; do not deprecate.

## Goal

Stop pretending Quelch is a deployment tool. Quelch becomes a **configuration tool** — it configures the Azure resources it depends on (Cosmos DB containers, Azure AI Search indexes / skillsets / indexers / knowledge sources / knowledge base) and emits per-instance config files that the user runs anywhere they like. The user is solely responsible for hosting Q-Ingest and Q-MCP processes; Quelch never deploys anything to Azure on the user's behalf.

The headline simplifications:

- No Bicep generated, ever.
- No on-disk `rigg/` directory; rigg is used as an embedded library only.
- No `azure deploy / pull / logs / destroy / generate-deployment` commands.
- No Container Apps env / Application Insights / Key Vault / managed identities / role assignments managed by Quelch.
- Each Q-Ingest / Q-MCP instance is named, has its own slim config file, and is started by the user with whatever supervisor they prefer (Docker, systemd, k8s, Container Apps, bare process — Quelch doesn't care).

## Non-goals

- Backwards compatibility — none.
- Generating per-target hosting templates (no `--as docker`, `--as systemd`, `--as k8s` artefact emission). Documentation gives copy-paste snippets; Quelch generates only the per-instance config.
- Quelch managing user-side secrets. Env-var references only; the user wires their own secret store.
- Quelch managing RBAC. The user wires their own service principals / managed identities / etc. against Cosmos / Search / AI provider.
- Quelch building or publishing container images as part of any user-facing workflow. The release pipeline still publishes `ghcr.io/mklab-se/quelch:<version>` for users to pull, but no CLI command interacts with the image registry.

## User journey

The 11-step path the new design must support, translating the user's stated workflow into commands:

1. Install Quelch (`brew install` / `cargo install`).
2. Create the Azure resources Quelch depends on (resource group, Cosmos account, AI Search service, Foundry project or Azure OpenAI, embedding + chat deployments). User does this with `az` or any IaC tool of their choice.
3. `quelch init` → write `quelch.yaml` interactively. `quelch azure apply` → create Cosmos containers + AI Search indexes etc.
4. `quelch ingest --config quelch.yaml --instance ingest-jira-internal` → run a Q-Ingest locally against one source for testing.
5. `quelch mcp --config quelch.yaml --instance mcp-prod` → run Q-MCP locally. `curl` it to verify it starts and responds.
6. Add more sources by editing `quelch.yaml` (more `source_connections`, more `instances`); re-run `quelch azure apply`. Test additional Q-Ingest instances locally.
7. Move Q-Ingest to production — user emits `quelch instance config <name> --kind ingest > q-ingest-X.yaml`, copies it to wherever they host (k8s pod / VM / Container App / whatever), sets env vars for credentials, runs `quelch ingest --config q-ingest-X.yaml`.
8. `quelch status` reads `quelch-meta` from Cosmos to confirm the deployed Q-Ingest is making progress.
9. Same flow for Q-MCP: emit per-instance config, host wherever, set `QUELCH_MCP_API_KEY`, run `quelch mcp --config q-mcp-prod.yaml`.
10. `quelch status`, `quelch query`, `quelch search` etc. for ongoing operator visibility.
11. New sources: edit `quelch.yaml`, `quelch validate`, `quelch azure apply`, regenerate the per-instance config, restart the instance.

## Master `quelch.yaml` schema

One file, checked into the user's git repo. The same schema is used for the on-host per-instance file (the per-instance file is just a slimmer slice — same parser).

```yaml
# Azure resources Quelch will configure (not deploy).
# All three sub-blocks describe pre-existing resources; Quelch configures
# their internals via control-plane REST (Cosmos containers) and
# rigg-as-library (AI Search).
azure:
  cosmos:
    # Control-plane fields — used by `quelch azure apply` to PUT containers.
    # Stripped from per-instance Q-Ingest / Q-MCP configs (runtime is data-plane only).
    subscription_id: ${AZURE_SUBSCRIPTION_ID}
    resource_group:  rg-quelch-prod
    account:         my-cosmos
    # Data-plane fields — used by every role.
    endpoint:        https://my-cosmos.documents.azure.com
    database:        quelch
    # Container layout — overridable per source-connection.
    containers:
      jira_issues:       jira-issues
      jira_sprints:      jira-sprints
      jira_fix_versions: jira-fix-versions
      jira_projects:     jira-projects
      confluence_pages:  confluence-pages
      confluence_spaces: confluence-spaces
    meta_container:      quelch-meta
  search:
    # Endpoint only — rigg-as-library uses the admin REST API + DefaultAzureCredential.
    # No ARM-level fields needed.
    endpoint: https://my-search.search.windows.net
  ai:
    # Wired into AI Search at apply-time (KB / vectoriser config).
    # Q-MCP does not call the AI provider directly at runtime.
    provider: foundry           # or: azure_openai
    endpoint: https://...
    embedding: { deployment: text-embedding-3-large, dimensions: 3072 }
    chat:      { deployment: gpt-5-mini, model_name: gpt-5-mini }

# Reusable named source connections (one entry per (base_url × credential) tuple).
# Instances reference these by name. This is what lets one Q-Ingest use
# multiple PATs without duplicating connection metadata.
source_connections:
  - name: jira-internal-pat-x
    type: jira
    base_url: https://jira.internal/
    auth: { kind: pat, token: ${JIRA_PAT_X} }
    projects: [DO, ANNA, SARA]

  - name: jira-internal-pat-y
    type: jira
    base_url: https://jira.internal/
    auth: { kind: pat, token: ${JIRA_PAT_Y} }
    projects: [EMMA, IT, LISA]

  - name: confluence-internal
    type: confluence
    base_url: https://confluence.internal/
    auth: { kind: pat, token: ${CONFLUENCE_PAT} }
    spaces: [ENG, DOCS]

# Named instances. Q-CLI generates a per-instance slice from each entry.
# Each entry corresponds to one process the user will run somewhere.
instances:
  - name: ingest-jira-internal
    kind: ingest
    connections: [jira-internal-pat-x, jira-internal-pat-y]
    cycle_interval: 5m

  - name: ingest-confluence-internal
    kind: ingest
    connections: [confluence-internal]
    cycle_interval: 10m

  - name: mcp-prod
    kind: mcp
    expose: [jira_issues, jira_sprints, confluence_pages]
    api_key: ${QUELCH_MCP_API_KEY}
    knowledge_base: quelch-prod-kb
    listen: 0.0.0.0:8080
```

### Fields gone vs. today's schema

- `deployments[]` — replaced by `instances[]`.
- `azure.naming` — gone (no resources to name; user names their own hosts).
- `azure.skip_role_assignments` — gone (no role assignments).
- `azure.resources.{container_apps_env, application_insights, key_vault, *_resource_group}` — gone.
- `rigg.ownership` and the on-disk `rigg/` directory — gone.
- Top-level `cosmos:` — collapsed into `azure.cosmos:` (was an awkward split between connectivity and layout).
- `state.*` — collapsed into `azure.cosmos.meta_container`.
- `azure.search.{subscription_id, resource_group, service}` — gone (rigg-as-library only needs the endpoint URL).

### Auth model

Credentials are referenced as env-var placeholders only (`${VAR}`). The runtime resolves them at config-load time. Missing env vars produce a precise error from `quelch validate --config <file>`. The user wires their host's secret store however they want (`.env`, k8s `Secret`, systemd `EnvironmentFile`, Container Apps secret refs).

For Azure-side authentication (Cosmos control plane, AI Search admin API), Quelch CLI uses `DefaultAzureCredential` — i.e. the user's `az login` token chain. There is no Quelch-managed identity.

## Per-instance config schema

Same parser, narrower content. Emitted by `quelch instance config <name> --kind ingest|mcp`.

**Q-Ingest** (only writes to Cosmos data plane; control-plane info stripped):

```yaml
azure:
  cosmos:
    endpoint: https://my-cosmos.documents.azure.com
    database: quelch
    containers: { jira_issues: jira-issues, ... }   # only the ones this instance writes to
    meta_container: quelch-meta
source_connections:
  - { name: jira-internal-pat-x, ... }
  - { name: jira-internal-pat-y, ... }
instances:
  - { name: ingest-jira-internal, kind: ingest,
      connections: [jira-internal-pat-x, jira-internal-pat-y],
      cycle_interval: 5m }
```

**Q-MCP** (queries Cosmos data plane + Search; AI provider is wired into the KB at apply-time, Q-MCP itself doesn't call AI at runtime):

```yaml
azure:
  cosmos:
    endpoint: https://my-cosmos.documents.azure.com
    database: quelch
    containers: { jira_issues: jira-issues, ... }
    meta_container: quelch-meta
  search:
    endpoint: https://my-search.search.windows.net
instances:
  - { name: mcp-prod, kind: mcp,
      expose: [jira_issues, jira_sprints, confluence_pages],
      api_key: ${QUELCH_MCP_API_KEY},
      knowledge_base: quelch-prod-kb,
      listen: 0.0.0.0:8080 }
```

The slimming rules:
- `azure.cosmos.{subscription_id, resource_group, account}` — absent from both Q-Ingest and Q-MCP configs (only `quelch azure apply` needs control-plane fields).
- `azure.search` — absent from Q-Ingest configs (Q-Ingest never reads from Search).
- `azure.ai` — absent from both (AI is configured into the KB at apply-time; neither role calls AI at runtime).
- `source_connections` — absent from Q-MCP configs (Q-MCP doesn't pull from sources).
- `azure.cosmos.containers` — only the entries that role actually touches.

### Auto-detect rule for `--instance`

`quelch ingest` and `quelch mcp` accept `--config <file>`. If `--instance <name>` is omitted:

- If the file has exactly one instance whose `kind` matches the binary (`ingest` or `mcp`), use it.
- Otherwise fail with a clear message listing the candidate instance names.

This lets a per-instance file run with no flag and lets the master file be used as-is for dev ergonomics if it happens to declare only one ingest (or one mcp) instance.

## CLI surface

```
quelch init                                      # wizard, writes quelch.yaml
quelch validate [--config FILE]                  # sanity-check master or per-instance

quelch instance list                             # list named instances in master
quelch instance config NAME --kind ingest|mcp    # emit per-instance config
        [--output PATH]                          # default stdout

quelch azure plan                                # show diff (cosmos + search)
quelch azure apply                               # apply diff (cosmos + search)
quelch azure indexer run|reset|status [NAME]     # operate AI Search indexers

quelch ingest --config FILE [--instance NAME]    # run Q-Ingest worker
quelch mcp    --config FILE [--instance NAME]    # run Q-MCP server

quelch dev                                       # all-in-one local sandbox
quelch mock                                      # local mock Jira / Confluence

quelch status [--tui]                            # read quelch-meta
quelch reset --instance NAME --source ...        # reset cursor (cf. ownership-transfer flag below)
quelch query | search | get                      # operator queries

quelch agent generate --target T                 # agent / skill bundle
```

### Verbs gone

- `effective-config` → folded into `instance config`.
- `generate-deployment` → folded into `instance config` (no per-target scaffolding).
- `azure deploy` → renamed `azure apply`.
- `azure pull` → gone (no on-disk rigg files; manual edits get reverted by next `azure apply`).
- `azure logs` → gone (Quelch isn't running the workload anymore; user uses their host's log system).
- `azure destroy` → gone (Quelch doesn't own the workload).
- `mcp-key set / rotate / show` → all gone. `mcp-key generate` also gone. Documented procedure: user generates their own key (`openssl rand -base64 32`), stores it in their host's secret store, sets `QUELCH_MCP_API_KEY` (or whatever name they prefer; the per-instance config's `api_key:` field can reference any env var).

### Mutating Azure command

`quelch azure apply` is idempotent and the only command that writes to Azure. It does two things, in order:

**A. Cosmos DB containers (no Bicep):**
- Authenticate via `DefaultAzureCredential`.
- Use ARM REST (`PUT /subscriptions/.../databaseAccounts/{acct}/sqlDatabases/{db}/containers/{name}`) to ensure database + containers exist with correct partition keys.
- If a container exists with a mismatched partition key, fail loudly. Never auto-recreate (would lose data).

**B. Azure AI Search via rigg-as-library:**
- Build desired state in-memory from `quelch.yaml`: indexes (one per Cosmos container exposed by any MCP instance), skillsets, indexers, data sources, knowledge sources, one knowledge base per MCP instance.
- Use rigg's existing diff / apply logic against the live AI Search service.
- No on-disk `rigg/` directory. No hand-takeover marker. No `quelch azure pull`.
- Drift surfaces in `azure plan` as "these will be reverted"; `apply` reverts. Users wanting manual control over a specific resource remove it from `quelch.yaml` and use the standalone `rigg` tool — Quelch and `rigg`-the-tool are mutually exclusive per resource.

`quelch azure plan` is the same logic but prints the diff and exits without applying.

### Indexer operations

`quelch azure indexer run|reset|status` stays — they're operator-side commands that nudge AI Search Indexers (force-run, reset state, check status). They don't mutate config; they trigger runtime operations on already-configured indexers. Implemented via the AI Search admin REST API.

## Conflict prevention

Two named ingest instances pulling overlapping source slices would race on the same Cosmos partition and clobber each other's documents. Two layers of protection:

**Static — at `quelch validate`:** build a claim set per ingest instance. Each entry is a `(source_type, base_url, subsource)` tuple, e.g. `(jira, https://jira.internal/, DO)`. If any tuple is claimed by ≥2 ingest instances, fail with a clear message naming the conflict and both instances.

**Dynamic — at runtime:** every cursor doc in `quelch-meta` carries an `owner_instance` field. Q-Ingest writes its instance name when it first claims a cursor; on every subsequent cursor write, if the doc's owner ≠ this instance's name, refuse with a hard error and exit (don't loop). This catches misconfiguration where someone independently starts a second instance against the same Cosmos without going through `quelch validate`.

**Ownership transfer:** `quelch reset --instance NEW_OWNER --source ... --take-ownership` rewrites the owner field. Without `--take-ownership`, `reset` operates only on cursors already owned by the named instance.

## Module map (after the change)

```
crates/quelch/src/
├── main.rs
├── cli.rs              — pruned verb set
├── config/             — new schema (instances[], source_connections[]); deployment fields gone
├── sources/            — unchanged
├── ingest/             — reads instance config, writes owner_instance on cursors
├── cosmos/             — unchanged at the data-plane layer
├── mcp/                — reads instance config, no deployment-target branching
├── azure/
│   ├── cosmos_config/  — NEW: control-plane container CRUD via ARM REST
│   ├── apply.rs        — NEW: orchestrator for `azure apply` (cosmos_config + rigg)
│   ├── plan.rs         — NEW: orchestrator for `azure plan`
│   ├── indexer.rs      — kept (AI Search indexer ops)
│   └── rigg/           — kept logic, on-disk I/O removed; pure in-memory now
├── agent/              — unchanged
├── commands/           — pruned (no deployment-target branching)
├── init/               — wizard rewrite for instances, no deploy-target prompts
├── dev/                — unchanged
├── tui/                — unchanged
├── sim/, mock/         — unchanged
└── ai.rs               — unchanged
```

### Files deleted outright

- `crates/quelch/src/azure/deploy/bicep.rs`
- `crates/quelch/src/azure/deploy/apply.rs` (the Bicep applier — different from the new top-level `azure/apply.rs`)
- `crates/quelch/src/azure/deploy/destroy.rs`
- `crates/quelch/src/azure/deploy/logs.rs`
- `crates/quelch/src/azure/deploy/whatif.rs`
- `crates/quelch/src/azure/deploy/diff_view.rs`
- `crates/quelch/src/azure/deploy/naming.rs`
- `crates/quelch/src/azure/deploy/mod.rs`
- `crates/quelch/src/azure/deploy/snapshots/` (whole directory)
- `crates/quelch/src/azure/rigg/write.rs`
- `crates/quelch/src/azure/rigg/pull.rs`
- `crates/quelch/src/azure/rigg/ownership.rs`
- `crates/quelch/src/onprem/` (whole directory)

### Heavily refactored

- `crates/quelch/src/azure/rigg/{generate,plan,push}.rs` — keep the AI Search-side logic, drop file I/O. Pure in-memory desired-state computation + diff + apply.
- `crates/quelch/src/azure/mod.rs` — re-exports new `apply` / `plan` / `cosmos_config` / `indexer` / `rigg` submodules.
- `crates/quelch/src/config/{schema,slice,validate}.rs` — `deployments` → `instances`; drop deploy-target enum; add `source_connections[]`; add static conflict validation.
- `crates/quelch/src/init/` — wizard prompts for instances and connections only. No "azure / on-prem" target prompt. No Container Apps env / Key Vault prompts.
- `crates/quelch/src/cli.rs` — verb pruning per the CLI surface section.
- `crates/quelch/src/commands/` — strip deployment-target branching.

## Documentation plan

- `docs/getting-started.md` — full rewrite. Mirrors the 11-step user journey above. Steps 4–5 in the current doc (Plan / Deploy) collapse into one step "Configure Azure" (`quelch azure apply`). New steps after that: "Run Q-Ingest locally", "Run Q-MCP locally", "Move to production with `quelch instance config`".
- `docs/deployment.md` → renamed `docs/hosting.md`. Reframed as "how to host Q-Ingest / Q-MCP yourself", with copy-paste snippets for Docker, systemd, k8s, Azure Container Apps. Snippets are example / illustrative — Quelch generates none of them.
- `docs/architecture.md` — drop "provisioning split", drop Container App framing, drop on-disk rigg framing. The data-flow architecture (sources → Cosmos → AI Search → KB → Q-MCP → agent) stays.
- `docs/configuration.md` — full rewrite for the new schema (master + per-instance, with the slimming rules).
- `docs/cli.md` — full rewrite for the new verb set.
- `docs/sync.md`, `docs/mcp-api.md`, `docs/agent-generation.md`, `docs/examples.md` — light edits to remove deployment-shaped wording.
- `CLAUDE.md` — adjust the architecture paragraph and module map.
- New short doc: `docs/api-key.md` (or a section in `mcp-api.md`) — explains how to generate a Q-MCP API key (`openssl rand -base64 32`) and wire it into the user's host secret store. Replaces today's `mcp-key set/rotate/show` workflow.

## Open questions

None at design time. Two decisions deferred to implementation:

- Exact JSON shape of the `azure plan` / `azure apply` combined diff (Cosmos changes + AI Search changes). Stylistically borrow from the current `diff_view.rs` rendering before deleting it.
- Whether `quelch azure indexer ...` keeps its current verb shape or moves under a flatter namespace. Lean: keep current shape; it's a small command set and the namespace makes it discoverable.

## Out of scope for this spec

- Re-architecting the simulator, mock servers, or `quelch dev`. They stay as-is.
- Changing the document model or canonical fields ingested from Jira / Confluence.
- Changing the MCP tool API or its 5-tool surface.
- Multi-tenant / SaaS shapes.

## Acceptance signals

The pivot is "done" when:

- `cargo build --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`, `cargo fmt --all -- --check` all pass.
- A new user can follow the rewritten `docs/getting-started.md` end-to-end with only `az`, the Quelch CLI, their preferred host (Docker / systemd / k8s / Container Apps), and the credentials for one Jira and one Confluence source — and reach a working agent query.
- `quelch validate` flags an intentionally-overlapping `instances[]` configuration with a precise message naming both instances and the conflicting subsource.
- A second Q-Ingest started against an already-claimed cursor exits hard with the owner-mismatch error.
- `quelch azure apply` against an empty AI Search service stands the indexes / skillsets / indexers / KS / KB up correctly; against a Cosmos account with no `quelch` database, creates the database and the required containers; on a second invocation, reports zero changes.
- No `Bicep`, `Container App`, `Key Vault`, `Application Insights`, `naming.prefix`, `skip_role_assignments`, or `rigg.ownership` strings remain anywhere in the source tree or docs (excluding this spec and prior specs in `docs/superpowers/specs/`).
