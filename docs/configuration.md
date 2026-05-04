# Configuration reference

Quelch is configured by a single `quelch.yaml` file that you version-control alongside your project. This file is the **source of truth**: Quelch reconciles Azure to it. You edit YAML, you run `quelch azure plan`, you review the diff, you run `quelch azure deploy`.

This document describes every section of the file.

## Top-level shape

```yaml
azure:        { ... }   # subscription, resource group, region, naming
cosmos:       { ... }   # account, database, default container names
search:       { ... }   # AI Search service shell (name, SKU). Internals are rigg-managed.
openai:       { ... }   # endpoint and embedding deployment used by the rigg-managed skillsets
sources:      [ ... ]   # named source instances (Jira, Confluence)
ingest:       { ... }   # global ingest worker behaviour (poll cadence, safety lag, reconcile)
deployments:  [ ... ]   # named workers — each is a slice of `sources` with a target host
mcp:          { ... }   # MCP service config (exposed data sources, auth, search backend)
rigg:         { ... }   # where Quelch writes generated rigg files
state:        { ... }   # where ingest cursors live
```

A complete minimal example (one cloud Jira + MCP in Azure) lives at the end of this document.

## `azure`

```yaml
azure:
  subscription_id: "${AZURE_SUBSCRIPTION_ID}"
  resource_group: "rg-quelch-prod"
  region: "swedencentral"
  naming:
    prefix: "quelch"          # all created resources will be named ${prefix}-...
    environment: "prod"
```

`subscription_id` and `resource_group` together identify where Quelch creates and reconciles resources. `region` is the Azure region used when Quelch creates new resources; existing resources keep their region.

`naming.prefix` and `naming.environment` are used to generate Azure resource names (e.g. `quelch-prod-cosmos`, `quelch-prod-search`). You can leave them out and supply names directly elsewhere if you want full control.

## `cosmos`

```yaml
cosmos:
  account: "quelch-prod-cosmos"     # Cosmos DB account name (auto-generated if absent)
  database: "quelch"
  containers:
    jira_issues:        "jira-issues"          # default — overridable per source
    confluence_pages:   "confluence-pages"
    jira_sprints:       "jira-sprints"
    jira_fix_versions:  "jira-fix-versions"
    jira_projects:      "jira-projects"
    confluence_spaces:  "confluence-spaces"
  meta_container:       "quelch-meta"          # cursors and worker state
  throughput:
    mode: "serverless"               # or "provisioned"
    # ru_per_second: 1000            # only when mode=provisioned
```

Defaults under `containers:` apply to any source that doesn't override its target container. Sources can opt out by setting `container:` on the source itself (see `sources` below).

`meta_container` is the shared `quelch-meta` container that holds cursors and per-worker state. See [architecture.md](architecture.md#state-model).

## `search`

```yaml
search:
  service: "quelch-prod-search"
  sku: "basic"                        # standard / standard2 / standard3 also OK
  indexer:
    schedule:
      interval: "PT15M"               # ISO 8601 duration; Indexer runs at this cadence
    high_water_mark_field: "updated"  # Cosmos field used by the indexer for incremental sync
```

`search` only configures the **AI Search service shell** that Bicep provisions — its name and SKU, plus the high-water-mark field convention used by the indexers. Everything *inside* the AI Search service (index schemas, skillsets, the indexer specs themselves, knowledge sources, knowledge bases) is managed by [rigg](https://github.com/mklab-se/rigg) — see the `rigg:` section below and [architecture.md](architecture.md#provisioning-split-bicep-vs-rigg).

`search.indexer.schedule.interval` is the cadence Quelch writes into the *generated* rigg indexer files. You can override per-indexer by hand-editing the rigg file once it's generated.

## `openai`

```yaml
openai:
  endpoint: "https://${AOI_ACCOUNT}.openai.azure.com"
  embedding_deployment: "text-embedding-3-large"
  embedding_dimensions: 3072
```

This is consumed by the rigg-managed skillsets — it's how the integrated vectoriser computes embeddings during indexing. Quelch ingest does not call OpenAI itself.

## `rigg`

```yaml
rigg:
  dir: "./rigg"                       # default; where Quelch writes generated rigg files
  ownership: "generated"              # "generated" (default) | "managed-by-user"
```

Quelch embeds the `rigg-core` and `rigg-client` crates directly — there's no separate `rigg` CLI to install. From `quelch.yaml`, Quelch generates a `rigg/` directory with files for every AI Search index, indexer, skillset, knowledge source, and knowledge base implied by the config. `quelch azure plan` and `quelch azure deploy` then plan and push them via the rigg library.

`ownership: "generated"` means Quelch overwrites the directory on each plan. `ownership: "managed-by-user"` means Quelch will only *generate* missing files; existing files are left alone for hand-tuning. You can also mix: a per-file marker comment (`# rigg:managed-by-user`) on a single file pins just that one to user ownership while the rest stay generated.

## `sources`

A list of source instances. Each one has a unique `name` that is used in deployments and as a prefix on document ids.

### Jira example (Cloud, with overrides)

```yaml
sources:
  - type: jira
    name: jira-cloud
    url: "https://example.atlassian.net"
    auth:
      email: "${JIRA_CLOUD_EMAIL}"
      api_token: "${JIRA_CLOUD_TOKEN}"
    projects: ["DO", "PROD", "INT"]
    container: "jira-issues-cloud"          # override: don't share the default container
    companion_containers:
      sprints:       "jira-sprints-cloud"
      fix_versions:  "jira-fix-versions-cloud"
      projects:      "jira-projects-cloud"
    fields:                                 # opt-in custom fields to ingest
      story_points: "customfield_10016"
      epic_link:    "customfield_10014"
```

### Jira example (Data Center, defaults)

```yaml
  - type: jira
    name: jira-internal
    url: "https://jira.internal.example"
    auth:
      pat: "${JIRA_INTERNAL_PAT}"
    projects: ["DO"]
    # container omitted → goes to default `jira-issues`
```

### Confluence example

```yaml
  - type: confluence
    name: confluence-internal
    url: "https://confluence.internal.example"
    auth:
      pat: "${CONFLUENCE_INTERNAL_PAT}"
    spaces: ["ENG", "OPS"]
    container: "confluence-pages-internal"
    companion_containers:
      spaces: "confluence-spaces-internal"
```

### Common source fields

| Field | Meaning |
|---|---|
| `type` | `jira` or `confluence`. |
| `name` | Unique identifier within the config. Used as document id prefix and in `quelch-meta`. |
| `url` | Base URL of the source instance. |
| `auth` | Either `{email, api_token}` (Cloud) or `{pat}` (Data Center). |
| `projects` / `spaces` | Subsource keys to ingest. Each becomes its own cursor. |
| `container` | Override default Cosmos container for primary entities (issues / pages). |
| `companion_containers` | Per-companion overrides. Defaults from `cosmos.containers`. |
| `fields` | Source-specific extras (e.g. Jira custom fields). |

## `ingest`

Global defaults for ingest worker behaviour. These knobs directly affect the [sync correctness algorithm](sync.md) — read that document before changing them.

```yaml
ingest:
  poll_interval: "300s"           # how often a worker tries to advance its window
  safety_lag_minutes: 2           # window upper bound = (now floored to minute) - this many minutes
  batch_size: 100                 # page size for source API calls
  reconcile_every: 12             # full deletion-reconciliation runs every Nth cycle
  max_cycle_duration: "30m"       # warn if a cycle takes longer than this; doesn't abort
  max_concurrent_per_source: 1    # in-flight source-API requests per source instance
  max_retries: 5                  # per-request retry cap on transient 5xx without Retry-After
```

| Knob | Default | What it controls |
|---|---|---|
| `poll_interval` | `300s` | Cycle cadence — how often a worker tries to advance its window. Shorter = fresher data, more API quota used. |
| `safety_lag_minutes` | `2` | How far behind real time the per-cycle window's upper bound stays. Absorbs Atlassian indexing lag. Increase if you see edge-of-minute drops; decrease for fresher data. Safe to change live. |
| `batch_size` | `100` | Page size for source API calls (`maxResults` for Jira, `limit` for Confluence). |
| `reconcile_every` | `12` | Full reconciliation pass every Nth cycle. With default `poll_interval` of 300s, that's ~60 minutes. Increase for large projects with low delete rates. |
| `max_cycle_duration` | `30m` | Logged warning threshold — long cycles are valid for big windows, this just flags them. |
| `max_concurrent_per_source` | `1` | Maximum concurrent in-flight requests to a single source instance. Atlassian rate-limits per account; concurrency rarely helps. |
| `max_retries` | `5` | Retry cap for transient 5xx responses without `Retry-After`. 429s (and 5xx with `Retry-After`) honour the server's value. |

These are global defaults. Future versions may allow per-source overrides; for now Quelch keeps it global to make the system easier to reason about.

## `deployments`

This is what makes the multi-instance story explicit. Each entry is a named worker with a role and a target.

```yaml
deployments:
  # On-prem ingest: Jira projects A through K
  - name: ingest-onprem-jira-ak
    role: ingest
    target: onprem
    sources:
      - source: jira-internal
        projects: ["A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K"]

  # On-prem ingest: Jira projects L through Z (and all Confluence spaces)
  - name: ingest-onprem-jira-lz
    role: ingest
    target: onprem
    sources:
      - source: jira-internal
        projects: ["L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z"]
      - source: confluence-internal

  # Cloud ingest: cloud Jira + cloud Confluence
  - name: ingest-azure-cloud
    role: ingest
    target: azure
    azure:
      container_app:
        cpu: 0.5
        memory: "1.0Gi"
        min_replicas: 1
        max_replicas: 1
    sources:
      - source: jira-cloud
      - source: confluence-cloud

  # MCP service in Azure
  - name: mcp-azure
    role: mcp
    target: azure
    azure:
      container_app:
        cpu: 1.0
        memory: "2.0Gi"
        min_replicas: 0      # scale-to-zero when idle
        max_replicas: 5
    expose:                  # logical data-source names — what agents see
      - jira_issues
      - jira_sprints
      - jira_fix_versions
      - jira_projects
      - confluence_pages
      - confluence_spaces
    auth:
      mode: "api_key"        # "api_key" (current) or "entra" (planned)
```

### Common deployment fields

| Field | Meaning |
|---|---|
| `name` | Unique identifier. Used by `quelch azure deploy <name>` and as the Container App name in Azure. |
| `role` | `ingest` or `mcp`. |
| `target` | `azure` (Quelch can deploy this) or `onprem` (Quelch generates artefacts for you to deploy). |
| `sources` | (ingest only) Which sources, optionally restricted to subsets of subsources. |
| `expose` | (mcp only) Which **logical data sources** (not physical containers) are visible to MCP clients. Anything not listed is invisible — defence in depth. The names must appear in `mcp.data_sources` (see below). |
| `azure.container_app` | (target=azure only) Container App sizing and scaling. |
| `auth` | (mcp only) Authentication mode. |

### Validation rules

Quelch validates that:

- Every `(source, subsource)` pair appears in **at most one** ingest deployment.
- Every name in any `expose:` list is defined in `mcp.data_sources` (or auto-derivable from the defaults).
- Every source referenced in a deployment exists in `sources`.

`quelch validate` runs all of these and prints diagnostics.

## `mcp`

The MCP section has two purposes: define the **logical data sources** the API exposes (mapped onto the physical Cosmos containers), and set global MCP defaults.

This is the layer that hides storage from agents. The `data_sources:` map is what makes "agents call `query(data_source: "jira_issues", ...)`" work even when there are multiple physical Jira containers underneath. See [architecture.md](architecture.md#two-layers-of-names).

```yaml
mcp:
  # Logical data sources. Map an API-layer name to one or more physical containers.
  # When omitted, Quelch derives sensible defaults from `sources` (see below).
  data_sources:
    jira_issues:
      kind: jira_issue
      backed_by:
        - container: jira-issues-internal
        - container: jira-issues-cloud
    jira_sprints:
      kind: jira_sprint
      backed_by:
        - container: jira-sprints-internal
        - container: jira-sprints-cloud
    jira_fix_versions:
      kind: jira_fix_version
      backed_by:
        - container: jira-fix-versions-internal
        - container: jira-fix-versions-cloud
    jira_projects:
      kind: jira_project
      backed_by:
        - container: jira-projects-internal
        - container: jira-projects-cloud
    confluence_pages:
      kind: confluence_page
      backed_by:
        - container: confluence-pages-internal
        - container: confluence-pages-cloud
    confluence_spaces:
      kind: confluence_space
      backed_by:
        - container: confluence-spaces-internal
        - container: confluence-spaces-cloud

  # The `search` MCP tool routes through an Azure AI Search Knowledge Base
  # (Agentic Retrieval) by default — better semantic results, built-in
  # query decomposition and reranking. Opt out per deployment for cost.
  search:
    disable_agentic: false        # default; set true to use direct hybrid search instead
    knowledge_base: "quelch-prod-kb"   # default name; rigg generates this

  # Global server defaults — overridden per deployment when relevant.
  default_top: 25
  max_top: 100
  query_timeout: "30s"
  search_timeout: "20s"
```

### Per-tool backend choice (not configurable)

The mapping between MCP tools and backends is fixed by the tool's *semantics*, not by config:

| Tool | Backend | Why |
|---|---|---|
| `search` | Azure AI Search **Knowledge Base** (Agentic Retrieval) | Tool exists for fuzzy semantic questions; decomposition + reranking are exactly what helps. |
| `query`, `get`, `aggregate` | Cosmos DB direct | Exact, exhaustive, structured — agentic retrieval would just add cost. |
| `list_sources` | Cached metadata | Static. |

The only knob is `mcp.search.disable_agentic` for cost-sensitive deployments — when set, `search` queries the underlying index directly instead of going through the knowledge base. The agent's view doesn't change; result quality drops.

### Auto-derived `data_sources`

If you omit `mcp.data_sources` entirely, Quelch derives one entry per `kind` from your `sources` and their `cosmos` defaults — i.e. the simple-installation case "just works":

| Default data source | Default kind | Default `backed_by` |
|---|---|---|
| `jira_issues` | `jira_issue` | every Jira source's primary container |
| `jira_sprints` | `jira_sprint` | every Jira source's `companion_containers.sprints` (or default) |
| `jira_fix_versions` | `jira_fix_version` | every Jira source's `companion_containers.fix_versions` |
| `jira_projects` | `jira_project` | every Jira source's `companion_containers.projects` |
| `confluence_pages` | `confluence_page` | every Confluence source's primary container |
| `confluence_spaces` | `confluence_space` | every Confluence source's `companion_containers.spaces` |

You only need to write `mcp.data_sources` explicitly when you want a non-default mapping — for example, exposing internal-only data on one MCP deployment and cloud-only on another.

## `state`

```yaml
state:
  backend: "cosmos"           # "cosmos" (default) or "local-file" (dev only)
  # local_path: ".quelch-state.json"  # only when backend=local-file
```

In production this is always `cosmos`. `quelch dev` automatically uses `local-file` regardless of what's written here.

## Environment variable substitution

Any string of the form `${VAR}` is substituted from the process environment at config-load time. Use it for anything secret:

```yaml
azure:
  subscription_id: "${AZURE_SUBSCRIPTION_ID}"

sources:
  - type: jira
    auth:
      pat: "${JIRA_INTERNAL_PAT}"
```

If a referenced env var is unset, `quelch validate` (and every other command that loads the config) fails fast with a clear error.

## Slicing per deployment

When you run `quelch azure deploy mcp-azure`, Quelch loads the full config, then synthesises the **effective config** for that one deployment. The effective config is what gets baked into the Container App as a secret/env var. It contains only:

- The Azure connection it needs (Cosmos endpoint, AI Search endpoint, OpenAI endpoint).
- The sources / containers it actually touches.
- Its own deployment block.
- Auth settings.

It does **not** contain other deployments, other sources' credentials, or the operator-level subscription id.

This means: if your MCP container is compromised, the attacker reads a config that exposes only the indexes the MCP was allowed to read in the first place. Ingest worker credentials, other sources, and the deploy-time subscription id are not in that container's environment.

You can preview the effective config:

```bash
quelch effective-config mcp-azure
```

## Worked example: small-scale single-host setup

```yaml
azure:
  subscription_id: "${AZURE_SUBSCRIPTION_ID}"
  resource_group: "rg-quelch-dev"
  region: "swedencentral"
  naming:
    prefix: "quelch"
    environment: "dev"

cosmos:
  database: "quelch"
  throughput:
    mode: "serverless"

search:
  sku: "basic"
  indexer:
    schedule:
      interval: "PT15M"

openai:
  endpoint: "https://${AOI_ACCOUNT}.openai.azure.com"
  embedding_deployment: "text-embedding-3-large"
  embedding_dimensions: 3072

sources:
  - type: jira
    name: jira-cloud
    url: "https://example.atlassian.net"
    auth:
      email: "${JIRA_EMAIL}"
      api_token: "${JIRA_TOKEN}"
    projects: ["DO"]
  - type: confluence
    name: confluence-cloud
    url: "https://example.atlassian.net/wiki"
    auth:
      email: "${JIRA_EMAIL}"
      api_token: "${JIRA_TOKEN}"
    spaces: ["ENG"]

deployments:
  - name: ingest
    role: ingest
    target: azure
    azure:
      container_app: { cpu: 0.5, memory: "1.0Gi" }
    sources:
      - source: jira-cloud
      - source: confluence-cloud
  - name: mcp
    role: mcp
    target: azure
    azure:
      container_app: { cpu: 1.0, memory: "2.0Gi", min_replicas: 0 }
    expose:
      - jira_issues
      - confluence_pages
      - jira_sprints
      - jira_fix_versions
    auth:
      mode: "api_key"
```

That config provisions a Cosmos DB, an AI Search service with one index per exposed data source (four in this example: `jira_issues`, `jira_sprints`, `jira_fix_versions`, `confluence_pages`), an Azure OpenAI account (assumed pre-existing), and two Container Apps — and it's enough to get `quelch agent generate` to produce working agent instructions.
