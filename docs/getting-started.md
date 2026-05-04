# Getting started

This walkthrough sets up a working Quelch deployment end-to-end: a Cosmos-DB-backed knowledge platform fed from your Jira and Confluence, with an MCP server that an agent (Copilot Studio, Claude Code, VS Code Copilot, GitHub Copilot CLI, or OpenAI Codex) can connect to.

It's the **happy path** for a single environment. For multi-environment, on-prem ingest, distributed deployments, and other shapes, see [deployment.md](deployment.md).

If you just want to **evaluate Quelch locally** without touching Azure or your real source systems, skip ahead to [Try it offline first with `quelch dev`](#try-it-offline-first-with-quelch-dev).

---

## 0. Prerequisites

Before you start, make sure you have:

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
- **At least Contributor on the Azure subscription** (or the resource group you'll deploy into). Owner is needed if you want Quelch to also create the role assignments that let Container Apps read Cosmos / AI Search / Key Vault. If you only have Contributor, set `azure.skip_role_assignments: true` in the config and have a subscription owner apply the role assignments manually (Quelch emits a script for this).
- **An existing Azure OpenAI account** with an embedding model deployment. `text-embedding-3-large` (3072 dimensions) is the recommended default. Quelch will *not* provision the OpenAI account itself — capacity quotas make that fragile to script.
- **Source credentials** for whichever sources you'll ingest:
  - **Jira Cloud**: an Atlassian email + API token ([generate one here](https://id.atlassian.com/manage-profile/security/api-tokens))
  - **Jira Data Center / Server**: a Personal Access Token from your Jira admin
  - **Confluence Cloud / DC**: same as Jira (often the same token)
- **A Git repository** to commit `quelch.yaml`, the generated `.quelch/` and `rigg/` directories. Treat the config as code.

That's all. You don't need to pre-create the Cosmos DB account, the AI Search service, the Key Vault, or the Container Apps environment — Quelch will provision them.

---

## 1. Initialise the config

```bash
mkdir -p ~/work/my-quelch && cd ~/work/my-quelch
quelch init
```

The wizard:

- Calls `az` to discover the subscriptions, resource groups, Cosmos accounts, AI Search services, and Azure OpenAI accounts you already have.
- Asks which subscription / resource group / region to use.
- Asks for source connections one at a time (Jira first, then Confluence). For each it will offer to test the credentials by hitting `/rest/api/2/myself`.
- Asks for the deployment shape — for a first run pick **"single ingest + MCP, both in Azure"**.

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

- Bicep changes (new Cosmos account, new AI Search service, new Container Apps, role assignments, …)
- rigg changes (new indexes, indexers, skillsets, Knowledge Sources, the Knowledge Base used for Agentic Retrieval)

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
quelch azure logs ingest-azure --follow
```

Once `quelch status` shows non-zero `documents_synced_total` for at least one source, your data is in Cosmos. Within ~15 minutes (the default Indexer cadence), it'll also be in the AI Search index and queryable through the Knowledge Base.

---

## 7. Test the MCP server directly

The MCP server is now running at the URL you'll find in Azure portal under your Container App's *Application Url* (also visible in `az containerapp show --query properties.configuration.ingress.fqdn`).

The MCP API key was auto-generated and stored in Key Vault:

```bash
KEY_VAULT_NAME=$(az keyvault list -g "$(grep -m1 resource_group quelch.yaml | awk '{print $2}' | tr -d \")" --query "[0].name" -o tsv)
QUELCH_MCP_API_KEY=$(az keyvault secret show --vault-name "$KEY_VAULT_NAME" --name mcp-api-key --query value -o tsv)
MCP_URL=$(az containerapp show -n "$(quelch effective-config mcp-azure | grep -A1 '^deployments:' | tail -1 | awk '{print $3}')" -g "$(grep -m1 resource_group quelch.yaml | awk '{print $2}' | tr -d \")" --query properties.configuration.ingress.fqdn -o tsv)

# List available data sources (round-trip MCP connectivity check)
curl -X POST "https://$MCP_URL/mcp" \
  -H "Authorization: Bearer $QUELCH_MCP_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```

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

- `--no-tui` — log to stdout instead of the dashboard.
- `--mcp-port 9000` — bind the MCP server elsewhere.
- `--seed 42` — deterministic simulator output for reproducible runs.

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
