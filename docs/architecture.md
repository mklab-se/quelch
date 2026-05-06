# Architecture

This document describes how Quelch is structured: the components, how data flows between them, the document model, the state model, the topology, and the module map.

Throughout this doc:

- **Quelch MCP** (Q-MCP) — the MCP server agents talk to. The user hosts it wherever they like (Docker, systemd, k8s, Container Apps, a bare VM); Quelch generates the per-instance config, not the hosting artefacts.
- **Quelch Ingest** (Q-Ingest) — the per-source worker that pulls into Cosmos DB. Same hosting story as Q-MCP. Typically runs close to its data source — often on-prem next to Confluence / Jira Data Center, since those installs usually aren't reachable from Azure.

## The big picture

```
                                                    ┌──────────────────────────┐
                                                    │  Azure AI Search         │
                            ┌────────────────────┐  │  ──────────────────────  │
   Source systems           │  Cosmos DB         │  │  Indexer + skillset      │
   ───────────────          │  ───────────────   │  │  (integrated             │
   Jira / Confluence /      │  jira-issues-*     │  │   vectorisation via      │
   future connectors        │  confluence-*      │  │   Azure OpenAI)          │
        │                   │  jira-sprints      │  │           │              │
        │  quelch ingest    │  jira-fix-versions │◄─┤           ▼              │
        │  (worker)   ─────►│  jira-projects     │  │  Search Index (vector +  │
        │                   │  confluence-spaces │  │           │  full-text)  │
        │                   │  quelch-meta       │  │           ▼              │
        │                   └────────────────────┘  │  Knowledge Base          │
        │                            ▲              │  (Agentic Retrieval)     │
        │                            │              └──────────────┬───────────┘
        │                            │                              │
        │                            │  query · get                 │  search
        │                            │  aggregate                   │  (semantic +
        │                            │  (Cosmos SQL)                │   reranked +
        │                            │                              │   optional answer)
        │                            │                              │
        │                   ┌────────┴──────────────────────────────┴┐
        │                   │  Q-MCP                                 │
        └──────────────────►│  (hosted by you — anywhere)            │
                            │  5 tools, per-tool routing:            │
                            │   • search        → Knowledge Base     │
                            │   • query/get/agg → Cosmos DB          │
                            │   • list_sources  → cached SchemaCatalog │
                            └──────────────────┬─────────────────────┘
                                               │  MCP Streamable HTTP
                                               ▼
                                   ┌──────────────────────────┐
                                   │  Agent platforms         │
                                   │  Copilot Studio / VS Code│
                                   │  Claude Code / Codex /   │
                                   │  gh Copilot CLI          │
                                   └──────────────────────────┘
```

## Roles

Quelch is one binary with three runtime roles selected by subcommand. The same code, the same Cargo features, run differently.

### `quelch ingest` — Quelch Ingest (Q-Ingest)

A long-running process that pulls from a defined slice of sources and writes raw JSON documents to Cosmos DB. It does **not** compute embeddings — Azure AI Search owns vectorisation via integrated vectorisation skillsets.

What it does on each cycle:

1. Read its instance's slice of the config (source connections, target containers).
2. Read each source's cursor from the `quelch-meta` Cosmos container.
3. Pull changes since the cursor.
4. Write the documents to their target Cosmos containers.
5. Write the new cursor back to `quelch-meta` (with `owner_instance` set to this instance's name; refuses to write a cursor owned by another instance).

The user hosts Q-Ingest anywhere — Docker, systemd, Kubernetes, Azure Container Apps, a bare VM — typically close to its data source. Quelch generates only the per-instance config (`quelch instance config <name> --kind ingest`); the hosting shape is up to the user. See [hosting.md](hosting.md).

### `quelch mcp` — Quelch MCP (Q-MCP)

A long-running HTTP server speaking the MCP Streamable HTTP transport. It exposes five tools:

- `search` — hybrid semantic + keyword over Azure AI Search.
- `query` — exact, structured, aggregable queries over Cosmos DB.
- `get` — point-read a document by id from Cosmos DB.
- `list_sources` — discoverability: containers, schemas, common enum values.
- `aggregate` — count, sum, group_by over Cosmos DB.

It only exposes the data sources explicitly listed in its instance's `expose:` block — defence in depth. It authenticates calls via API key (current) or Microsoft Entra ID (planned). Hosting story is identical to Q-Ingest.

### `quelch` — the operator CLI

The human-facing CLI. It reads the full config, talks to Azure, and reconciles state. It is the only role that:

- Configures Cosmos DB containers via ARM REST.
- Configures Azure AI Search (indexes, indexers, skillsets, knowledge sources, knowledge bases) via the embedded rigg library, in-memory.
- Triggers AI Search Indexer runs and resets.
- Runs ad-hoc queries against the data.
- Generates agent-side instructions.
- Emits the per-instance config files the user copies to their hosts.

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
Agent ──MCP─► quelch mcp ─┬─► AI Search Knowledge Base   (search — Agentic Retrieval)
                          ├─► Cosmos DB                  (query / get / aggregate)
                          └─► (cached SchemaCatalog)     (list_sources)
```

The MCP server picks a backend per tool:

- `search` → AI Search **Knowledge Base** (Agentic Retrieval — question decomposition, hybrid semantic+keyword, reranking, optional answer synthesis). Use when the agent has natural language and wants the best answer. With `mcp.search.disable_agentic: true` the call falls back to the underlying index for raw hybrid search.
- `query` → Cosmos SQL. Use when the agent has exact filters and needs all matches.
- `get` → Cosmos cross-partition point-read. Use when the agent has an id.
- `aggregate` → Cosmos SQL aggregations. Use for `COUNT`, `SUM`, `GROUP BY`.
- `list_sources` → cached `SchemaCatalog` built at MCP server startup. Doesn't hit any backend at request time.

The MCP layer translates each tool call into the appropriate backend call(s), resolves logical data-source names to physical containers/indexes (see [Two layers of names](#two-layers-of-names)), applies the deployment's exposure rules, paginates with cursors, and returns results with deep-link `source_link` fields so the agent can hand the user back to Jira/Confluence directly.

## Two layers of names

There are two naming layers in Quelch and they should never meet on the wrong side of the boundary.

| Layer | Examples | Who sees it |
|---|---|---|
| **Storage** — physical Cosmos containers and AI Search indexes | `jira-issues-internal`, `jira-issues-cloud`, `quelch-meta` | Operator (you), `quelch.yaml`, ARM REST calls Quelch makes during `azure apply` |
| **API** — logical data sources | `jira_issues`, `jira_sprints`, `confluence_pages` | Agents, MCP tool calls, generated bundles |

The MCP server is the boundary. Inside Quelch, the server holds a static map: each logical data source resolves to one or more physical containers (and matching AI Search indexes). When an agent calls `query(data_source: "jira_issues", ...)`, the MCP layer:

1. Resolves `jira_issues` to its set of underlying Cosmos containers.
2. Fans out the query to each.
3. Merges results, paginates with a unified cursor, returns to the agent.

The agent never sees container names, never sees index names, never knows whether one logical data source is backed by one physical container or twenty. This abstraction is the entire point of having an MCP layer.

When you (the operator) work with `quelch.yaml`, you work in the storage layer — you spell containers and indexes by their physical names, because that's what Cosmos and AI Search understand. When agents work with the live API, they only ever spell data sources. The mapping between the two is configured per MCP instance via the `expose:` field; see [configuration.md](configuration.md).

This document and [configuration.md](configuration.md) talk about both layers, because they're how you set the system up. [mcp-api.md](mcp-api.md), [agent-generation.md](agent-generation.md), and [examples.md](examples.md) talk only about the API layer, because that's all an agent ever sees.

## Document model

### Storage layout: one container per source-type, overridable

Default Cosmos containers:

| Source type | Default container |
|---|---|
| Jira issues | `jira-issues` |
| Jira sprints | `jira-sprints` |
| Jira fix versions | `jira-fix-versions` |
| Jira projects | `jira-projects` |
| Confluence pages | `confluence-pages` |
| Confluence spaces | `confluence-spaces` |

Any source connection can override the target container in the config:

```yaml
azure:
  cosmos:
    containers:
      jira_issues: jira-issues          # global default

source_connections:
  - name: jira-internal-pat-x
    type: jira
    base_url: https://jira.internal/
    auth: { kind: pat, token: ${JIRA_PAT_X} }
    projects: [DO, ANNA]
    container: jira-issues-internal     # per-connection override
  - name: jira-cloud
    type: jira
    base_url: https://example.atlassian.net/
    auth: { kind: basic, email: ${JIRA_EMAIL}, token: ${JIRA_TOKEN} }
    projects: [PUBLIC]
    # no override → goes to default `jira-issues`
```

Each container gets its own AI Search Index (and Indexer + skillset). Quelch knows the topology because it owns the config.

### Companion containers for metadata

Jira and Confluence ingest don't just write the obvious entities (issues, pages). They also populate companion containers so agents can resolve domain concepts via the API layer:

- `jira-sprints` — `{ id, name, state: active|future|closed, start_date, end_date, project_key, ... }`. Surfaced as the `jira_sprints` data source. Lets an agent answer "what is the next sprint in DO?" with a single `query`.
- `jira-fix-versions` — `{ id, name, project_key, released, release_date, ... }`. Surfaced as `jira_fix_versions`. Lets an agent resolve "the last iXX firmware release".
- `jira-projects` — `{ key, name, lead, project_type, ... }`. Surfaced as `jira_projects`. Lets an agent discover available projects.
- `confluence-spaces` — `{ key, name, type, ... }`. Surfaced as `confluence_spaces`. Discoverability for spaces.

These are populated on the same ingest cycle as the primary entity. Updating a sprint state from `future` → `active` takes the next ingest cycle to surface.

### Canonical field reference

These are the fields the ingest worker writes for each source type by default. Every field is read by the AI Search Indexer (and surfaced through `list_sources`) unless marked otherwise. Custom fields beyond these are opt-in via `sources[].fields` in the config.

#### Jira issue (`jira_issues`)

```json
{
  "id": "jira-internal-DO-1234",                          // composite: source_name + key
  "source_name": "jira-internal",
  "source_link": "https://jira.internal.example/browse/DO-1234",

  "key": "DO-1234",
  "project_key": "DO",

  "type": "Story",                                         // Story|Task|Bug|Epic|Sub-task
  "status": "In Progress",
  "status_category": "In Progress",                        // To Do | In Progress | Done
  "priority": "High",                                      // Highest|High|Medium|Low|Lowest
  "resolution": null,                                      // null while open; e.g. "Done", "Duplicate"
  "resolved": null,                                        // datetime; null while open

  "summary": "Camera disconnects intermittently on WiFi",
  "description": "<rendered HTML or markdown>",

  "assignee":  { "id": "...", "name": "Kristofer Liljeblad", "email": "..." },
  "reporter":  { "id": "...", "name": "...",                "email": "..." },

  "created":  "2026-04-12T10:21:00Z",
  "updated":  "2026-04-28T14:02:11Z",
  "due_date": "2026-05-15",

  "labels":     ["wifi", "regression"],
  "components": ["camera", "firmware"],

  "fix_versions":     [{ "id": "...", "name": "iXX-2.7.0" }],
  "affects_versions": [{ "id": "...", "name": "iXX-2.6.3" }],

  "sprint": { "id": "204", "name": "DO Sprint 42",
              "state": "active",
              "start_date": "...", "end_date": "...", "goal": "..." },

  "parent":     { "id": "...", "key": "DO-1100", "type": "Epic" },   // Sub-task or Epic-child
  "epic_link":  "DO-1100",                                            // legacy custom field; redundant with parent on new Jira

  "issuelinks": [
    { "type": "blocks",     "direction": "outward", "target_key": "DO-1180", "target_summary": "..." },
    { "type": "is blocked by", "direction": "inward", "target_key": "DO-1170", "target_summary": "..." }
  ],

  "comments": [
    { "id": "...", "author": { ... }, "body": "...", "created": "...", "updated": "..." }
  ],

  // configurable (sources[].fields)
  "story_points": 5,

  // Quelch internals
  "_partition_key": "DO",                                  // = project_key
  "_deleted":       false,                                 // set true by reconciliation
  "_deleted_at":    null
}
```

#### Jira sprint (`jira_sprints`)

```json
{
  "id":           "jira-internal-sprint-204",
  "source_name":  "jira-internal",
  "source_link":  "https://jira.internal.example/.../sprints/204",
  "key":          "204",
  "name":         "DO Sprint 42",
  "state":        "active",                  // active | future | closed
  "start_date":   "2026-04-15T00:00:00Z",
  "end_date":     "2026-04-29T00:00:00Z",
  "complete_date": null,                      // datetime once state=closed
  "goal":         "Stabilise iXX firmware connectivity",
  "project_keys": ["DO"],                     // sprints are board-level; usually 1 project
  "board_id":     "12",
  "created":      "...", "updated": "...",
  "_partition_key": "DO",
  "_deleted": false, "_deleted_at": null
}
```

#### Jira fix version (`jira_fix_versions`)

```json
{
  "id":           "jira-internal-fixversion-iXX-2.7.0",
  "source_name":  "jira-internal",
  "source_link":  "https://jira.internal.example/.../versions/...",
  "name":         "iXX-2.7.0",
  "description":  "Quarterly camera firmware release",
  "released":     true,
  "release_date": "2026-04-09",
  "archived":     false,
  "project_key":  "DO",
  "created":      "...", "updated": "...",
  "_partition_key": "DO",
  "_deleted": false, "_deleted_at": null
}
```

#### Jira project (`jira_projects`)

```json
{
  "id":          "jira-internal-DO",
  "source_name": "jira-internal",
  "source_link": "https://jira.internal.example/projects/DO",
  "key":         "DO",
  "name":        "DataOps",
  "description": "...",
  "lead":        { "id": "...", "name": "...", "email": "..." },
  "project_type_key": "software",             // software | business | service_desk
  "category":    { "id": "...", "name": "Engineering" },
  "created":     "...", "updated": "...",
  "_partition_key": "DO",
  "_deleted": false, "_deleted_at": null
}
```

#### Confluence page (`confluence_pages`)

```json
{
  "id":          "confluence-internal-ENG-12345",
  "source_name": "confluence-internal",
  "source_link": "https://confluence.internal.example/display/ENG/Camera+Connectivity",

  "space_key":   "ENG",
  "page_id":     "12345",
  "title":       "Camera Connectivity Pipeline",
  "body":        "<rendered storage or view format>",

  "version":     { "number": 7, "when": "...", "by": { "id":"...", "name":"...", "email":"..." } },
  "ancestors":   [ { "id": "...", "title": "Architecture" } ],   // breadcrumbs to root

  "created":     "2026-01-12T10:00:00Z",
  "created_by":  { "id": "...", "name": "...", "email": "..." },
  "updated":     "2026-04-28T14:02:11Z",
  "updated_by":  { "id": "...", "name": "...", "email": "..." },

  "labels":      ["camera", "architecture"],

  "_partition_key": "ENG",
  "_deleted": false, "_deleted_at": null
}
```

#### Confluence space (`confluence_spaces`)

```json
{
  "id":          "confluence-internal-space-ENG",
  "source_name": "confluence-internal",
  "source_link": "https://confluence.internal.example/display/ENG/",
  "key":         "ENG",
  "name":        "Engineering",
  "description": "...",
  "type":        "global",                    // global | personal | team
  "homepage_id": "10001",
  "created":     "...", "updated": "...",
  "_partition_key": "ENG",
  "_deleted": false, "_deleted_at": null
}
```

#### Common conventions

- `id` is always `{source_name}-{stable-key}` — globally unique across Quelch. For Jira, `stable-key` is the issue key (`DO-1234`), sprint number (`sprint-204`), version name (`fixversion-iXX-2.7.0`), or project key. For Confluence, `stable-key` is `{space_key}-{page_id}` so a page moving between spaces becomes a new record (Cosmos partition keys are immutable; see [sync.md "Confluence-specific deletion cases"](sync.md#confluence-specific-deletion-cases)).
- `source_name` is the configured source instance (e.g. `jira-internal`, `jira-cloud`). Lets agents filter to "only stuff from cloud Jira" without per-instance MCP calls.
- `source_link` is mandatory on every document — agents include it in user-facing answers so users can click through.
- `_partition_key` is set by the ingest worker — project key for Jira, space key for Confluence. It's how Cosmos partitions the container.
- `_deleted` / `_deleted_at` participate in the soft-delete column policy of the AI Search Indexer (see [sync.md](sync.md#deletions)).
- All datetimes are UTC ISO-8601.

The MCP `list_sources` tool surfaces this schema (in API-layer terms) at runtime so agents don't need to be hard-coded against it. Custom fields configured per source appear in `list_sources` automatically.

## Lifecycle of config changes

What happens when you edit `quelch.yaml` and run `quelch azure apply` again?

| Change | Effect |
|---|---|
| Add a new `projects: [..., NEW]` to a Jira connection | The new project becomes a new `(source, subsource)` tuple. On its first ingest cycle, `last_complete_minute` is unset so the worker runs an initial backfill of NEW. Existing projects continue normally. Re-emit the per-instance config and restart the host. |
| Remove a project from `projects:` | The worker stops syncing that project — it's no longer in its config. The Cosmos data is left in place; reconciliation will not delete it (reconciliation only marks docs missing *from the source*, not docs whose subsource was removed from config). To purge: `quelch azure indexer reset <indexer>` to drop from search, then drop manually from Cosmos if desired. |
| Add a new source connection (`jira-cloud` next to `jira-internal`) | `quelch azure plan` shows a new container (if a new container shape is implied), indexer, knowledge source, etc. Apply, and the new connection backfills from scratch on its first cycle when an instance starts using it. |
| Add a custom field via `source_connections[].fields.foo: customfield_X` | Backfill is *not* automatically re-run. New issues / updated issues will include the new field; old ones won't until they're updated in the source. To force a full re-ingest of the field: `quelch reset --instance <name> --source ... --subsource ...` to wipe cursors and restart backfill. |
| Add a data source to an MCP instance's `expose:` | `azure apply` adds the corresponding knowledge-source / knowledge-base entry. Agent's `list_sources` includes it next call. Re-emit the per-instance Q-MCP config and restart the host. |
| Remove a data source from `expose:` | `azure apply` removes the corresponding knowledge-source / KB entry. The Cosmos container is *not* dropped (data is preserved). |
| Add or move an ingest instance | `quelch validate` checks that no two ingest instances claim overlapping `(source_type, base_url, subsource)` tuples. The first cursor write from the new instance claims `owner_instance` in `quelch-meta`; the old instance is yours to decommission (Quelch doesn't reach into your hosts to stop processes). For deliberate transfer: `quelch reset --instance NEW --take-ownership`. |
| Bump `safety_lag_minutes` | Live-safe; cursor never moves backward. See [sync.md](sync.md#trade-offs). |
| Change `azure.cosmos.containers.jira_issues` (rename) | This is a **destructive** change — `azure plan` will show a `+` new container, `-` old container, all data lost on rename. Avoid renaming containers; use per-connection `container:` overrides for new sources instead. |

## State model

### Cursors live in `quelch-meta`

The shared `quelch-meta` container is the single source of truth for what each ingest worker has done. One document per `{owner_instance, source_name, subsource}` triple. The full schema is in [sync.md](sync.md#state-stored-per-source-subsource); the load-bearing summary:

```json
{
  "id": "ingest-jira-internal::jira-internal-pat-x::DO",
  "owner_instance": "ingest-jira-internal",
  "source_name": "jira-internal-pat-x",
  "subsource": "DO",
  "last_complete_minute": "2026-04-30T08:14:00Z",
  "documents_synced_total": 12894,
  "last_sync_at": "2026-04-30T08:14:25Z",
  "last_error": null,
  "backfill_in_progress": false,
  "last_reconciliation_at": "2026-04-30T07:30:00Z"
}
```

`last_complete_minute` is at exact-minute resolution and means *"every change with `updated <= this minute` is durably in Cosmos"*. The reasoning is in [sync.md](sync.md).

This means:

- `quelch status` from your laptop reads a single Cosmos container and shows live state of every running worker, regardless of where it's hosted.
- A restarted worker reads its cursor on startup; no full re-sync.
- Multiple workers share the same Cosmos account without stepping on each other (each owns its cursors via `owner_instance`).

For `quelch dev` (no Cosmos), an in-memory backend implements the same trait.

### Ownership boundaries

Distributed ingest workers are designed to be **disjoint by config**, not coordinated at runtime. If you want to split Jira projects across workers, you do so in the config (`projects: [A,B,...,K]` vs `projects: [L,...,Z]`). Quelch enforces this in two layers:

- **Static — at `quelch validate`:** every ingest instance produces a claim set of `(source_type, base_url, subsource)` tuples. Any tuple claimed by ≥2 instances fails validation with a clear error naming both instances.
- **Dynamic — at runtime:** every cursor doc carries an `owner_instance` field. Q-Ingest writes its own instance name on first claim; subsequent writes by a different instance are refused with a hard error and the worker exits. This catches misconfiguration that bypassed `quelch validate` (e.g. someone independently started a second instance pointing at the same Cosmos).

Ownership transfer is explicit: `quelch reset --instance NEW_OWNER --source ... --subsource ... --take-ownership` rewrites the `owner_instance` field. Without `--take-ownership`, `reset` operates only on cursors already owned by the named instance.

## Sync correctness

Incremental sync against Atlassian APIs is the most error-prone part of the system. Atlassian's filter precision is per-minute, the document `updated` field is per-second, and Atlassian's own indexes lag — the obvious naive algorithm ("remember the latest `updated` seen, query everything ≥ that") is wrong on every one of those mismatches and was a real source of bugs in earlier iterations of this codebase.

The algorithm is:

- **Cursor at exact-minute resolution** (`last_complete_minute`), with the semantic "every change with `updated <= last_complete_minute` is durably in Cosmos".
- **Sync in closed minute-resolution intervals** with a fixed safety lag (default 2 minutes) behind real time.
- **Idempotent upserts** to Cosmos so repeating any window is harmless.
- **Crash-safe.** The cursor advances only on full window success; a crashed worker re-runs its current window from scratch.
- **Backfill resumes** from a `(updated, key)` checkpoint with a fixed `backfill_target`, so the result set walked across a resume is stable.
- **Deletions detected** via periodic full reconciliation against the source; soft-deleted in Cosmos via a `_deleted` flag that the AI Search Indexer's soft-delete column policy honours.

The full algorithm — including JQL/CQL formats, field semantics, the per-cycle pseudocode, the backfill resume protocol, and operator FAQs — is in [sync.md](sync.md). Read that document before debugging anything sync-related.

## Topology

A single Quelch installation typically looks like:

```
┌─ Your config repo ─────────────────────────────────────┐
│  quelch.yaml          (the source of truth — you edit) │
└────────────┬───────────────────────────────────────────┘
             │ quelch azure apply       (Cosmos + AI Search config)
             │ quelch instance config   (per-instance YAML to ship to hosts)
             │
   ┌─────────┴───────────────────────────────────────────────────────────┐
   ▼                                                                     ▼
┌─ Azure (configured by Quelch) ───────────────┐    ┌─ Hosts (you run) ─────┐
│                                              │    │                       │
│   Cosmos DB account                          │    │  Q-Ingest             │
│   ├─ Database: quelch                        │◄──┤  (Docker / systemd /  │
│   ├─ Containers: jira-issues, conf-pages, ...│    │   k8s / Container App │
│   └─ Container: quelch-meta                  │    │   / bare VM, ...)     │
│                                              │    │                       │
│   AI Search service                          │    │  Q-MCP                │
│   ├─ Indexes / indexers / skillsets          │◄──┤  (same hosting        │
│   ├─ Knowledge sources                       │    │   choices)            │
│   └─ Knowledge base (one per MCP instance)   │    │                       │
│                                              │    └───────┬───────────────┘
│   AI provider (Foundry / Azure OpenAI)       │            │ MCP Streamable HTTP
│   └─ Wired into the AI Search KB at apply    │            ▼
│      time (vectoriser + chat models)         │       Agent platforms
└──────────────────────────────────────────────┘
```

All Q-Ingest workers — wherever they're hosted — write to the same Cosmos account. Q-MCP reads it. Quelch never deploys hosts; it only configures the Azure side and emits the per-instance configs the user copies to their hosts.

## What `quelch azure apply` does

`apply` is the only command that writes to Azure. It does two things, in order, against the resources the user pre-provisioned:

**A. Cosmos DB containers (no Bicep, no Terraform):**
- Authenticate as the operator via `DefaultAzureCredential` (the `az login` token chain).
- Use ARM REST (`PUT /subscriptions/.../databaseAccounts/{acct}/sqlDatabases/{db}/containers/{name}`) to ensure database + containers exist with correct partition keys.
- If a container exists with a mismatched partition key, fail loudly. Never auto-recreate.

**B. Azure AI Search via the embedded rigg library:**
- Build the desired-state graph in-memory from `quelch.yaml`: indexes (one per Cosmos container exposed by any MCP instance), skillsets, indexers, data sources, knowledge sources, one knowledge base per MCP instance.
- Use rigg's existing diff / apply logic against the live AI Search service.
- No on-disk `rigg/` directory, no hand-takeover marker, no `quelch azure pull`. Drift surfaces in `azure plan` as "these will be reverted"; `apply` reverts them.
- Users wanting manual control over a specific resource remove it from `quelch.yaml` and use the standalone `rigg` tool — Quelch and `rigg`-the-tool are mutually exclusive per resource.

`quelch azure plan` is the same logic but prints the diff and exits without applying.

## Module map

```
crates/quelch/src/
├── main.rs            — CLI entry point
├── cli.rs             — clap definitions for the pruned verb set
├── config/            — schema (instances[], source_connections[]); slicing; static conflict validation
├── sources/           — SourceConnector trait + Jira/Confluence
├── ingest/            — per-cycle algorithm; writes owner_instance on cursors
├── cosmos/            — Cosmos data-plane client (real + in-memory backend)
├── mcp/               — Streamable HTTP server, 5 tools, where-grammar parser, expose filter
├── azure/
│   ├── cosmos_config/ — control-plane container CRUD via ARM REST (used by `azure apply`)
│   ├── apply.rs       — orchestrator: apply Cosmos config, then push AI Search via rigg-as-library
│   ├── plan.rs        — orchestrator: same diff logic as apply, prints and exits
│   ├── indexer.rs     — AI Search indexer ops (run / reset / status)
│   └── rigg/          — in-memory desired-state computation + diff + push (no on-disk rigg dir)
├── agent/             — agent + skill bundle generator (6 targets)
├── commands/          — operator CLI handlers (status, query, search, get, reset, instance, ...)
├── init/              — interactive `quelch init` wizard for the new schema
├── dev/               — `quelch dev` (sim + in-memory backends + ingest + MCP, all in one process)
├── tui/               — fleet dashboard polling quelch-meta
├── sim/, mock/        — activity simulator + local Jira/Confluence mock servers
└── ai.rs              — ailloy integration (reserved for future AI features)
```

## Cross-cutting concerns

- **Auth to Azure resources:** `DefaultAzureCredential` everywhere — the operator's `az login` chain in the CLI, managed identity / workload identity / service-principal env vars in Q-Ingest and Q-MCP hosts. The config never contains a literal Azure secret.
- **Auth to source systems:** PAT for Data Center, email + API token for Cloud, both as env-var references in the config (`${JIRA_PAT_X}`).
- **Auth to Q-MCP from agents:** API key in `Authorization: Bearer ...`; per-instance config references it via env var (e.g. `api_key: ${QUELCH_MCP_API_KEY}`). See [api-key.md](api-key.md).
- **Logging:** `tracing` + `tracing-subscriber`, JSON output in production, TUI-friendly fields. Per-document logs only at `debug!`.
- **Errors:** typed per module with `thiserror`, `anyhow` at CLI boundaries.
- **Versioning:** the user picks the container image tag they run; recommended is to match the CLI version (`quelch --version`) so the image and the per-instance config the CLI emitted are aligned.
- **External library deps:**
  - `rigg-core` and `rigg-client` for Azure AI Search configuration — used in-memory only; no on-disk artefacts.
  - `ailloy` for AI configuration (reserved for future AI features in Quelch itself).
  Both are MKLab tools we own; we bump versions across them in lockstep when needed.
