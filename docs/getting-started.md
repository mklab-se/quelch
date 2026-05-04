# Getting started

This walkthrough sets up a working Quelch deployment end-to-end: a Cosmos-DB-backed knowledge platform fed from your Jira and Confluence, with **Quelch MCP** (Q-MCP) exposing the data so an agent (Copilot Studio, Claude Code, VS Code Copilot, GitHub Copilot CLI, OpenAI Codex) can talk to it.

Two service components, one config file:

- **Quelch MCP** (Q-MCP) — the MCP server agents call. Most often runs in **Azure** (Container Apps), since it's a long-running HTTP service and benefits from Azure's identity / networking story. It doesn't have to.
- **Quelch Ingest** (Q-Ingest) — the worker that pulls from each data source. Most often runs **close to the source** — for Atlassian Cloud that can be Azure too, but for Jira / Confluence Data Center it's typically on-prem next to those servers. Q-Ingest writes into the same Cosmos DB account that Q-MCP reads from.

This walkthrough covers the **happy path**: Q-MCP in Azure + Q-Ingest in Azure with Atlassian Cloud sources. For mixed topologies (Q-MCP in Azure + Q-Ingest on-prem), see [deployment.md "Hybrid topology"](deployment.md#hybrid-topology).

If you just want to **evaluate Quelch locally** without touching Azure or your real source systems, skip ahead to [Try it offline first with `quelch dev`](#try-it-offline-first-with-quelch-dev).

---

## 0. Prerequisites

Quelch deliberately does **not** provision the Azure infrastructure it depends on — it only configures internals (Cosmos containers, AI Search indexes / skillsets / knowledge sources / knowledge bases) and deploys the Container App that runs the MCP server. The rest you create up front in Azure. That's a deliberate split: it keeps the Quelch-managed surface small, makes role assignments transparent, and avoids fighting the quota and capacity issues that come with provisioning Cognitive Services accounts.

### Tooling

- **Quelch installed**:
  ```bash
  brew install mklab-se/tap/quelch     # macOS / Linux
  # or
  cargo install quelch
  ```
- **Azure CLI installed and logged in**:
  ```bash
  az login
  az account show       # confirm the right subscription is active
  ```
  Quelch uses your `az` credentials directly — there is no separate Quelch identity.
- **At least Contributor on the resource group** you'll work in. **Owner** (or User Access Administrator) is needed if you want Quelch to grant the Container App's managed identity RBAC on Cosmos / AI Search / Key Vault / the AI provider — set `azure.skip_role_assignments: true` and apply the role assignments manually if you don't have that.
- **A Git repository** to commit `quelch.yaml`, the generated `.quelch/` and `rigg/` directories. Treat the config as code.

### Azure resources you must create before running `quelch init`

What you actually need depends on **where Q-MCP and Q-Ingest will run**. Pick a topology, create the resources for it.

All-in-Azure setups (the happy path this doc walks through): everything in the **same resource group** keeps the wizard simple. (Cross-RG references are supported for shared resources like a Foundry project owned by another team — that's covered in [deployment.md "Hybrid topology"](deployment.md#hybrid-topology).)

#### Always required (regardless of where Q-MCP / Q-Ingest run)

| Resource | Why Quelch needs it | Create with |
|---|---|---|
| **Resource group** | Container for the resources below | `az group create -n <rg> -l <region>` |
| **Cosmos DB account** (NoSQL API) | System of record. Quelch creates the database and containers inside. Both Q-MCP and Q-Ingest read/write it. | `az cosmosdb create -n <name> -g <rg> --kind GlobalDocumentDB --capabilities EnableServerless` |
| **Azure AI Search service** (Basic+, semantic ranker enabled) | Hosts the indexes and the agentic Knowledge Base that Q-MCP queries via the `search` tool. | `az search service create -n <name> -g <rg> --sku basic` then [enable semantic ranker](https://learn.microsoft.com/azure/search/semantic-how-to-enable-disable). |
| **AI model provider** — pick one: |||
| &nbsp;&nbsp;**Microsoft Foundry project** *(recommended)* | Holds the embedding deployment (used by the AI Search vectorizer) and the chat deployment (used by the Knowledge Base for query planning + answer synthesis). | Create in the [Foundry portal](https://ai.azure.com); deploy `text-embedding-3-large` + a supported chat model (e.g. `gpt-5-mini`). |
| &nbsp;&nbsp;**Azure OpenAI account** | Same role as the Foundry project; older surface. | `az cognitiveservices account create -n <name> -g <rg> --kind OpenAI --sku S0 -l <region>`, then deploy embedding + chat models. |

#### Required only when a deployment has `target: azure`

If **at least one** of Q-MCP or Q-Ingest will run in Azure (the typical setup for Q-MCP):

| Resource | Why Quelch needs it | Create with |
|---|---|---|
| **Container Apps environment** | Hosts the Q-MCP / Q-Ingest Container Apps. | `az containerapp env create -n <name> -g <rg> -l <region>` (also creates a Log Analytics Workspace). |
| **Application Insights** | Telemetry destination for the Container Apps. | `az monitor app-insights component create --app <name> -g <rg> -l <region>` |
| **Key Vault** | Holds the Q-MCP API key and (if Q-Ingest runs in Azure) the Jira / Confluence credentials. The Container App reads them via managed identity. | `az keyvault create -n <globally-unique-name> -g <rg> -l <region>` |

For **Q-Ingest running on-prem**, none of the three above apply — secrets live wherever your on-prem secret store does (env var / `.env` / k8s `Secret` / HashiCorp Vault / etc.) and `quelch generate-deployment` writes scaffolding for whichever supervisor you're using. See [deployment.md "Hybrid topology"](deployment.md#hybrid-topology).

**Supported chat models** (per the Azure AI Search 2025-11-01-preview): `gpt-4o`, `gpt-4o-mini`, `gpt-4.1`, `gpt-4.1-nano`, `gpt-4.1-mini`, `gpt-5`, `gpt-5-nano`, `gpt-5-mini`. **Recommended: `gpt-5-mini`** — newer than the 4.1 family, in Microsoft's portal-validated subset, similar cost/latency tier. Use `gpt-5` if you need higher answer-synthesis quality and can absorb the cost; use `gpt-5-nano` for lowest latency when query complexity is moderate.

**Recommended embedding model**: `text-embedding-3-large` (3072 dims).

### Source credentials

For whichever sources you'll ingest:

- **Jira Cloud**: an Atlassian email + API token ([generate one here](https://id.atlassian.com/manage-profile/security/api-tokens))
- **Jira Data Center / Server**: a Personal Access Token from your Jira admin
- **Confluence Cloud / DC**: same as Jira (often the same token)

### Verify before continuing

```bash
az resource list -g <your-rg> -o table
```

You should see your Cosmos account, AI Search service, AI provider, ACA environment, App Insights, and Key Vault. `quelch init` and `quelch validate` will check this same list and tell you exactly what's missing.

---

## 1. Initialise the config

```bash
mkdir -p ~/work/my-quelch && cd ~/work/my-quelch
quelch init
```

The wizard:

- Calls `az` to discover your subscriptions and resource groups.
- Asks which subscription / resource group / region to use.
- Asks whether your model deployments live in **Microsoft Foundry** or **Azure OpenAI**, lists the existing accounts/projects of that kind in the chosen RG, and lets you pick. If none are found it prints the `az` command you need.
- Lists the **embedding** and **chat** deployments inside the selected provider so you can pick from supported models, and asks for retrieval reasoning effort + output mode for the Knowledge Base.
- Asks for source connections one at a time (Jira first, then Confluence). For each it offers to test the credentials by hitting `/rest/api/2/myself`.
- Asks for the deployment shape — for a first run pick **"single ingest + MCP, both in Azure"**.
- Runs a final **prerequisite check** against `az` and prints a per-resource ✓/✗/? report. Missing items get a copy-pasteable `az` create command; "?" means `az` couldn't determine status (not signed in, transient, etc.).

The wizard writes a `quelch.yaml` in the current directory.

If you already have credentials in environment variables (recommended for repeatability), the wizard will reference them with `${VAR}` placeholders rather than baking them in.

> **Don't want the wizard?** Use a template:
>
> ```bash
> quelch init --non-interactive --from-template minimal
> ```
>
> Available templates: `minimal`, `multi-source`, `distributed`. Edit `quelch.yaml` afterwards to fill in subscription / region / credentials.

---

## 2. Set environment variables for credentials

Quelch's loader substitutes `${VAR}` references at runtime. Set them in your shell:

```bash
# Jira Cloud:
export JIRA_EMAIL="you@example.com"
export JIRA_API_TOKEN="..."

# Jira Data Center:
export JIRA_PAT="..."

# Confluence Cloud (often the same token as Jira Cloud):
export CONFLUENCE_EMAIL="you@example.com"
export CONFLUENCE_API_TOKEN="..."

# Confluence DC:
export CONFLUENCE_PAT="..."

# Azure subscription (sometimes used in azure: section):
export AZURE_SUBSCRIPTION_ID="$(az account show --query id -o tsv)"
```

Quelch reads these at deploy time and writes them into Azure Key Vault for the Container App workers to consume — your shell env vars never end up directly in the deployed container.

---

## 3. Validate the config

```bash
quelch validate
```

Sanity check: are env vars set, are deployments disjoint, do exposed data sources exist? Exit-code 0 means good.

```bash
quelch effective-config ingest
```

Optional but recommended: prints the *sliced* config that the `ingest` deployment will see. Useful for confirming the right credentials and source connections are in scope.

---

## 4. Plan the Azure changes

```bash
quelch azure plan
```

Quelch generates Bicep into `.quelch/azure/` and `rigg/` files into `rigg/` from your `quelch.yaml`, then runs `az deployment group what-if` and `rigg diff` against your live Azure to show exactly what will change.

The output lists:

- Bicep changes — the Cosmos database + containers (created inside your existing Cosmos account), one user-assigned managed identity per deployment, role assignments on your existing resources, and the Container App that runs the MCP / ingest worker. **Quelch does not create the Cosmos account, AI Search service, Key Vault, ACA environment, App Insights, or AI provider** — they're referenced via `existing` in the generated Bicep.
- rigg changes — new indexes, indexers, skillsets, Knowledge Sources, and the Knowledge Base used for Agentic Retrieval (with both the embedding deployment wired into the vectorizer and the chat deployment wired into `models[]`).

Read the diff. Both `.quelch/azure/` and `rigg/` should be committed to your config repo so they're reviewable in PRs alongside `quelch.yaml`.

---

## 5. Deploy

```bash
quelch azure deploy
```

Same as `plan` but actually applies the changes. It prompts before doing anything destructive. Use `--yes` in CI.

Steps Quelch runs internally:

1. `az deployment group what-if` (preview) → show the diff again.
2. Prompt for confirmation.
3. `az deployment group create` to apply Bicep.
4. `rigg push` (via the embedded library) to apply the AI Search side.
5. Save a `last.json` snapshot at `.quelch/azure/<deployment>.last.json` so you can review after the fact.

**Expected duration:** 5–15 minutes for the first deployment (Cosmos accounts and AI Search services take a while to provision). Subsequent deploys are seconds.

---

## 6. Wait for the first ingest cycle

Once the deploy completes, the ingest Container App starts on its own. Watch progress:

```bash
quelch status
```

Reads the `quelch-meta` Cosmos container and shows last-sync time, doc count, and state per `(source, subsource)` triple.

Add `--tui` for the live fleet dashboard:

```bash
quelch status --tui
```

The first cycle does a **full backfill** — for a typical Jira project this is 1–10 minutes; for a 10k-issue project it can take longer. Quelch advances the cursor incrementally and persists progress, so a worker restart picks up where it left off.

You can also tail logs from the running Container App:

```bash
quelch azure logs ingest --follow
```

(Replace `ingest` with whatever your ingest deployment is named in `quelch.yaml`.)

Once `quelch status` shows non-zero `documents_synced_total` for at least one source, your data is in Cosmos. Within ~15 minutes (the default Indexer cadence), it'll also be in the AI Search index and queryable through the Knowledge Base.

---

## 7. Test the MCP server directly

The MCP server is now running at the URL you'll find in Azure portal under your Container App's *Application Url* (also visible in `az containerapp show --query properties.configuration.ingress.fqdn`).

Quelch does not generate the API key for you — you set it in Key Vault before (or after) the first deploy. Pick a value, store it, point the Container App at it:

```bash
RG=<your-resource-group>
KV=<your-key-vault-name>          # e.g. quelch-prod-kv
APP=<your-mcp-container-app-name> # e.g. quelch-prod-mcp

# 1) Generate a key (any high-entropy value works):
NEW_KEY=$(openssl rand -base64 32)

# 2) Store it in Key Vault under the canonical name `quelch-mcp-api-key`:
az keyvault secret set --vault-name "$KV" --name quelch-mcp-api-key --value "$NEW_KEY"

# 3) Restart the Container App revision so it picks up the new secret value:
az containerapp revision restart -g "$RG" -n "$APP" \
  --revision $(az containerapp revision list -g "$RG" -n "$APP" \
                 --query "[?properties.active].name | [0]" -o tsv)

# 4) Read it back to call Q-MCP from your shell:
QUELCH_MCP_API_KEY=$(az keyvault secret show --vault-name "$KV" --name quelch-mcp-api-key --query value -o tsv)
MCP_URL=$(az containerapp show -n "$APP" -g "$RG" --query properties.configuration.ingress.fqdn -o tsv)

# 5) List available data sources (round-trip Q-MCP connectivity check):
curl -X POST "https://$MCP_URL/mcp" \
  -H "Authorization: Bearer $QUELCH_MCP_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```

To **rotate** later: re-run steps 1–3. See [mcp-api.md "Setting and rotating the API key"](mcp-api.md#current--api-key) for the on-prem equivalents.

If you see a JSON-RPC response listing five tools (`search`, `query`, `get`, `list_sources`, `aggregate`) you're connected.

---

## 8. Connect an agent

`quelch agent generate` produces a copy-pasteable bundle for the platform you use. Generate one and follow its `README.md`:

```bash
# For Microsoft 365 Copilot Studio:
quelch agent generate --target copilot-studio --output ./bundle-copilot-studio

# For Claude Code (drops a project-local skill):
quelch agent generate --target claude-code --output ./bundle-claude

# For GitHub Copilot CLI / VS Code Copilot Chat / OpenAI Codex:
quelch agent generate --target copilot-cli      --output ./bundle-gh
quelch agent generate --target vscode-copilot   --output ./bundle-vscode
quelch agent generate --target codex            --output ./bundle-codex

# Generic markdown (paste anywhere):
quelch agent generate --target markdown --output ./bundle-md
```

Each bundle's `README.md` walks through the platform-specific install. Common pattern:

1. Copy the included `.mcp.json` (or platform-equivalent) into your IDE / agent config.
2. Set `QUELCH_API_KEY` in your shell to the value you fetched from Key Vault.
3. The bundle's main file (`SKILL.md` for Claude Code, `agent-instructions.md` for Copilot Studio, `AGENTS.md` for Codex, `copilot-instructions.md` for VS Code Copilot) goes into the agent's instructions slot.

Now ask your agent something like:

> *"How many open Jira issues are assigned to me?"*

The agent will call `query(data_source: "jira_issues", where: {...})` against your MCP server and return the answer. See [examples.md](examples.md) for 17 worked walkthroughs.

---

## 9. Day-2 operations

Once it's deployed, the things you'll most often do:

- **Check sync progress**: `quelch status` (or `--tui`).
- **Reset a stuck cursor**: `quelch reset --source jira-cloud --subsource DO`.
- **Trigger an indexer run** (if the AI Search index is stale): `quelch azure indexer run jira-issues`.
- **Tail logs**: `quelch azure logs <deployment>`.
- **Pull portal-side changes** (if someone edited an index in the Azure portal): `quelch azure pull` brings the live state into local `rigg/` files for review.
- **Refresh agent bundles** after config changes: re-run `quelch agent generate`, diff, commit.
- **Roll forward**: `brew upgrade quelch && quelch azure deploy` — Container Apps swap to the new image with a rolling revision.

For the full operator command surface, see [cli.md](cli.md).

---

## Try it offline first with `quelch dev`

If you want to evaluate Quelch *before* committing to the Azure provisioning, the `quelch dev` mode runs the simulator, an in-memory Cosmos backend, the ingest worker, and the MCP server — all in one process, no Azure account required.

```bash
quelch dev
```

This:

- Spawns mock Jira and Confluence HTTP servers fed by the activity simulator.
- Runs an ingest worker against those mocks.
- Exposes a local MCP server on `127.0.0.1:8080`.
- Renders the fleet-dashboard TUI.

You can point a local agent at `http://127.0.0.1:8080/mcp` and exercise the same tool calls you'd make against a deployed instance. Press `q` in the TUI to quit.

Useful flags:

- `--no-tui` — disable the dashboard and emit structured logs to stdout instead (global flag).
- `--mcp-port 9000` — bind the MCP server elsewhere (default `8080`).
- `--seed 42` — deterministic simulator output for reproducible runs.
- `--rate-multiplier 5.0` — speed up simulated activity for quicker exercises.
- `--use-cosmos-emulator` — point at the local Cosmos DB emulator instead of the in-memory backend.

This is the recommended way to **first** experience Quelch before paying for any Azure resources.

---

## Where to next

- **Multi-source / distributed setups** — see [deployment.md](deployment.md) "Hybrid topology".
- **On-prem ingest** for sources behind a firewall — see [deployment.md](deployment.md#on-premises-deployment).
- **Configuration reference** — every section of `quelch.yaml` is documented in [configuration.md](configuration.md).
- **MCP API reference** for agent authors — [mcp-api.md](mcp-api.md).
- **Real-question walkthroughs** — [examples.md](examples.md) shows how an agent uses each MCP tool to answer concrete user questions.
- **Sync correctness deep-dive** if you're debugging anything sync-related — [sync.md](sync.md).

If you hit something that doesn't work, the first thing to check is `quelch validate` and `quelch azure plan`. If those are happy and ingest still isn't moving, `quelch azure logs <deployment> --follow` shows what the worker is actually doing.
