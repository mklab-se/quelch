# CLI reference

Every Quelch command, every flag, with examples.

The CLI is the operator's entry point. It loads `quelch.yaml`, validates it, and then either does something locally (validate, dev, query) or reaches into Azure (plan, deploy, indexer, logs). It also runs **Quelch MCP** (Q-MCP, via `quelch mcp`) and **Quelch Ingest** (Q-Ingest, via `quelch ingest`) when the binary is invoked as the long-running service inside a Container App or on-prem supervisor.

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
├── reset                      Reset ingest cursors (forces re-sync / re-backfill)
│   ├── --source <name>         Reset one source instance only
│   ├── --subsource <key>       Restrict to one project / space within --source
│   └── --yes                   Skip the confirmation prompt
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

### `quelch reset`

Clears ingest cursors so that ingest workers will run a fresh backfill on their next cycle.

```bash
quelch reset                                          # all (source, subsource) tuples — prompts
quelch reset --source jira-internal                   # one source — all subsources
quelch reset --source jira-internal --subsource DO    # one subsource only
quelch reset --source jira-internal --yes             # skip confirmation
```

`reset` only touches `quelch-meta`. Cosmos data and the AI Search index are *not* dropped — they're idempotently overwritten as the backfill progresses. To also drop the data, use `quelch azure indexer reset` (clears the AI Search index) and Cosmos `delete container` (drops the raw data).

Flags:

- `--source <name>` — restrict to one configured source.
- `--subsource <key>` — restrict to one project (Jira) or space (Confluence) within that source.
- `--yes` — skip the confirmation prompt (CI mode).

See [sync.md](sync.md) for what cursor reset means and when to use it.

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
- `--use-cosmos-emulator` — use the local Cosmos DB emulator at `https://localhost:8081`.
- `--mcp-port <P>` — port the embedded MCP server listens on (default `8080`).
- `--seed <N>` — seed the fixture data generator for deterministic runs.
- `--rate-multiplier <f>` — scale the simulated activity rate.

## Ad-hoc query commands

These are convenience wrappers around the MCP tools. They use the same code path as the MCP server, so what you see here is what the agent sees over MCP.

These commands are agent-API-shaped — they speak `data_source`, not physical container names, identical to what the deployed MCP server exposes (see [mcp-api.md](mcp-api.md)).

### `quelch query "..."`

Runs a structured query against one data source.

```bash
quelch query --data-source jira_issues \
  --where '{"assignee.email": "kristofer@example.com", "type": "Story", "status": {"not": "Done"}}' \
  --top 100
```

Flags:

- `--data-source <name>` — required. Logical data-source name (e.g. `jira_issues`, `jira_sprints`).
- `--where <json>` — structured predicate as JSON, matching the `where` grammar in [mcp-api.md](mcp-api.md#filter-grammar). Pass via shell quoting or `--where-file <path>` for complex predicates.
- `--where-file <path>` — read the `where` JSON from a file instead of the command line.
- `--order-by <field:dir>` — repeatable, e.g. `--order-by updated:desc`.
- `--top <N>` — page size.
- `--cursor <token>` — continuation token from a previous call.
- `--count-only` — return just the count.
- `--json` — raw JSON instead of formatted output.

### `quelch search "..."`

Runs a hybrid semantic search.

```bash
quelch search "camera connection problems" \
  --data-sources jira_issues,confluence_pages \
  --top 25
```

Flags:

- `--data-sources <a,b,c>` — comma-separated logical data-source names. Default: every searchable data source the active config exposes.
- `--where <json>` — optional structured filter (same grammar as `query`).
- `--top <N>` — page size.
- `--cursor <token>` — continuation.
- `--include-content [snippet|full|agentic_answer]` — see [mcp-api.md](mcp-api.md#search). Default `snippet`.
- `--json` — raw JSON.

### `quelch get <id>`

Point-read a document by id.

```bash
quelch get --data-source jira_issues jira-internal-DO-1234
```

Flags:

- `--data-source <name>` — required.
- `--json` — raw JSON.

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

### `quelch azure pull`

Pulls live AI Search and Microsoft Foundry configuration back into the local `rigg/` directory. See [deployment.md](deployment.md#quelch-azure-pull) for the full workflow.

```bash
quelch azure pull                   # all rigg-managed resources
quelch azure pull index             # only indexes
quelch azure pull knowledge_base    # only knowledge bases
quelch azure pull --diff            # show what would change locally without writing
```

Flags:

- `[<resource>]` — optional positional arg restricting to one resource type (`index`, `indexer`, `skillset`, `knowledge_source`, `knowledge_base`, `agent`, …). Default: all.
- `--diff` — show what `pull` would do without modifying local files.
- `--out <dir>` — write to a different directory than `rigg.dir` from the config.

### `quelch azure indexer`

Operate Azure AI Search Indexers from the CLI.

```bash
quelch azure indexer status
quelch azure indexer run jira-issues
quelch azure indexer reset jira-issues   # forces full re-index next run
```

`reset` clears the indexer's high-water mark, so the next run pulls every Cosmos document. Useful after a schema change. Note: this is the AI-Search-side indexer, not the Quelch ingest cursor — for that, see [`quelch reset`](#quelch-reset).

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

## `quelch mcp-key`

Manage the Q-MCP API key for a deployment. The key is the bearer token agents present in `Authorization: Bearer ...` when calling Q-MCP.

```bash
# Generate + store a fresh key, restart the running Q-MCP if it's in Azure:
quelch mcp-key set --deployment mcp

# Use a specific value (e.g. one your secret manager already minted):
quelch mcp-key set --deployment mcp --value "$(cat ~/.config/quelch/mcp.key)"

# Generate a new value and replace the stored one (alias for set without --value):
quelch mcp-key rotate --deployment mcp

# Print the current value (Azure deployments only — on-prem stores are local-only):
quelch mcp-key show --deployment mcp
```

Behaviour by deployment target:

- **`target: azure`** — `set` / `rotate` shell out to `az keyvault secret set --name quelch-mcp-api-key`, then `az containerapp revision restart` so the new value takes effect within seconds. `show` reads the secret back via `az keyvault secret show`. Your operator identity needs `Key Vault Secrets Officer` on the deployment's vault.
- **`target: onprem`** — `set` / `rotate` print the generated value plus copy-pasteable docker / systemd / k8s commands to apply it locally. `show` is rejected — Quelch can't read secrets out of a remote on-prem secret store.

Flags:

- `--deployment <name>` — required; must be a `role: mcp` deployment in the loaded config.
- `--value <key>` — `set` only; supply a specific value instead of generating one.
- `--quiet` — `set` and `rotate`; suppress printing the new key to stdout.

## Embedded helpers

### `quelch ai`

Manages the `ailloy` integration. Reserved for future AI features in Quelch itself; embeddings happen in Azure AI Search, not here.

```bash
quelch ai status     # show whether AI features are configured (default if no subcommand)
quelch ai config     # interactive configuration wizard
quelch ai test       # send a test embedding through the configured model
quelch ai enable     # mark AI features as active
quelch ai disable    # mark AI features as inactive
```

### `quelch sim`

Runs the activity simulator without the rest of the stack — useful for snapshotting deterministic fixture streams. For an end-to-end local environment (mock sources + ingest + MCP + TUI) use `quelch dev` instead.

### `quelch mock`

Starts a local mock Jira + Confluence HTTP server.

```bash
quelch mock --port 9999
```

Useful for pointing a real `quelch ingest` at a fake source without touching production.
