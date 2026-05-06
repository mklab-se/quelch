# Configuration reference

Quelch is configured by a single `quelch.yaml` file you check into your repo. It is the **source of truth**: `quelch azure apply` reconciles Cosmos DB and Azure AI Search to it; `quelch instance config` slices it into per-instance configs you ship to your hosts.

This document describes every section of the file, how the per-instance slimming works, and the rules `quelch validate` enforces.

## Top-level shape

```yaml
azure:               { ... }   # Azure resources Quelch will configure
source_connections:  [ ... ]   # named source connections (one per (base_url × credential) tuple)
instances:           [ ... ]   # named runtime instances (Q-Ingest workers + Q-MCP servers)
```

That's it. There's no `deployments[]`, no `cosmos:` or `search:` at the top level, no `naming:` block, no `rigg.ownership`, no `state:` block.

A complete worked example lives at the end of this document.

---

## `azure`

Three sub-blocks describe pre-existing Azure resources Quelch configures (it does not provision them at the account level).

```yaml
azure:
  cosmos:
    # Control-plane fields — used by `quelch azure apply` to PUT containers.
    # Stripped from per-instance Q-Ingest / Q-MCP configs.
    subscription_id: ${AZURE_SUBSCRIPTION_ID}
    resource_group:  rg-quelch-prod
    account:         my-cosmos
    # Data-plane fields — used by every role.
    endpoint:        https://my-cosmos.documents.azure.com
    database:        quelch
    # Container layout — overridable per source-connection.
    containers:
      jira_issues:        jira-issues
      jira_sprints:       jira-sprints
      jira_fix_versions:  jira-fix-versions
      jira_projects:      jira-projects
      confluence_pages:   confluence-pages
      confluence_spaces:  confluence-spaces
    meta_container:       quelch-meta

  search:
    # Endpoint only — rigg-as-library uses the AI Search admin REST API
    # via DefaultAzureCredential. No ARM-level fields.
    endpoint: https://my-search.search.windows.net

  ai:
    # Wired into the AI Search Knowledge Base at apply-time
    # (vectoriser config + KB chat model). Q-MCP does not call AI directly.
    provider: foundry                      # foundry | azure_openai
    endpoint: https://my-foundry.cognitiveservices.azure.com
    embedding:
      deployment: text-embedding-3-large
      dimensions: 3072
    chat:
      deployment: gpt-5-mini
      model_name: gpt-5-mini
```

### `azure.cosmos`

| Field | Meaning |
|---|---|
| `subscription_id` | Azure subscription containing the Cosmos account. Used by `azure apply` to construct the ARM REST URL. Typically `${AZURE_SUBSCRIPTION_ID}`. |
| `resource_group` | Resource group containing the Cosmos account. |
| `account` | Cosmos DB account name (the user-supplied name; not the FQDN). |
| `endpoint` | The Cosmos DB account's documents endpoint (`https://<account>.documents.azure.com`). Used by Q-Ingest and Q-MCP at the data plane. |
| `database` | Database name inside the account. Quelch will PUT the database during `azure apply` if missing. |
| `containers` | Map of canonical container kinds (`jira_issues`, `jira_sprints`, ...) to physical container names. The names you put here are what Quelch creates and what AI Search indexes. |
| `meta_container` | Name of the container that holds cursor state (`quelch-meta`). Always required. |

The first three fields are **control-plane only** — `azure apply` uses them to call ARM REST. They're stripped from per-instance configs because Q-Ingest and Q-MCP don't need to PUT containers.

### `azure.search`

| Field | Meaning |
|---|---|
| `endpoint` | The AI Search service's admin endpoint (`https://<service>.search.windows.net`). Used by `azure apply` (via the embedded rigg library) to manage indexes / indexers / KBs, and by Q-MCP at runtime to issue `search` calls. |

No `subscription_id`, `resource_group`, `service`, or `sku` — rigg-as-library only needs the endpoint URL, and Quelch never creates the service shell.

### `azure.ai`

| Field | Meaning |
|---|---|
| `provider` | `foundry` (recommended, newer surface) or `azure_openai`. Affects no apply-time behaviour beyond the endpoint shape Quelch expects. |
| `endpoint` | The AI provider's REST endpoint. Foundry: `https://<project>.cognitiveservices.azure.com`. Azure OpenAI: `https://<account>.openai.azure.com` (the `endpoint` shape from `az cognitiveservices account show`). |
| `embedding.deployment` | Name of the deployed embedding model. Used by the AI Search vectoriser. **Recommended:** `text-embedding-3-large`. |
| `embedding.dimensions` | Embedding output dimensions. Must match the deployment. `3072` for `text-embedding-3-large`. |
| `chat.deployment` | Name of the deployed chat model. Used by the AI Search Knowledge Base for query planning + answer synthesis. |
| `chat.model_name` | The underlying model name (e.g. `gpt-5-mini`). Sometimes the same as `deployment`, sometimes not. |

Supported chat models for the Knowledge Base (per AI Search 2025-11-01-preview): `gpt-4o`, `gpt-4o-mini`, `gpt-4.1`, `gpt-4.1-nano`, `gpt-4.1-mini`, `gpt-5`, `gpt-5-nano`, `gpt-5-mini`. **Recommended: `gpt-5-mini`.**

The `azure.ai` block is **stripped from per-instance configs** for both Q-Ingest and Q-MCP — neither role calls the AI provider at runtime; the AI is wired into the AI Search KB at apply-time and stays there.

---

## `source_connections`

A list of named source connections. Each entry is one `(base_url × credential)` tuple. Multiple ingest instances can share a connection; multiple connections can share a base_url with different credentials.

### Jira (PAT — Data Center / Server)

```yaml
source_connections:
  - name: jira-internal-pat-x
    type: jira
    base_url: https://jira.internal/
    auth: { kind: pat, token: ${JIRA_PAT_X} }
    projects: [DO, ANNA, SARA]
```

### Jira (basic — Atlassian Cloud)

```yaml
  - name: jira-cloud
    type: jira
    base_url: https://example.atlassian.net/
    auth:
      kind: basic
      email: ${JIRA_EMAIL}
      api_token: ${JIRA_API_TOKEN}
    projects: [PUBLIC, INT]
```

### Confluence (PAT — Data Center / Server)

```yaml
  - name: confluence-internal
    type: confluence
    base_url: https://confluence.internal/
    auth: { kind: pat, token: ${CONFLUENCE_PAT} }
    spaces: [ENG, DOCS]
```

### Common fields

| Field | Meaning |
|---|---|
| `name` | Unique connection name. Referenced from `instances[].connections`. |
| `type` | `jira` or `confluence`. |
| `base_url` | Source-system base URL (with trailing slash). |
| `auth.kind` | `pat` (Personal Access Token; Data Center) or `basic` (Cloud — email + API token). |
| `auth.token` | PAT value, env-var reference. (`pat` kind only.) |
| `auth.email`, `auth.api_token` | Cloud credentials, env-var references. (`basic` kind only.) |
| `projects` | (Jira) List of project keys to ingest. Each becomes a `(source, subsource)` cursor. |
| `spaces` | (Confluence) List of space keys to ingest. Each becomes a `(source, subsource)` cursor. |
| `container` | Optional override of the default Cosmos container for this connection's primary entities. Defaults from `azure.cosmos.containers`. |
| `companion_containers` | Optional per-companion overrides (e.g. `sprints: jira-sprints-cloud`). Defaults from `azure.cosmos.containers`. |
| `fields` | Optional Jira custom-field map (`story_points: customfield_10016`, etc.). |

`source_connections` is **stripped from Q-MCP per-instance configs** — Q-MCP doesn't pull from sources.

---

## `instances`

A list of named runtime instances. Each entry corresponds to one process the user will run somewhere.

```yaml
instances:
  - name: ingest-jira-internal
    kind: ingest
    connections: [jira-internal-pat-x]
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

### Common fields

| Field | Meaning |
|---|---|
| `name` | Unique instance name. Used as `owner_instance` on cursors and as the argument to `quelch instance config NAME`. |
| `kind` | `ingest` or `mcp`. Determines which fields below apply. |

### Ingest-only fields

| Field | Meaning |
|---|---|
| `connections` | List of `source_connections[].name` this instance will pull from. |
| `cycle_interval` | How often the worker tries to advance its window. Duration string (`5m`, `30s`). Default: `5m`. |

### MCP-only fields

| Field | Meaning |
|---|---|
| `expose` | List of canonical data-source names (`jira_issues`, `confluence_pages`, ...) the agent can see. Anything not listed is invisible — defence in depth. |
| `api_key` | Bearer token Q-MCP will accept. **Always an env-var reference**, never a literal value. Typically `${QUELCH_MCP_API_KEY}`. See [api-key.md](api-key.md). |
| `knowledge_base` | Name of the AI Search Knowledge Base this MCP queries via the `search` tool. Created by `azure apply`. |
| `listen` | Address Q-MCP binds to. Default: `0.0.0.0:8080`. |

---

## Per-instance config schema

`quelch instance config <name> --kind ingest|mcp [--output PATH]` emits a per-instance config — same parser, narrower content.

### Q-Ingest per-instance config

Contains:

- `azure.cosmos` minus the control-plane fields (`subscription_id`, `resource_group`, `account`).
- `azure.cosmos.containers` reduced to only those this instance writes to.
- `source_connections` reduced to only those this instance uses.
- The single `instances[]` entry for this instance.

Does NOT contain:

- `azure.search` (Q-Ingest never reads Search).
- `azure.ai` (AI is wired into the KB at apply time, not used by Q-Ingest).
- Other instances or other connections.

```yaml
azure:
  cosmos:
    endpoint: https://my-cosmos.documents.azure.com
    database: quelch
    containers:
      jira_issues: jira-issues
      jira_sprints: jira-sprints
      jira_fix_versions: jira-fix-versions
      jira_projects: jira-projects
    meta_container: quelch-meta

source_connections:
  - name: jira-internal-pat-x
    type: jira
    base_url: https://jira.internal/
    auth: { kind: pat, token: ${JIRA_PAT_X} }
    projects: [DO, ANNA, SARA]

instances:
  - name: ingest-jira-internal
    kind: ingest
    connections: [jira-internal-pat-x]
    cycle_interval: 5m
```

### Q-MCP per-instance config

Contains:

- `azure.cosmos` minus the control-plane fields.
- `azure.cosmos.containers` reduced to only those this instance exposes.
- `azure.search`.
- The single `instances[]` entry.

Does NOT contain:

- Source connections (Q-MCP doesn't pull from sources).
- `azure.cosmos.{subscription_id, resource_group, account}`.
- `azure.ai`.

```yaml
azure:
  cosmos:
    endpoint: https://my-cosmos.documents.azure.com
    database: quelch
    containers:
      jira_issues: jira-issues
      jira_sprints: jira-sprints
      confluence_pages: confluence-pages
    meta_container: quelch-meta
  search:
    endpoint: https://my-search.search.windows.net

instances:
  - name: mcp-prod
    kind: mcp
    expose: [jira_issues, jira_sprints, confluence_pages]
    api_key: ${QUELCH_MCP_API_KEY}
    knowledge_base: quelch-prod-kb
    listen: 0.0.0.0:8080
```

### Slimming rules summary

| Field | Master | Q-Ingest config | Q-MCP config |
|---|---|---|---|
| `azure.cosmos.{subscription_id, resource_group, account}` | yes | no | no |
| `azure.cosmos.{endpoint, database, meta_container}` | yes | yes | yes |
| `azure.cosmos.containers` (full map) | yes | only used by this instance | only exposed by this instance |
| `azure.search` | yes | no | yes |
| `azure.ai` | yes | no | no |
| `source_connections` | yes | only those this instance uses | no |
| `instances` | yes (all) | one (this instance only) | one (this instance only) |

---

## Auto-detect rule for `--instance`

`quelch ingest --config FILE` and `quelch mcp --config FILE` accept an optional `--instance NAME`. When omitted:

- If the file declares exactly one instance whose `kind` matches the binary, use it.
- Otherwise fail with a clear message listing the candidate instance names.

This means a per-instance file produced by `quelch instance config` runs with no flag (it contains exactly one instance). The master file works flag-less too, if it happens to declare only one instance of the matching kind.

---

## Auth model

Credentials are referenced as env-var placeholders only (`${VAR}`). The runtime resolves them at config-load time. Missing env vars produce a precise error from `quelch validate --config <file>`.

The user wires their host's secret store however they want:

- Docker `.env` / `--env-file`.
- Kubernetes `Secret` + `envFrom`.
- systemd `EnvironmentFile=`.
- Azure Container Apps secrets.
- Azure Key Vault references via the host's native AKV integration (not Quelch's).

For Azure-side authentication (Cosmos data plane, AI Search), every Quelch role uses `DefaultAzureCredential` — the operator's `az login` chain in the CLI; managed identity / workload identity / `AZURE_CLIENT_*` env vars / `AZURE_COSMOS_KEY` in Q-Ingest and Q-MCP. Quelch does not manage Azure-side identities.

For Q-MCP API-key auth (agent ↔ Q-MCP), see [api-key.md](api-key.md).

---

## Validation rules

`quelch validate` runs:

- **Env-var resolution.** Every `${VAR}` reference must resolve at validate time.
- **Reference integrity.** Every `instances[].connections[]` entry references a `source_connections[].name`. Every `instances[].expose[]` entry is a canonical data-source name backed by a Cosmos container in `azure.cosmos.containers`.
- **Static conflict prevention.** For ingest instances, build a claim set of `(source_type, base_url, subsource)` tuples per instance. Any tuple claimed by ≥2 instances fails validation with a clear error naming both instances and the conflicting tuples.

`validate` also runs as the first step of `azure plan` and `azure apply`. Exit code is non-zero on any failure; safe for CI.

---

## Cursor ownership

Static conflict prevention catches the obvious case at YAML edit time. Runtime ownership catches the rest:

Every cursor doc in `quelch-meta` carries an `owner_instance` field. Q-Ingest writes its own instance name on first claim. On every subsequent cursor write, if the doc's owner ≠ this instance's name, Q-Ingest refuses with a hard error and exits.

To deliberately transfer a cursor between instances:

```bash
quelch reset --instance NEW_OWNER --source NAME --subsource KEY --take-ownership
```

Without `--take-ownership`, `reset` operates only on cursors already owned by the named instance.

---

## Worked example: minimal master config

```yaml
azure:
  cosmos:
    subscription_id: ${AZURE_SUBSCRIPTION_ID}
    resource_group: rg-quelch-prod
    account: quelch-prod-cosmos
    endpoint: https://quelch-prod-cosmos.documents.azure.com
    database: quelch
    containers:
      jira_issues:        jira-issues
      jira_sprints:       jira-sprints
      jira_fix_versions:  jira-fix-versions
      jira_projects:      jira-projects
      confluence_pages:   confluence-pages
      confluence_spaces:  confluence-spaces
    meta_container:       quelch-meta
  search:
    endpoint: https://quelch-prod-search.search.windows.net
  ai:
    provider: foundry
    endpoint: https://quelch-prod-foundry.cognitiveservices.azure.com
    embedding:
      deployment: text-embedding-3-large
      dimensions: 3072
    chat:
      deployment: gpt-5-mini
      model_name: gpt-5-mini

source_connections:
  - name: jira-cloud
    type: jira
    base_url: https://example.atlassian.net/
    auth:
      kind: basic
      email: ${JIRA_EMAIL}
      api_token: ${JIRA_API_TOKEN}
    projects: [DO, INT]

  - name: confluence-cloud
    type: confluence
    base_url: https://example.atlassian.net/wiki/
    auth:
      kind: basic
      email: ${JIRA_EMAIL}
      api_token: ${JIRA_API_TOKEN}
    spaces: [ENG, DOCS]

instances:
  - name: ingest-cloud
    kind: ingest
    connections: [jira-cloud, confluence-cloud]
    cycle_interval: 5m

  - name: mcp-prod
    kind: mcp
    expose: [jira_issues, jira_sprints, jira_fix_versions, jira_projects,
             confluence_pages, confluence_spaces]
    api_key: ${QUELCH_MCP_API_KEY}
    knowledge_base: quelch-prod-kb
    listen: 0.0.0.0:8080
```

That config runs Cosmos + AI Search configuration via `quelch azure apply`, then emits two per-instance configs — `q-ingest-cloud.yaml` and `q-mcp-prod.yaml` — that the user copies to their hosts and runs with `quelch ingest --config q-ingest-cloud.yaml` and `quelch mcp --config q-mcp-prod.yaml`.
