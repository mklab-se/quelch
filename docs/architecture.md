# Architecture

This document describes how Quelch is structured: the components, how data flows between them, the document model, the state model, the deployment topology, and which parts of today's codebase carry over.

## The big picture

```
                                                    ┌──────────────────────┐
   Source systems          ┌────────────────────┐   │  Azure AI Search     │
   ───────────────         │  Cosmos DB         │   │  ────────────────    │
   Jira / Confluence /     │  ───────────────   │   │  per-container       │
   future connectors       │  jira-issues-*     │   │  Indexer + skillset  │
        │                  │  confluence-*      │   │  (integrated         │
        │  quelch ingest   │  jira-sprints      │   │   vectorisation via  │
        │  (worker)   ────►│  jira-fix-versions │◄──┤   Azure OpenAI)      │
        │                  │  jira-projects     │   │  Search Index        │
        │                  │  confluence-spaces │   │     │                │
        │                  │  quelch-meta       │   │     ▼                │
        │                  └────────────────────┘   │  hybrid + vector     │
        │                            ▲              │  search index        │
        │                            │              └──────────┬───────────┘
        │                            │                         │
        │                            │  point-read             │  semantic /
        │                            │  Cosmos SQL             │  hybrid query
        │                            │                         │
        │                  ┌─────────┴─────────────────────────┴──┐
        │                  │  Quelch MCP server                   │
        └─────────────────►│  (Container App)                     │
                           │  Tools: search / query / get /       │
                           │         list_sources / aggregate     │
                           └──────────────────┬───────────────────┘
                                              │  MCP Streamable HTTP
                                              ▼
                                  ┌──────────────────────────┐
                                  │  Agent platforms         │
                                  │  Copilot Studio / VS Code│
                                  │  Claude Code / gh CLI    │
                                  └──────────────────────────┘
```

## Roles

Quelch is one binary with three runtime roles selected by subcommand. The same code, the same Cargo features, deployed differently.

### `quelch ingest` — the worker

A long-running process that pulls from a defined slice of sources and writes raw JSON documents to Cosmos DB. It does **not** compute embeddings — Azure AI Search owns vectorisation via integrated vectorisation skillsets.

What it does on each cycle:

1. Read its slice of the config (sources, target containers).
2. Read each source's cursor from the `quelch-meta` Cosmos container.
3. Pull changes since the cursor.
4. Write the documents to their target Cosmos containers.
5. Write the new cursor back to `quelch-meta`.

It runs anywhere: Azure Container Apps for cloud-side sources, on-prem (Docker, systemd, K8s) for sources behind a corporate firewall.

### `quelch mcp` — the agent-facing API

A long-running HTTP server speaking the MCP Streamable HTTP transport. It exposes five tools:

- `search` — hybrid semantic + keyword over Azure AI Search.
- `query` — exact, structured, aggregable queries over Cosmos DB.
- `get` — point-read a document by id from Cosmos DB.
- `list_sources` — discoverability: containers, schemas, common enum values.
- `aggregate` — count, sum, group_by over Cosmos DB.

It only exposes the indexes/containers explicitly listed in its `expose:` block — defence in depth. It's deployed on Azure Container Apps and authenticates calls via API key (v1) or Microsoft Entra ID (v1.x).

### `quelch` — the operator CLI

The human-facing CLI. It reads the full config, talks to Azure, and reconciles state. It is the only role that:

- Provisions Azure resources via Bicep.
- Triggers AI Search Indexer runs and resets.
- Tails logs from deployed workers.
- Runs ad-hoc queries against the data.
- Generates agent-side instructions.
- Generates on-prem deployment artefacts.

The CLI never runs in production — it is a developer/operator tool you run from your config repo.

### `quelch dev` — the local-dev shortcut

Runs the simulator, an in-memory mock for Cosmos and AI Search, an `ingest` worker, and an `mcp` server, all in one process. The TUI is the default UX. It exists so you can iterate on connectors, document shapes, and MCP tool behaviour without touching Azure.

## Data flow

### Ingest (write path)

```
Source API ──► quelch ingest ──► Cosmos DB ──► AI Search Indexer ──► AI Search Index
                                  (raw JSON,                              (vectorised,
                                  no embedding)        ▲                  semantic-config'd)
                                                       │
                                                Azure OpenAI
                                              (text-embedding-3 via
                                                skillset)
```

Quelch ingest writes documents as plain JSON to Cosmos DB. From the source's point of view, an issue or page is a normal record. The Cosmos DB change feed feeds the AI Search Indexer; the Indexer runs a skillset that calls Azure OpenAI to compute embeddings and then writes the augmented document to the search index.

This means Quelch ingest is dumb on purpose: no model dependencies, no embedding cost in the worker process, no need to coordinate model versions between worker fleets.

### Query (read path)

```
Agent ──MCP─► quelch mcp ─┬─► AI Search   (search: hybrid semantic+keyword)
                          ├─► Cosmos DB   (query / get / aggregate)
                          └─► AI Search   (list_sources read-side)
```

The MCP server picks a backend per tool:

- `search` → AI Search (hybrid semantic+keyword). Use when the agent has natural language and wants the index's ranking.
- `query` → Cosmos SQL. Use when the agent has exact filters and needs all matches.
- `get` → Cosmos point-read. Use when the agent has an id.
- `aggregate` → Cosmos SQL aggregations. Use for `COUNT`, `SUM`, `GROUP BY`.
- `list_sources` → static metadata + a sample of values from Cosmos / AI Search.

The MCP layer translates each tool call into the appropriate backend call, applies the deployment's `expose:` filter, paginates with cursors, and returns results with deep-link `source_link` fields so the agent can hand the user back to Jira/Confluence directly.

## Document model

### One container per source-type, overridable

Default Cosmos containers:

| Source type | Default container |
|---|---|
| Jira issues | `jira-issues` |
| Jira sprints | `jira-sprints` |
| Jira fix versions | `jira-fix-versions` |
| Jira projects | `jira-projects` |
| Confluence pages | `confluence-pages` |
| Confluence spaces | `confluence-spaces` |

Any source instance can override the target container in the config:

```yaml
sources:
  - type: jira
    name: jira-internal
    container: jira-issues-internal     # override
    ...
  - type: jira
    name: jira-cloud
    # no override → goes to default `jira-issues`
    ...
```

Each container gets its own AI Search Index (and Indexer + skillset). Quelch knows the topology because it owns the config.

### Companion containers for metadata

Jira and Confluence ingest don't just write the obvious entities (issues, pages). They also populate companion containers so agents can resolve domain concepts:

- `jira-sprints` — `{ id, name, state: active|future|closed, start_date, end_date, project_key, ... }`. Lets an agent answer "what is the next sprint in DO?" with a single `query`.
- `jira-fix-versions` — `{ id, name, project_key, released, release_date, ... }`. Lets an agent resolve "the last iXX firmware release".
- `jira-projects` — `{ key, name, lead, project_type, ... }`. Lets an agent discover available projects.
- `confluence-spaces` — `{ key, name, type, ... }`. Discoverability for spaces.

These are populated on the same ingest cycle as the primary entity. Updating a sprint state from `future` → `active` takes the next ingest cycle to surface.

### Document shape (illustrative)

A Jira issue document in `jira-issues` looks roughly like:

```json
{
  "id": "jira-internal-DO-1234",
  "source_name": "jira-internal",
  "source_link": "https://jira.internal.example/browse/DO-1234",
  "project_key": "DO",
  "key": "DO-1234",
  "type": "Story",
  "status": "In Progress",
  "summary": "Camera disconnects intermittently on WiFi",
  "description": "...",
  "assignee": { "id": "...", "name": "Kristofer Liljeblad", "email": "..." },
  "reporter": { ... },
  "story_points": 5,
  "sprint": { "id": "204", "name": "DO Sprint 42", "state": "active" },
  "fix_versions": [{ "id": "...", "name": "iXX-2.7.0" }],
  "components": ["camera", "firmware"],
  "labels": ["wifi", "regression"],
  "created": "2026-04-12T10:21:00Z",
  "updated": "2026-04-28T14:02:11Z",
  "comments": [...],
  "_partition_key": "DO"
}
```

`_partition_key` is set by the ingest worker (project key for Jira issues, space key for Confluence pages, etc.).

`source_link` is mandatory on every document — it's how agents hand the user back to the source system.

The exact field set is defined per source-type and documented in [configuration.md](configuration.md). `list_sources` exposes the schema to agents at runtime so they don't have to be hard-coded.

## State model

### Cursors live in `quelch-meta`

The shared `quelch-meta` container is the single source of truth for what each ingest worker has done. One document per `{deployment_name, source_name, subsource}` triple:

```json
{
  "id": "ingest-onprem-jira-ak::jira-internal::DO",
  "deployment_name": "ingest-onprem-jira-ak",
  "source_name": "jira-internal",
  "subsource": "DO",
  "cursor": { "last_updated": "2026-04-30T08:14:22Z" },
  "documents_synced": 12894,
  "last_sync_at": "2026-04-30T08:14:25Z",
  "last_error": null
}
```

This means:

- `quelch status` from your laptop reads a single Cosmos container and shows live state of every deployed worker.
- A redeployed worker reads its cursor on startup; no full re-sync.
- Multiple workers can share a Cosmos DB without stepping on each other (each owns its keys via `deployment_name`).

For `quelch dev` (no Cosmos), a local-file backend implements the same trait.

### Ownership boundaries

Distributed ingest workers are designed to be **disjoint by config**, not coordinated at runtime. If you want to split Jira projects across workers, you do so in the config (`projects: [A,B,...,K]` vs `projects: [L,...,Z]`). Quelch validates that each `(source, subsource)` pair is owned by exactly one deployment.

## Deployment topology

A single Quelch installation typically looks like:

```
┌─ Your config repo ─────────────────────────────────────┐
│  quelch.yaml                                           │
│  .quelch/azure/<deployment>.bicep   (generated)        │
└────────────┬───────────────────────────────────────────┘
             │ quelch azure deploy ...
             │
   ┌─────────┴───────────────────────────────────────────┐
   ▼                                                     │
┌─ Azure ─────────────────────────────────────────────┐  │
│                                                     │  │
│   Cosmos DB account                                 │  │
│   ├─ Database: quelch                               │  │
│   ├─ Containers: jira-issues, conf-pages, ...       │  │
│   └─ Container: quelch-meta                         │  │
│                                                     │  │
│   AI Search service                                 │  │
│   ├─ Indexer: jira-issues                           │  │
│   ├─ Indexer: confluence-pages                      │  │
│   └─ ...                                            │  │
│                                                     │  │
│   Azure OpenAI                                      │  │
│   └─ Embedding deployment (text-embedding-3)        │  │
│                                                     │  │
│   Container Apps environment                        │  │
│   ├─ ingest-cloud-jira  (image: ghcr.io/.../quelch) │  │
│   ├─ mcp-server         (image: ghcr.io/.../quelch) │  │
│   └─ ...                                            │  │
└─────────────────────────────────────────────────────┘  │
                                                         │ quelch generate-deployment ...
                                                         ▼
                                              ┌─ On-premises hosts ─────┐
                                              │  ingest-onprem-jira-ak  │
                                              │  ingest-onprem-jira-lz  │
                                              │  ingest-onprem-conf     │
                                              │  (docker-compose /      │
                                              │   systemd / K8s)        │
                                              └─────────────────────────┘
```

All ingest workers — cloud or on-prem — write to the same Cosmos DB. The MCP server is typically deployed in Azure so Copilot agents can reach it.

## What carries over from v1

Quelch v1 ships ~5,000 lines of Rust today. Most of it is reusable.

### Kept and extended

| Module | Role in v2 |
|---|---|
| `sources/jira.rs`, `sources/confluence.rs`, `sources/mod.rs` | The `SourceConnector` trait stays. We extend the connectors to populate companion containers (sprints, fix versions, spaces). |
| `config/` | Stays as the YAML loader; new sections (`cosmos`, `openai`, `deployments`, `mcp`, `state`) added. |
| `tui/` | Refocused: default UX of `quelch dev`, plus `quelch status --tui` becomes a fleet dashboard reading `quelch-meta`. |
| `sim/` | Drives `quelch dev` and CI. |
| `mock/` | Local Jira/Confluence HTTP server, used by `quelch dev` and integration tests. |
| `ai.rs` (ailloy integration) | Stays as a dependency for future AI features. Not used in v2 ingest path. |
| `copilot.rs` | Becomes the `copilot-studio` target of `quelch agent generate`. |

### Replaced or removed

| Module | Fate |
|---|---|
| `azure/mod.rs` (REST client to AI Search write API) | Gutted. AI Search no longer receives writes from Quelch. A small read-side client remains for the MCP `search` tool. |
| `sync/embedder.rs` | Removed. Embeddings happen in Azure AI Search via skillset. |
| `sync/mod.rs` engine | Replaced by a simpler `ingest/` engine: pull → write Cosmos → write cursor. |
| Commands `sync`, `watch`, `setup`, `reset-indexes`, `search`, `generate-agent` | Replaced by the new command tree (see [cli.md](cli.md)). |

### New modules

| Module | Purpose |
|---|---|
| `ingest/` | Replaces `sync/`. Source → Cosmos write loop. |
| `cosmos/` | Cosmos DB client (writes, point-reads, SQL queries, change-feed cursor metadata). |
| `mcp/` | The Streamable HTTP MCP server and the five tool implementations. |
| `azure/deploy/` | Bicep generator + `az` shell-out helpers + `what-if` parser. |
| `agent/` | Agent-bundle generators (Copilot Studio, VS Code MCP, Copilot CLI, Claude Code, generic Markdown). |
| `config/deployments.rs` | Slicing logic — turns the full config into a per-deployment effective config. |

## Cross-cutting concerns

- **Auth to Azure resources:** managed identity wherever possible (Container Apps → Cosmos / AI Search / OpenAI). API key fallbacks for local development. Keys are read from environment variables; the config never contains a literal secret.
- **Auth to source systems:** unchanged from v1 (PAT for Data Center, email + API token for Cloud).
- **Logging:** `tracing` + `tracing-subscriber`, JSON output in production, TUI-friendly fields. Per-document logs only at `debug!`.
- **Errors:** typed per module with `thiserror`, `anyhow` at CLI boundaries (unchanged from v1).
- **Versioning:** the Quelch CLI version pins the Container App image tag. `quelch 0.9.0 azure deploy` always deploys `ghcr.io/mklab-se/quelch:0.9.0`. No drift between operator and worker.
