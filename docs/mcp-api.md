# MCP API reference

Quelch exposes a small, opinionated MCP toolset designed for agents that need to answer real questions about Jira, Confluence, and similar enterprise data — including questions that require **exhaustive** answers, **exact** counts, and **semantic** understanding in the same turn.

This reference covers the transport, authentication, the five tools, the filter grammar, pagination, and the schema-discovery contract.

## Transport

Quelch's MCP server speaks the **MCP Streamable HTTP** transport. This is the network-friendly transport supported by Microsoft 365 Copilot Studio, GitHub Copilot CLI, the VS Code MCP integration, and Claude Code.

> The exact transport spec is checked against the latest MCP specification at implementation time; this document describes intent, not protocol bits.

The server URL is the public ingress of the Container App, e.g.

```
https://quelch-prod-mcp.<region>.azurecontainerapps.io
```

## Authentication

### v1 — API key

The deployed Container App generates and stores an API key in Azure Key Vault. Clients pass it as a header:

```
Authorization: Bearer <api-key>
```

Retrieve the key with:

```bash
quelch agent generate --target <...>     # the bundle includes the key
# or
az keyvault secret show --vault-name quelch-prod-kv --name mcp-api-key
```

### v1.x — Microsoft Entra ID

When `mcp.auth.mode: "entra"` is set, the Container App uses Container Apps' built-in Easy Auth integration. Agent platforms (Copilot Studio, VS Code MCP) acquire a token for the Quelch app registration and present it as a bearer token.

Until you have an Entra app registration to use, stay on `api_key`.

## The five tools

Each tool has a single, clearly described purpose. Agents pick the right one based on what they're trying to do.

| Tool | Backend | When to use |
|---|---|---|
| `search` | Azure AI Search | The agent has natural language and wants the index's semantic ranking. |
| `query` | Cosmos DB | The agent has exact filters and needs every match (exhaustive). |
| `get` | Cosmos DB | The agent already has a document id. |
| `list_sources` | Static + sample | The agent needs to learn the shape of the data before querying. |
| `aggregate` | Cosmos DB | The agent needs counts, sums, or grouped totals. |

### `search`

```yaml
name: search
description: |
  Hybrid semantic + keyword search across one or more Quelch indexes.
  Use this when the user's question contains natural-language concepts that
  may be expressed differently across documents (e.g. "connection problems"
  matching "wifi issues" and "camera disconnects").

  Returns ranked results with deep links back to the source system. Use
  list_sources first if you don't yet know which indexes to search.
arguments:
  query:
    type: string
    required: true
  indexes:
    type: string[]
    required: false
    description: Subset of exposed indexes to search. Default: all.
  filters:
    type: string
    required: false
    description: |
      OData-style filter expression applied alongside semantic ranking.
      Example: `project_key eq 'DO' and status ne 'Done'`.
  top:
    type: integer
    required: false
    default: 25
    max: 100
  cursor:
    type: string
    required: false
    description: Continuation token from a previous response.
returns:
  items:
    - id, score, source, source_link, snippet, fields
  next_cursor: string|null
  total_estimate: integer
```

Notes:

- `total_estimate` is approximate (AI Search returns an estimated count). For exact counts, use `aggregate` or `query --count-only`.
- The agent should iterate with `cursor` until `next_cursor` is `null` if it needs all matches.

### `query`

```yaml
name: query
description: |
  Structured query against a single Cosmos DB container. Use this when the
  user's question maps to exact filters and ordering — for example, "all my
  open Stories in DO". Returns every match (paginated by cursor) and an
  exact total count.
arguments:
  container:
    type: string
    required: true
  where:
    type: object
    required: false
    description: |
      Structured predicate. See "Filter grammar" below.
  order_by:
    type: array
    required: false
    description: |
      Repeatable. Each element: { field: string, dir: "asc"|"desc" }.
  top:
    type: integer
    default: 50
    max: 1000
  cursor:
    type: string
    required: false
  count_only:
    type: boolean
    default: false
returns:
  items:
    - <full Cosmos document>
  next_cursor: string|null
  total: integer
```

`query` is the right tool when the user wants "all" of something.

### `get`

```yaml
name: get
description: |
  Point-read a document by id from Cosmos DB. Returns the full document or
  null if not found.
arguments:
  container:
    type: string
    required: true
  id:
    type: string
    required: true
returns:
  document: object|null
```

### `list_sources`

```yaml
name: list_sources
description: |
  Enumerate the data sources exposed by this Quelch deployment, with
  schema hints, common enum values, and example filter expressions. Call
  this BEFORE constructing query/search filters if you don't already know
  the shape of the data.
arguments: { }
returns:
  sources:
    - container: string                 # Cosmos container name
      index: string|null                # AI Search index name (null if not searchable)
      source_type: string               # "jira_issue" | "confluence_page" | "jira_sprint" | ...
      source_names: string[]            # which configured sources contribute
      schema:
        - field: string
          type: string                  # "string" | "integer" | "datetime" | "object" | "array<...>"
          enum: string[]|null           # known values when small (e.g. status)
          description: string|null
      examples:
        - { description: "...", filter: "...", search_query: "..." }
```

This is the agent's grounding source. It's how the LLM learns that `jira-issues` has a `status` field whose values are typically `Open`, `In Progress`, `In Review`, `Done`.

### `aggregate`

```yaml
name: aggregate
description: |
  Counts, sums, and grouped totals over a Cosmos container. Use this for
  "how many", "how much", or "top N grouped by ...". Always returns exact
  numbers.
arguments:
  container:
    type: string
    required: true
  where:
    type: object
    required: false
  group_by:
    type: string|null
    required: false
  count:
    type: boolean
    default: true
  sum_field:
    type: string|null
    required: false
  top_groups:
    type: integer
    required: false
    description: When group_by is set, return only the top N groups.
returns:
  groups:
    - { key: string|null, count: integer, sum: number|null }
  total: { count: integer, sum: number|null }
```

Examples:

- `aggregate(container="jira-issues", where={project_key:"DO", status:["In Progress","To Do"]}, sum_field="story_points")` → "how much work is left".
- `aggregate(container="jira-issues", where={created:{gte:"6 months ago"}}, group_by="labels", count=true, top_groups=5)` → "top 5 most common labels last 6 months".

## Filter grammar

There are two filter syntaxes in the API, used for different reasons:

- **`where` (structured object)** — used by `query` and `aggregate`. Translated by the MCP layer into Cosmos SQL. Designed to be obvious to an LLM.
- **`filters` (OData string)** — used by `search`. Passed through to Azure AI Search as an OData `$filter` expression (note: AI Search uses `/` for nested fields, e.g. `sprint/id eq '204'`).

This document describes the structured `where` grammar. For OData, see Azure AI Search documentation.

```jsonc
// Equality (default)
{ "status": "Open" }

// Membership
{ "type": ["Story", "Task", "Bug"] }

// Comparison
{ "story_points": { "gte": 3, "lt": 8 } }

// Date / duration
{ "created": { "gte": "6 months ago" } }
{ "updated": { "gte": "2026-01-01T00:00:00Z" } }

// Pattern matching (SQL-style; % is the wildcard)
{ "name": { "like": "iXX-%" } }
{ "summary": { "like": "%firmware%" } }

// Negation
{ "status": { "not": "Done" } }

// Nested fields (use dots — these are translated to Cosmos SQL paths)
{ "assignee.email": "kristofer@example.com" }
{ "sprint.state": "active" }

// Boolean combination
{
  "and": [
    { "project_key": "DO" },
    { "type": ["Story", "Task", "Bug"] },
    { "or": [
      { "status": "In Progress" },
      { "status": "In Review" }
    ]}
  ]
}

// Existence
{ "fix_versions": { "exists": true } }
```

Relative dates supported: `N seconds|minutes|hours|days|weeks|months|years ago`.

## Pagination

All list-returning tools (`search`, `query`) use opaque cursor strings.

- The first call omits `cursor`.
- The response includes `next_cursor` (string) or `null` when exhausted.
- Subsequent calls pass the previous `next_cursor` as `cursor` to continue.

Cursors are short-lived but deterministic enough that an agent can reliably page through "all" results without missing or duplicating items, as long as the underlying data isn't being modified mid-iteration.

For exact totals, agents should call `aggregate` or set `count_only: true` on `query` rather than counting paginated items themselves.

## The `expose:` filter

A deployed MCP server only sees the containers / indexes its `mcp.expose:` config lists. Calls referencing anything else return `403 Forbidden`. This is enforced server-side and is independent of agent identity.

`list_sources` reflects only exposed sources, so the agent never even learns about hidden ones.

## Errors

All tools return errors as MCP errors with structured payloads:

| Code | Meaning |
|---|---|
| `not_found` | The requested document/container/index doesn't exist. |
| `forbidden` | The container/index is not exposed by this deployment. |
| `invalid_argument` | Bad filter, unknown field, malformed cursor. |
| `unauthenticated` | Missing or invalid auth header. |
| `unavailable` | Backend (Cosmos / AI Search) returned a retryable error after retries. |
| `internal` | Unexpected server-side error; check `quelch azure logs`. |

## Result shape — the `source_link` contract

Every item returned by `search`, `query`, `get`, or `aggregate` (when items are returned) carries a top-level `source_link` field — the canonical URL back to the source system:

- Jira issues: `https://<jira>/browse/<KEY>`.
- Confluence pages: `https://<confluence>/<space>/<page-id>` (or the actual `_links.webui`).

Agents must surface this link in their response so the user can click through to verify.

## Discoverability flow (recommended for agents)

For any non-trivial question, the recommended agent flow is:

1. Call `list_sources` once at conversation start. Cache it.
2. Inspect the schema — what containers exist, what fields they have, what enum values are typical.
3. Pick the right tool: `search` if the question is fuzzy, `query` if it's precise, `aggregate` if it's a count.
4. Run the call.
5. If it returned `next_cursor`, decide whether to paginate: agents should paginate exhaustively when the user asked for "all", and stop early when they asked for a sample.
6. Return results with `source_link`s.

The `quelch agent generate` command (see [agents.md](agents.md)) produces system-prompt material that encodes exactly this flow, customised to the agent platform you're using.
