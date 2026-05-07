# Getting started

This walkthrough takes you from `brew install` to a working agent query against your own Jira and Confluence, in eleven concrete steps.

The new mental model: **Quelch configures, you host.** Quelch reaches into your Azure subscription to create Cosmos DB containers and configure Azure AI Search (indexes, skillsets, indexers, knowledge sources, knowledge base). It does **not** deploy Q-Ingest or Q-MCP processes — those run wherever you choose to host them (Docker, systemd, Kubernetes, Azure Container Apps, a bare VM). Quelch generates a small per-instance config file for each one; you take it from there.

Two service components, named throughout:

- **Quelch Ingest** (Q-Ingest) — pulls data from Jira / Confluence into Cosmos DB. One process per ingest *instance* (an instance owns a disjoint slice of the source data).
- **Quelch MCP** (Q-MCP) — the Streamable-HTTP MCP server agents talk to. Reads Cosmos DB and Azure AI Search; never writes.

If you only want to **evaluate Quelch locally** without Azure, jump straight to [Try it offline first with `quelch dev`](#try-it-offline-first-with-quelch-dev) at the bottom.

---

## 1. Install Quelch

```bash
brew install mklab-se/tap/quelch     # macOS / Linux
# or
cargo install quelch
```

Verify:

```bash
quelch --version
```

You also need the Azure CLI:

```bash
az login
az account show       # confirm the right subscription is active
```

Quelch uses `DefaultAzureCredential` and reuses your `az login` token chain. There is no separate Quelch identity.

---

## 2. Create the Azure resources Quelch depends on

Quelch does not provision Azure resources at the account level — it only configures their internals. You create the empty shells once with `az`, Bicep, Terraform, the portal, or whatever you already use; Quelch doesn't care.

What you need:

| Resource | Purpose |
|---|---|
| **Resource group** | Container for everything below. |
| **Cosmos DB account** (NoSQL API) | System of record. Quelch creates the database and containers inside via ARM REST. |
| **Azure AI Search service** (Basic+ with semantic ranker enabled) | Hosts the indexes, indexers, knowledge sources, and the agentic Knowledge Base. |
| **Microsoft Foundry project *or* Azure OpenAI account** | Holds the embedding deployment (used by the AI Search vectoriser) and the chat deployment (used by the Knowledge Base for query planning + answer synthesis). |

Copy-pasteable `az` commands:

```bash
RG=rg-quelch-prod
LOC=swedencentral

az group create -n "$RG" -l "$LOC"

az cosmosdb create -n my-cosmos -g "$RG" \
  --kind GlobalDocumentDB --capabilities EnableServerless

az search service create -n my-search -g "$RG" --sku basic
# Then enable the semantic ranker:
# https://learn.microsoft.com/azure/search/semantic-how-to-enable-disable

# Pick ONE AI provider. Foundry is recommended (newer surface).
# Foundry: create the project + deploy text-embedding-3-large + gpt-5-mini
#          in https://ai.azure.com — there is no `az foundry` yet.
# Azure OpenAI:
az cognitiveservices account create -n my-openai -g "$RG" \
  --kind OpenAI --sku S0 -l "$LOC"
# Then deploy text-embedding-3-large and gpt-5-mini in the portal.
```

**Recommended models:** `text-embedding-3-large` (3072 dims) for embeddings; `gpt-5-mini` for chat (in Microsoft's portal-validated subset for AI Search Knowledge Base, similar cost / latency to `gpt-4.1-mini`, newer).

### RBAC the operator running `quelch azure apply` needs

Quelch authenticates to Azure as you. Make sure your principal has, on the resources above:

- **Cosmos DB Operator** on the Cosmos account (to PUT containers via ARM REST).
- **Search Service Contributor** on the AI Search service (to manage indexes / indexers / knowledge bases via the admin API).
- **Cognitive Services User** on the AI provider (so the Knowledge Base can be wired with embedding + chat deployments).

You also need read access to the resource group itself. None of these roles allow Quelch to create the resources — they only let it configure their internals.

Source-system credentials (Jira / Confluence PATs / API tokens) are not Azure roles — they live in your shell and end up in env vars Quelch reads at runtime.

---

## 3. Configure Azure resources with the Q-CLI

Pick a directory in your config repo and run:

```bash
mkdir -p ~/work/my-quelch && cd ~/work/my-quelch
quelch init
```

The wizard:

- Asks for the Azure account / resource group / Cosmos / Search / AI provider you created in step 2.
- Asks for the embedding + chat deployments inside the AI provider.
- Asks for your source connections (Jira and Confluence, by `(base_url × credential)` tuple).
- Asks for the named instances you want — one per Q-Ingest worker plus one per Q-MCP server. For a first run, "one ingest instance per source connection plus one MCP" is fine; add more later.
- Writes `quelch.yaml` (and offers to `git init` + write `.gitignore` if the directory isn't already a repo).

Credentials never end up in `quelch.yaml`. The wizard records env-var references like `${JIRA_PAT_X}`; you set the actual values in your shell before running ingest / MCP.

Sanity-check the config:

```bash
quelch validate
```

Validation includes a static **conflict prevention** pass: if any two ingest instances claim overlapping `(source_type, base_url, subsource)` tuples, you get a clear error naming both instances and the conflicting tuples. Fix in YAML before applying.

Show the diff Quelch would apply to Azure:

```bash
quelch azure plan
```

This prints a combined diff in two halves:

- **Cosmos DB**: containers Quelch will PUT (database + containers, with partition keys).
- **Azure AI Search**: indexes, skillsets, indexers, data sources, knowledge sources, and a knowledge base per MCP instance — diffed against the live service.

Read it. Then apply:

```bash
quelch azure apply
```

`azure apply` is idempotent. Running it again with no YAML changes is a no-op. If you change `quelch.yaml`, it will diff against the live state and apply only the differences.

> **No Bicep, no Container Apps, no Key Vault, no role assignments.** `quelch azure apply` only touches Cosmos DB containers and the AI Search service's contents. Hosting Q-Ingest / Q-MCP is up to you (steps 7 and 9).

---

## 4. Test Q-Ingest locally against one source

Before moving anything to production, smoke-test ingest from your laptop. Set the credential env vars referenced by `quelch.yaml`:

```bash
# Whichever your config references — typical setup:
export JIRA_PAT_X="..."
export CONFLUENCE_PAT="..."
export AZURE_SUBSCRIPTION_ID="$(az account show --query id -o tsv)"
```

Pick one ingest instance and run it directly against your real Cosmos:

```bash
quelch ingest --config quelch.yaml --instance ingest-jira-internal
```

The worker starts, claims its cursors in `quelch-meta`, and runs the first cycle. Initial backfill takes 1–10 minutes for a typical Jira project; for a 10k-issue project it can take longer. The cursor advances incrementally — interrupting and restarting picks up where it left off.

In another terminal:

```bash
quelch status
```

Reads `quelch-meta` and shows last-sync time and document count per `(instance, source, subsource)`. Wait until `documents_synced_total` climbs above zero on at least one tuple — that confirms data is landing in Cosmos.

Stop the local ingest when you're satisfied (`Ctrl-C`).

---

## 5. Test Q-MCP locally

Q-MCP needs an API key for agent authentication. The env-var **name** was decided during `quelch init` — open `quelch.yaml`, find the `mcp-prod` instance, and look at the `api_key:` field; it will read something like `api_key: ${MCP_PROD_API_KEY}`. The wizard derives the default from the instance name (`mcp-prod` → `MCP_PROD_API_KEY`), but you may have chosen a different name. The rest of this section uses `MCP_PROD_API_KEY` as a stand-in — substitute whatever your yaml references.

Generate the **value** once and reuse it everywhere — Q-MCP and the agent / `curl` test must present the matching string:

```bash
export MCP_PROD_API_KEY="$(openssl rand -base64 32)"
echo "$MCP_PROD_API_KEY"   # note this value — you'll re-export it in any other shell that talks to Q-MCP
```

> Re-running `openssl rand …` produces a **new** key each time. If you do that in a second terminal, your `curl` will fail with `unauthenticated`.

(See [api-key.md](api-key.md) for the longer story — generation, storage, rotation, secret-store integration.)

Start Q-MCP in the current terminal:

```bash
quelch mcp --config quelch.yaml --instance mcp-prod
```

By default it listens on `0.0.0.0:8080`. From another terminal, export the **same** value (paste the string you noted above) and confirm connectivity:

```bash
export MCP_PROD_API_KEY=<paste the value from above>

curl -X POST http://127.0.0.1:8080/mcp \
  -H "Authorization: Bearer $MCP_PROD_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```

You should see five tools come back: `search`, `query`, `get`, `list_sources`, `aggregate`. That round-trip is the connectivity check.

Stop the local MCP (`Ctrl-C`).

---

## 6. Add additional sources

Add more `source_connections` (one per `(base_url × credential)` tuple) and either fold them into existing instances or declare new ones:

```yaml
source_connections:
  - name: jira-internal-pat-x
    type: jira
    base_url: https://jira.internal/
    auth: { kind: pat, token: ${JIRA_PAT_X} }
    projects: [DO, ANNA]

  - name: confluence-internal       # NEW
    type: confluence
    base_url: https://confluence.internal/
    auth: { kind: pat, token: ${CONFLUENCE_PAT} }
    spaces: [ENG, DOCS]

instances:
  - name: ingest-jira-internal
    kind: ingest
    connections: [jira-internal-pat-x]
    cycle_interval: 5m

  - name: ingest-confluence-internal   # NEW
    kind: ingest
    connections: [confluence-internal]
    cycle_interval: 10m
```

Then:

```bash
quelch validate           # static conflict check — fails fast on overlapping ingest claims
quelch azure plan         # diff: typically just adds new AI Search artefacts for the new container
quelch azure apply
```

Test the new ingest the same way as step 4:

```bash
quelch ingest --config quelch.yaml --instance ingest-confluence-internal
```

Quelch is built for repeated edit / validate / apply / test cycles. There is no "tear down and start over" — every apply is incremental.

---

## 7. Move Q-Ingest to production

The local test in step 4 proved the credentials and config work. Now move the worker off your laptop.

Generate a per-instance config file:

```bash
quelch instance config ingest-jira-internal --kind ingest --output q-ingest-jira.yaml
```

The output is a **slimmed** config containing only what this one Q-Ingest needs — no control-plane Cosmos fields, no AI Search endpoint, no other instances, no other source connections. Ship it.

On the host where you want to run the worker:

1. Copy `q-ingest-jira.yaml` to the host.
2. Set the credential env vars in the host's secret store. The variables are exactly the `${VAR}` references in the file; missing ones produce a precise error from `quelch validate --config q-ingest-jira.yaml`.
3. Run:
   ```bash
   quelch ingest --config q-ingest-jira.yaml
   ```

   `--instance` is auto-detected when the file contains exactly one ingest instance.

For copy-pasteable Docker / systemd / Kubernetes / Azure Container Apps snippets, see [docs/hosting.md](hosting.md). Quelch generates only the per-instance config — the supervisor / scheduler / image runtime is yours.

---

## 8. Verify the deployed Q-Ingest is working

`quelch status` reads `quelch-meta` from Cosmos and shows live sync state for every running ingest, regardless of where it's hosted:

```bash
quelch status                       # one-shot
quelch status --tui                 # live fleet dashboard
```

Per row you see `last_complete_minute`, `documents_synced_total`, `last_error`, and `backfill_in_progress`. A worker that's caught up shows `last_complete_minute` within the last few minutes; one that's still backfilling has `backfill_in_progress: true`.

### Cursor ownership

Every cursor in `quelch-meta` carries an `owner_instance`. The first instance to claim a cursor writes its name; subsequent writes by a *different* instance are refused with a hard error. That catches misconfiguration where two hosts accidentally point at the same Cosmos with overlapping config — the second one fails fast instead of clobbering the first one's cursors.

If you legitimately want to transfer ownership (e.g. you renamed an instance, or you're migrating between hosts), use:

```bash
quelch reset --instance new-owner --source jira --subsource DO --take-ownership
```

Without `--take-ownership`, `reset` only operates on cursors already owned by the named instance.

---

## 9. Move Q-MCP to production

Same shape as step 7:

```bash
quelch instance config mcp-prod --kind mcp --output q-mcp.yaml
```

The slimmed config contains the Cosmos and AI Search endpoints, the MCP listen address, and an `api_key: ${...}` env-var reference (the name your wizard chose for this instance — typically `MCP_PROD_API_KEY` for an instance named `mcp-prod`). No source connections (Q-MCP doesn't pull from sources).

On the host:

1. Copy `q-mcp.yaml` over.
2. Generate an API key per [docs/api-key.md](api-key.md) (`openssl rand -base64 32`) and put it in the host's secret store under the env-var name your `q-mcp.yaml` references in `api_key:`.
3. Run:
   ```bash
   quelch mcp --config q-mcp.yaml
   ```

For Docker / systemd / Kubernetes / Azure Container Apps snippets, see [docs/hosting.md](hosting.md).

Once Q-MCP is up at a public address, generate an agent bundle and connect:

```bash
quelch agent generate --target claude-code --output ./bundle-claude
# Or: --target copilot-studio | copilot-cli | vscode-copilot | codex | markdown
```

Each bundle's `README.md` walks through the platform-specific installation. The bundle's `.mcp.json` references `${QUELCH_API_KEY}` — set it on the agent host to the same value the Q-MCP server expects.

---

## 10. Monitor and reconfigure the running instances

The day-to-day operator surface:

- **Sync state** — `quelch status` (or `--tui` for the live dashboard).
- **Operator queries against the data** — `quelch query`, `quelch search`, `quelch get`. These speak the same five-tool MCP API your agents see; useful for spot-checking behaviour without involving an agent.
- **Reset a stuck cursor** — `quelch reset --instance NAME --source ... --subsource ...`. Forces a fresh backfill on the next cycle. Add `--take-ownership` only if you're moving the cursor between instances.
- **Nudge an indexer** — `quelch azure indexer run|reset|status [NAME]`. The AI Search index can be stale (its run cadence is on the order of minutes); these commands trigger an immediate run, force a full re-index, or check status.

To change config (add a new project to a connection, expose a new data source on Q-MCP, etc.):

1. Edit `quelch.yaml`.
2. `quelch validate`.
3. `quelch azure apply` — picks up the diff.
4. `quelch instance config NAME --kind ...` — re-emit the affected per-instance configs.
5. Restart the affected hosts so they pick up the new per-instance configs.

For the full operator command surface, see [cli.md](cli.md).

---

## 11. Add more sources later

The same loop as step 6, then step 7 if you need a new ingest instance. To see what's currently declared:

```bash
quelch instance list
```

Lists every named instance in the master `quelch.yaml` along with its kind and the source connections it owns.

---

## Try it offline first with `quelch dev`

If you want to evaluate Quelch *before* committing to any Azure spend, `quelch dev` runs the simulator, an in-memory Cosmos backend, an ingest worker, and the MCP server — all in one process, no Azure account needed.

```bash
quelch dev
```

This:

- Spawns mock Jira and Confluence HTTP servers fed by the activity simulator.
- Runs an ingest worker against those mocks (in-memory Cosmos).
- Exposes a local MCP server on `127.0.0.1:8080`.
- Renders the fleet-dashboard TUI.

You can point a local agent at `http://127.0.0.1:8080/mcp` and exercise the same five tools you'd hit against a real deployment. Press `q` in the TUI to quit.

Useful flags:

- `--no-tui` — disable the dashboard, emit structured logs to stdout.
- `--mcp-port 9000` — bind the MCP server elsewhere (default `8080`).
- `--seed 42` — deterministic simulator output for reproducible runs.
- `--rate-multiplier 5.0` — speed up simulated activity.

This is the recommended way to **first** experience Quelch.

---

## Where to next

- **Hosting recipes** — copy-paste snippets for Docker / systemd / Kubernetes / Container Apps: [hosting.md](hosting.md).
- **Configuration reference** — every field in `quelch.yaml`: [configuration.md](configuration.md).
- **CLI reference** — every command, flag, and example: [cli.md](cli.md).
- **MCP API reference** — for agent authors: [mcp-api.md](mcp-api.md).
- **Q-MCP API key handling** — generation, storage, rotation: [api-key.md](api-key.md).
- **Real-question walkthroughs** — how an agent uses each MCP tool: [examples.md](examples.md).
- **Sync correctness deep-dive** — if you're debugging anything sync-related: [sync.md](sync.md).

If something doesn't work, the first things to check are `quelch validate` and `quelch azure plan` (do they fail / show unexpected drift?), then `quelch status` (is ingest making progress?), then your host's own log system (Container Apps log streaming, `journalctl`, `kubectl logs`, `docker logs` — Quelch isn't running the workload, so it doesn't tail your logs).
