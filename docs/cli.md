# CLI reference

Every Quelch command, every flag, with examples.

The CLI is the operator's entry point. It loads `quelch.yaml`, validates it, and either does something locally (`validate`, `dev`, `query`, `instance config`) or reaches into Azure to configure resources (`azure plan`, `azure apply`, `azure indexer`). It also runs **Quelch MCP** (Q-MCP, via `quelch mcp`) and **Quelch Ingest** (Q-Ingest, via `quelch ingest`) when the same binary is invoked as the long-running service on a host.

## Global flags

| Flag | Default | Meaning |
|---|---|---|
| `-c, --config <PATH>` | `quelch.yaml` | Config file path (master `quelch.yaml` or per-instance file). |
| `-v, --verbose` | off | Increase verbosity (`-v`, `-vv`, `-vvv`). |
| `-q, --quiet` | off | Suppress all but errors. |
| `--json` | off | Emit logs as JSON. |
| `--no-tui` | off | Disable TUI; fall back to plain logs. |
| `--version` | — | Print version. |

## Command tree

```
quelch
├── init [DIRECTORY]                       Interactive wizard, writes quelch.yaml
├── validate                                Check the config without doing anything
│
├── instance
│   ├── list                                List instances declared in the master config
│   └── config NAME --kind ingest|mcp        Emit a per-instance config file
│
├── azure
│   ├── plan                                 Show diff (Cosmos + AI Search) against Azure
│   ├── apply [--yes]                        Apply the diff
│   └── indexer
│       ├── run NAME                          Trigger an indexer run
│       ├── reset NAME                        Reset an indexer (forces full re-index)
│       └── status                             Show all indexers and their state
│
├── ingest [--instance NAME]                Run a Q-Ingest worker (long-running)
├── mcp [--instance NAME]                   Run a Q-MCP server (long-running)
│
├── status [--tui --instance NAME]          Read sync state from quelch-meta
├── reset --instance NAME --source NAME --subsource NAME [--take-ownership --yes]
│                                            Reset / transfer a cursor
│
├── query | search | get                    Operator queries against the data (same code path as MCP tools)
│
├── agent generate --target T [--instance NAME --output PATH]
│                                            Agent / skill bundle generator
│
├── dev                                     Local dev (sim + mocks + ingest + mcp, in one process)
├── mock                                    Local mock Jira + Confluence HTTP server
└── ai                                      Manage embedding integration via ailloy
```

---

## Lifecycle commands

### `quelch init`

Scaffolds a `quelch.yaml` interactively.

```bash
quelch init                    # writes ./quelch.yaml
quelch init my-quelch          # creates ./my-quelch/ if needed and writes inside
```

The wizard:

1. Asks for the Azure subscription / resource group / Cosmos / AI Search / AI provider.
2. Asks for the embedding + chat deployments inside the AI provider.
3. Asks for source connections one at a time (Jira and Confluence, by `(base_url × credential)` tuple).
4. Asks for instances — one per Q-Ingest worker plus one per Q-MCP server.
5. Optionally `git init` + writes `.gitignore` if the directory isn't already a repo.
6. Writes `quelch.yaml`.

Flags:

- `directory` (positional) — folder to write `quelch.yaml` into. Default `.`.
- `--non-interactive` — skip all prompts and write a template directly. Pair with `--from-template`.
- `--from-template <name>` — start from a built-in template. Names: `minimal`, `multi-source`, `distributed`.
- `--force` — overwrite an existing `quelch.yaml` without asking.

Errors:

- If `quelch.yaml` exists and `--force` is not set.
- If `--non-interactive` is set without `--from-template`.

### `quelch validate`

Loads the config and runs all validation rules. Exit code 0 means good.

```bash
quelch validate
quelch validate --config q-ingest-jira.yaml      # works on per-instance files too
```

Validation includes:

- Env-var resolution for every `${VAR}` reference.
- Reference integrity (`instances[].connections` resolve to `source_connections[].name`; `instances[].expose` are valid data-source names backed by Cosmos containers in `azure.cosmos.containers`).
- **Static conflict prevention** for ingest instances — any `(source_type, base_url, subsource)` tuple claimed by ≥2 ingest instances is a hard error naming both instances and the conflicting tuples.

Errors:

- Missing env vars, broken references, conflict-prevention violations.
- Malformed YAML.

---

## Instance subcommands

### `quelch instance list`

List the instances declared in the master config.

```bash
quelch instance list
```

Prints each instance with its kind (`ingest` / `mcp`) and the source connections it owns (for ingest) or data sources it exposes (for MCP).

### `quelch instance config NAME --kind ingest|mcp`

Emit the per-instance config — a slimmed slice of the master file containing only what one instance needs at runtime.

```bash
quelch instance config ingest-jira-internal --kind ingest --output q-ingest-jira.yaml
quelch instance config mcp-prod --kind mcp --output q-mcp.yaml
quelch instance config mcp-prod --kind mcp                    # to stdout
```

Flags:

- `name` (positional) — instance name from the master config.
- `--kind ingest|mcp` — sanity-check that the named instance has this kind. Catches typos that would dispatch the wrong slice to the wrong host.
- `--output <path>` — write to file instead of stdout.

The slimming rules are documented in [configuration.md](configuration.md#per-instance-config-schema).

Errors:

- If the named instance doesn't exist or its kind doesn't match `--kind`.

---

## Azure commands

### `quelch azure plan`

Compute the Cosmos + AI Search diff against the live Azure account and print it. Read-only; never mutates Azure.

```bash
quelch azure plan
```

Output covers two halves:

- **Cosmos DB**: containers Quelch will PUT (with their partition keys), against what's currently in the account.
- **Azure AI Search**: indexes, skillsets, indexers, data sources, knowledge sources, knowledge bases — diffed against the live service.

Authenticates as the operator via `DefaultAzureCredential`.

Errors:

- Unable to authenticate (`az login` first).
- Operator lacks the required RBAC: Cosmos DB Operator on the account, Search Service Contributor on the AI Search service, Cognitive Services User on the AI provider.

### `quelch azure apply [--yes]`

Compute the diff, prompt for confirmation, then push it to Azure.

```bash
quelch azure apply             # interactive
quelch azure apply --yes       # CI mode
```

Two-phase apply, in order:

1. **Cosmos DB**: ARM REST PUTs to ensure database + containers exist with correct partition keys. If a container exists with a mismatched partition key, fails loudly without auto-recreating.
2. **Azure AI Search**: rigg-as-library applies the diff against the live service.

Idempotent. Running twice with no YAML changes is a no-op.

Flags:

- `--yes` — skip the interactive `[y/N]` prompt.

Errors:

- Same as `azure plan`, plus partition-key mismatches and rigg-side errors (e.g. an index field with a type change that AI Search rejects).

### `quelch azure indexer run|reset|status [NAME]`

Operate AI Search indexers. They don't mutate config; they trigger runtime operations on already-configured indexers.

```bash
quelch azure indexer status                  # list all indexers and their state
quelch azure indexer run jira-issues         # trigger an immediate run
quelch azure indexer reset jira-issues       # clear high-water mark, force full re-index
```

The `reset` is for the AI Search Indexer (i.e. the Cosmos → Search reindex pump). For Quelch's own ingest cursors, see `quelch reset`.

Errors:

- Indexer name doesn't exist.

---

## Long-running roles

These are typically run by your hosting platform (Container Apps, k8s, systemd, ...), not by you on your laptop. They're the same `quelch` binary; the subcommand selects the role.

### `quelch ingest`

Run a Q-Ingest worker.

```bash
quelch ingest --config q-ingest-jira.yaml                     # auto-detects instance
quelch ingest --config quelch.yaml --instance ingest-jira     # explicit
quelch ingest --config quelch.yaml --once --max-docs 100      # debugging
```

Flags:

- `--instance <name>` — required when the config declares ≥2 ingest instances; auto-detected when there is exactly one.
- `--once` — run a single cycle then exit (useful for k8s `CronJob` patterns and CI smoke tests).
- `--max-docs <N>` — stop after N documents (debugging).

Errors:

- Cursor owner mismatch — another instance already owns one of the cursors this worker is about to write. Either fix the YAML or use `quelch reset --take-ownership`.
- Static conflict at validate time (see `quelch validate`).

### `quelch mcp`

Run a Q-MCP server.

```bash
quelch mcp --config q-mcp.yaml                                # auto-detects instance
quelch mcp --config quelch.yaml --instance mcp-prod
quelch mcp --config q-mcp.yaml --port 9000 --bind 127.0.0.1
```

Flags:

- `--instance <name>` — required when the config declares ≥2 MCP instances; auto-detected when there's exactly one.
- `-p, --port <P>` — listen port (default `8080`).
- `--bind <ADDR>` — bind address (default `0.0.0.0`).
- `--api-key <KEY>` — override the API key from config / env. When neither is set, Q-MCP runs in unauthenticated dev mode (don't do this in production).

Errors:

- Bind port already in use.
- Required env vars missing (the `${VAR}` references in the config).

---

## Status and observability

### `quelch status`

Read `quelch-meta` from Cosmos and print the state of every cursor.

```bash
quelch status
quelch status --instance ingest-jira-internal       # filter to one instance
quelch status --tui                                  # live dashboard
quelch status --json                                 # machine-readable
```

Per row: `last_complete_minute`, `documents_synced_total`, `last_error`, `backfill_in_progress`, `last_reconciliation_at`.

Flags:

- `--tui` — open the live fleet dashboard.
- `--instance <name>` — restrict to cursors owned by this instance.
- `--json` — JSON output.

### `quelch reset`

Reset (or transfer) a single cursor in `quelch-meta`.

```bash
quelch reset --instance ingest-jira-internal --source jira-internal-pat-x --subsource DO
quelch reset --instance new-owner --source ... --subsource ... --take-ownership
quelch reset --instance ... --source ... --subsource ... --yes      # skip confirmation
```

`reset` only touches `quelch-meta`. Cosmos data and the AI Search index are *not* dropped — they're idempotently overwritten on the next backfill.

Flags:

- `--instance <name>` — required. The instance that owns (or wants to own) the cursor.
- `--source <name>` — required. Source-connection name from the config.
- `--subsource <key>` — required. Project key (Jira) or space key (Confluence).
- `--take-ownership` — rewrite `owner_instance` even if a different instance currently owns the cursor.
- `--yes` — skip the interactive confirmation.

Errors:

- Cursor owner mismatch when `--take-ownership` is not set.

---

## Operator queries

These are convenience wrappers around the MCP tools. They use the same code path as the MCP server, so what you see is what an agent would see.

### `quelch query`

Structured query against one data source.

```bash
quelch query --data-source jira_issues \
  --where '{"assignee.email":"kristofer@example.com","status":{"not":"Done"}}' \
  --top 100
```

Flags:

- `--data-source <name>` — required. Logical data-source name.
- `--where <json>` / `--where-file <path>` — structured predicate. See [mcp-api.md "Filter grammar"](mcp-api.md#filter-grammar).
- `--order-by <field:dir>` — repeatable, e.g. `--order-by updated:desc`.
- `--top <N>` — page size (default 50).
- `--cursor <token>` — continuation from a previous response.
- `--count-only` — return just the count.
- `--include-deleted` — include soft-deleted documents.
- `--json` — raw JSON.
- `--instance <name>` — MCP instance to use as the source of expose rules. Auto-detected when there's only one MCP instance.

### `quelch search`

Hybrid semantic search via the AI Search Knowledge Base.

```bash
quelch search "camera connection problems" \
  --data-sources jira_issues,confluence_pages --top 25
```

Flags:

- `query` (positional) — required. Free-text query.
- `--data-sources <a,b,c>` — comma-separated logical data sources. Default: every searchable data source the active MCP instance exposes.
- `--where <json>` — optional structured filter.
- `--top <N>` — page size (default 25).
- `--cursor <token>` — continuation.
- `--include-content [snippet|full|agentic_answer]` — content level. Default `snippet`. See [mcp-api.md](mcp-api.md#search).
- `--include-deleted` — include soft-deleted documents.
- `--json` — raw JSON.
- `--instance <name>` — MCP instance. Auto-detected as above.

### `quelch get`

Point-read a document by id.

```bash
quelch get --data-source jira_issues jira-internal-pat-x-DO-1234
```

Flags:

- `id` (positional) — required.
- `--data-source <name>` — required.
- `--include-deleted` — return soft-deleted documents (default returns null for them).
- `--json` — raw JSON.
- `--instance <name>` — auto-detected.

---

## `quelch dev`

The all-in-one local sandbox. Runs sim + in-memory backends + ingest + MCP in one process. TUI is the default UX.

```bash
quelch dev
quelch dev --no-tui                # use the global --no-tui flag
quelch dev --mcp-port 9000 --seed 42 --rate-multiplier 5.0
```

Flags:

- `--use-real-search` — use a real Azure AI Search service instead of the in-memory mock (requires Azure credentials).
- `--use-cosmos-emulator` — use the local Cosmos DB Emulator at `https://localhost:8081` instead of the in-memory backend.
- `--mcp-port <P>` — port for the embedded MCP server (default `8080`).
- `--seed <N>` — seed the fixture data generator (deterministic runs; reserved for future use).
- `--rate-multiplier <f>` — scale the simulated activity rate (default `1.0`; reserved for future use).

`--no-tui` is the global flag (suppresses the TUI and emits structured logs to stdout instead).

---

## `quelch mock`

Start a local mock Jira + Confluence HTTP server. Useful for pointing a real `quelch ingest` at a fake source without touching production.

```bash
quelch mock --port 9999
```

Flags:

- `-p, --port <P>` — listen port (default `9999`).

---

## Agent and skill generation

### `quelch agent generate --target <platform>`

Produce a copy-pasteable bundle of agent or skill material tailored to your active MCP instance. See [agent-generation.md](agent-generation.md).

```bash
quelch agent generate --target copilot-studio --output ./bundle-copilot
quelch agent generate --target claude-code    --output ./bundle-claude
quelch agent generate --target codex          --output ./bundle-codex
quelch agent generate --target vscode-copilot --output ./bundle-vscode
quelch agent generate --target copilot-cli    --output ./bundle-copilot-cli
quelch agent generate --target markdown       --output ./bundle-md
```

Flags:

- `--target <platform>` — required. One of `copilot-studio`, `claude-code`, `copilot-cli`, `vscode-copilot`, `codex`, `markdown`.
- `--format [agent|skill|both]` — override the target's default form.
- `--output <dir>` — output directory (default `./agent-bundle`).
- `--instance <name>` — MCP instance to base the bundle on. Auto-detected when there's only one.
- `--url <url>` — override the public URL of the MCP server (for custom domains where the URL isn't derivable from config).

Errors:

- Unknown target.
- Multiple MCP instances declared and `--instance` not provided.

---

## Embedded helpers

### `quelch ai`

Manages the `ailloy` integration. Reserved for future AI features in Quelch itself; embeddings happen in Azure AI Search, not here.

```bash
quelch ai status      # show whether AI features are configured
quelch ai config      # interactive configuration
quelch ai test        # send a test embedding through the configured model
quelch ai enable      # mark AI features as active
quelch ai disable     # mark AI features as inactive
```
