# CLI reference

Every Quelch command, every flag, with examples.

The CLI is the operator's entry point. It loads `quelch.yaml`, validates it, and then either does something locally (validate, dev, query) or reaches into Azure (plan, deploy, indexer, logs).

## Global flags

| Flag | Default | Meaning |
|---|---|---|
| `-c, --config <PATH>` | `quelch.yaml` | Config file path. |
| `-v, --verbose` | off | Increase verbosity (`-v`, `-vv`, `-vvv`). |
| `-q, --quiet` | off | Suppress all but errors. |
| `--json` | off | Emit logs as JSON. |
| `--no-tui` | off | Disable TUI fallback to plain logs. |
| `--version` | — | Print version. |

## Command tree

```
quelch
├── init                       Scaffold quelch.yaml interactively
├── validate                   Validate quelch.yaml without doing anything
├── effective-config <name>    Print the sliced config for one deployment
│
├── status                     Show all deployments' state from quelch-meta
│   └── --tui                  Live dashboard
│
├── ingest                     Run an ingest worker (long-running)
├── mcp                        Run the MCP server (long-running)
├── dev                        Local dev (sim + mocks + MCP, in one process)
│
├── query "..."                Run a structured query via MCP tools
├── search "..."               Run a hybrid search via MCP tools
├── get <id>                   Point-read a document
│
├── azure
│   ├── plan [<deployment>]    Synthesise Bicep + rigg files; show combined diff
│   ├── deploy [<deployment>]  Plan + apply (Bicep + rigg)
│   ├── pull [<resource>]      Pull live AI Search/Foundry config into local rigg/ files
│   ├── destroy <deployment>   Remove a deployment from Azure
│   ├── indexer
│   │   ├── run <name>          Trigger an indexer run
│   │   ├── reset <name>        Reset an indexer (force full re-index)
│   │   └── status              Show all indexers and their state
│   └── logs <deployment>       Tail logs from a deployed worker
│
├── generate-deployment <name> --target [docker|systemd|k8s]
│       Generate on-prem deployment artefacts for a `target: onprem` deployment
│
├── agent
│   └── generate --target <platform> [--output <dir>]
│       Generate agent-side instructions tailored to your config
│
├── ai                         Manage embedding/AI integration via ailloy
├── sim                        Run the simulator
└── mock                       Run a local Jira+Confluence mock server
```

## Lifecycle commands

### `quelch init`

Scaffolds a `quelch.yaml` in the current directory, interactively.

```bash
quelch init
```

It will:

1. Run `az account list` to discover available subscriptions.
2. Run `az group list` to find existing resource groups.
3. Look for an existing Cosmos DB account, AI Search service, and Azure OpenAI account in the chosen resource group.
4. Prompt for source connections (Jira, Confluence) one at a time.
5. Test source credentials against the source's API.
6. Write `quelch.yaml`.

Flags:

- `--non-interactive` — fail if any prompt would be needed.
- `--from-template <name>` — start from a built-in template instead of from scratch (`minimal`, `multi-source`, `distributed`).
- `--force` — overwrite an existing `quelch.yaml`.

### `quelch validate`

Loads the config, runs all validation rules (env vars set, deployments disjoint, exposed data sources exist, sources referenced, ...), and prints the result.

```bash
quelch validate
```

Exit code is non-zero on any failure; safe for CI.

### `quelch effective-config <deployment>`

Prints the sliced config that will be baked into that deployment's container.

```bash
quelch effective-config mcp-azure
```

Useful for understanding exactly what credentials and topology a deployed worker has access to.

## Status and observability

### `quelch status`

Reads `quelch-meta` and prints the state of every deployment: last sync time, doc counts, errors.

```bash
quelch status
```

Flags:

- `--tui` — open a live dashboard. Refreshes every few seconds.
- `--deployment <name>` — filter to one deployment.
- `--json` — machine-readable output.

### `quelch azure logs <deployment>`

Tails logs from a deployed Container App.

```bash
quelch azure logs ingest-azure-cloud --tail 200 --follow
```

Flags:

- `--tail <N>` — number of trailing lines to fetch first (default 100).
- `--follow` — stream new logs as they arrive.
- `--since <duration>` — only logs newer than this (`5m`, `1h`, `24h`).

## Long-running roles

These are typically run by Container Apps, not by you on your laptop. They exist as Quelch CLI subcommands so the same binary handles every role.

### `quelch ingest`

Runs a single ingest worker.

```bash
quelch ingest --deployment ingest-onprem-jira-ak
```

Flags:

- `--deployment <name>` — required. Tells the worker which slice of the config it owns.
- `--once` — run one cycle then exit (useful as a K8s `CronJob`).
- `--max-docs <N>` — stop after N documents (debugging only).

When `--deployment` is omitted, the worker expects a sliced config in its own filesystem (the deploy-time mode).

### `quelch mcp`

Runs the MCP server.

```bash
quelch mcp --deployment mcp-azure --port 8080
```

Flags:

- `--deployment <name>` — required.
- `--port <P>` — listen port (default `8080`).
- `--bind <ADDR>` — bind address (default `0.0.0.0`).
- `--api-key <KEY>` — override config (uses `QUELCH_MCP_API_KEY` env by default).

### `quelch dev`

The local-development shortcut. Runs sim + in-memory backends + ingest + mcp in one process. TUI is the default UX.

```bash
quelch dev
```

Flags:

- `--use-real-search` — use a real Azure AI Search service instead of the in-memory mock.
- `--use-cosmos-emulator` — use the local Cosmos DB emulator.
- `--mcp-port <P>` — port the embedded MCP server listens on (default `8080`).
- `--seed <N>` — seed the simulator for deterministic runs.
- `--rate-multiplier <f>` — speed up / slow down simulated activity.

`quelch dev` is exhaustively documented by the simulator's existing snapshot/recording features (`--snapshot-to`, `--snapshot-frames`).

## Ad-hoc query commands

These are convenience wrappers around the MCP tools. They use the same code path as the MCP server, so what you see here is what the agent sees over MCP.

### `quelch query "..."`

Runs a structured query.

```bash
quelch query --container jira-issues \
  --where 'assignee.email = "kristofer@example.com" and type = "Story" and status != "Done"' \
  --top 100
```

Flags:

- `--container <name>` — required.
- `--where <expr>` — structured predicate (see [mcp-api.md](mcp-api.md#filter-grammar)).
- `--order-by <field:dir>` — repeatable, e.g. `--order-by updated:desc`.
- `--top <N>` — page size.
- `--cursor <token>` — continuation token from a previous call.
- `--count-only` — return just the count.
- `--json` — raw JSON instead of formatted output.

### `quelch search "..."`

Runs a hybrid search.

```bash
quelch search "camera connection problems" --indexes jira-issues --top 25
```

Flags:

- `--indexes <a,b,c>` — repeatable. Default: every index exposed by the active deployment.
- `--filters <expr>` — OData filter expression.
- `--top <N>` — page size.
- `--cursor <token>` — continuation.
- `--json` — raw JSON.

### `quelch get <id>`

```bash
quelch get jira-internal-DO-1234
```

Flags:

- `--container <name>` — required if the id doesn't disambiguate.
- `--json`.

## Azure provisioning

### `quelch azure plan [<deployment>]`

Synthesises the Bicep file(s) and runs `az deployment group what-if`. Prints the diff. Does **not** apply.

```bash
quelch azure plan                  # all deployments
quelch azure plan mcp-azure        # one deployment
```

Output looks like:

```
Synthesising .quelch/azure/mcp-azure.bicep
Running az deployment group what-if ...

Resource changes:
  + Microsoft.App/containerApps/quelch-prod-mcp           (Create)
  ~ Microsoft.DocumentDB/databaseAccounts/quelch-prod-cosmos
      throughput.mode: serverless → provisioned
  - Microsoft.App/containerApps/quelch-prod-old           (Delete)

3 resources will change. Run `quelch azure deploy` to apply.
```

Flags:

- `--out <path>` — write Bicep to a custom location (default `.quelch/azure/<deployment>.bicep`).
- `--no-what-if` — synthesise only, skip Azure call.

### `quelch azure deploy [<deployment>]`

Same as `plan` plus an interactive "apply?" prompt and the actual apply step.

```bash
quelch azure deploy
quelch azure deploy mcp-azure --yes      # skip the prompt
```

Flags:

- `--yes` — skip the interactive prompt (CI mode).
- `--dry-run` — equivalent to `quelch azure plan`.

### `quelch azure destroy <deployment>`

Removes a single deployment from Azure (the Container App, its revisions, its ingress). Does **not** delete shared resources (Cosmos, AI Search, OpenAI).

```bash
quelch azure destroy ingest-azure-cloud
```

To delete the entire resource group, use `az group delete` directly.

### `quelch azure indexer`

Operate Azure AI Search Indexers from the CLI.

```bash
quelch azure indexer status
quelch azure indexer run jira-issues
quelch azure indexer reset jira-issues   # forces full re-index next run
```

`reset` clears the indexer's high-water mark, so the next run pulls every Cosmos document. Useful after a schema change.

## On-prem deployment artefacts

### `quelch generate-deployment <deployment> --target <platform>`

Produces a directory the user copies to an on-prem host and runs.

```bash
quelch generate-deployment ingest-onprem-jira-ak --target docker --output ./deploy/
```

Targets:

- `docker` — `docker-compose.yaml` + `.env.example`.
- `systemd` — `quelch-<deployment>.service` + `quelch-<deployment>.env.example`.
- `k8s` — `Deployment` + `ConfigMap` + `Secret` template + optional `Helm` chart.

The generated directory always contains:

- The artefact(s) listed above.
- The sliced effective config as a JSON/YAML file.
- A `.env.example` listing every environment variable the worker needs.
- A `README.md` with the three commands the user runs.

## Agent and skill generation

### `quelch agent generate --target <platform>`

Produces a copy-pasteable bundle of agent or skill material tailored to your deployment. The default form (agent vs skill) depends on the target; `--format` overrides it. See [agent-generation.md](agent-generation.md) for the full spec.

```bash
quelch agent generate --target copilot-studio --output ./agent-bundle
quelch agent generate --target claude-code    --output ./agent-bundle
quelch agent generate --target codex          --output ./agent-bundle
quelch agent generate --target vscode-copilot --output ./agent-bundle
quelch agent generate --target copilot-cli    --output ./agent-bundle
quelch agent generate --target markdown       --output ./agent-bundle
```

Flags:

- `--target <platform>` — required. One of `copilot-studio`, `claude-code`, `copilot-cli`, `vscode-copilot`, `codex`, `markdown`.
- `--format [agent|skill|both]` — override the target's default form.
- `--output <dir>` — output directory (default `./agent-bundle/`).

## Embedded helpers

### `quelch ai`

Manages the `ailloy` integration. In v2 this is reserved for future AI features; embeddings happen in Azure AI Search, not here.

```bash
quelch ai status
quelch ai config
```

### `quelch sim`

Runs the activity simulator without the rest of the stack. Same flags as v1 (`--duration`, `--seed`, `--rate-multiplier`, `--fault-rate`, `--snapshot-to`).

### `quelch mock`

Starts a local mock Jira + Confluence HTTP server.

```bash
quelch mock --port 9999
```

Useful for pointing a real `quelch ingest` at a fake source without touching production.
