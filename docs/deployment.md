# Deployment

Quelch deploys itself in two flavours:

- **Azure** — Quelch CLI provisions and reconciles all the Azure resources (Cosmos DB, AI Search, Container Apps for ingest and MCP, Key Vault for secrets). This is fully automated.
- **On-premises** — Quelch generates ready-to-run artefacts (Docker compose, systemd unit, K8s manifests). You copy them to your host, supply credentials, and run them. Quelch does not touch on-prem infrastructure itself.

## Azure deployment

### Mental model

```
quelch.yaml                          ─┐
.quelch/azure/<deployment>.bicep    ─┼─►  az deployment group create  ─►  Azure
.quelch/azure/<deployment>.last.json ─┘    (or what-if for plan)
```

The full config is the source of truth. Quelch generates Bicep from it. The Bicep files are committed to your config repo so they're reviewable in PRs and auditable in version control. They are **generated output** — never hand-edited; the next `quelch azure plan` regenerates them.

### Two commands: `plan` and `deploy`

`quelch azure plan` answers "what would change". `quelch azure deploy` answers "make it so".

Both commands:

1. Load `quelch.yaml`, validate.
2. Compute the **effective config** for each deployment.
3. Synthesise Bicep at `.quelch/azure/<deployment>.bicep`.
4. Run `az deployment group what-if --resource-group <rg> --template-file <bicep>`.
5. Display the diff.

`deploy` adds:

6. Prompt for confirmation (skip with `--yes`).
7. Run `az deployment group create` with the same template.
8. Wait for completion.
9. Save a `last.json` snapshot for later diffing.

### Reading a `what-if` diff

Quelch translates the raw `what-if` JSON into a human-readable diff:

```
Plan for deployment 'mcp-azure'
───────────────────────────────────────────────────────────
+ Microsoft.App/containerApps/quelch-prod-mcp           (Create)
    image: ghcr.io/mklab-se/quelch:0.9.0
    cpu: 1.0    memory: 2.0Gi
    min_replicas: 0    max_replicas: 5

~ Microsoft.DocumentDB/databaseAccounts/quelch-prod-cosmos
    throughput.mode:  serverless → provisioned
    throughput.ru:    -          → 1000

= Microsoft.Search/searchServices/quelch-prod-search    (Unchanged)

3 changes pending. Continue?  [y/N]
```

`+` means create, `~` means modify, `-` means delete, `=` means unchanged. Drift caused by manual portal edits surfaces as `~` — applying reconciles to the config.

### What gets created

A typical `quelch.yaml` materialises into:

| Azure resource | Purpose | Created when |
|---|---|---|
| Resource group | Holds everything | If missing |
| Cosmos DB account + database | Document store | First deploy |
| Cosmos containers (`jira-issues`, `confluence-pages`, `quelch-meta`, ...) | Per `cosmos.containers` and source overrides | First deploy |
| Azure AI Search service | Search backend | First deploy |
| AI Search Indexers + Indexes + Skillsets | One per Cosmos container that has a deployed `mcp` exposing it | First deploy |
| Azure OpenAI account + embedding deployment | Vectoriser model | Optional — you typically point at an existing AOAI |
| Key Vault | Stores MCP API key, source credentials | First deploy |
| Managed identities | One per Container App, with role assignments to Cosmos / AI Search / OpenAI / Key Vault | Per deployment |
| Container Apps environment | Hosts ingest and mcp deployments | First deploy |
| Container Apps | One per deployment with `target: azure` | Per deployment |

### Container image

Container Apps run a single image: `ghcr.io/mklab-se/quelch:<version>`. The version always matches the CLI doing the deploy — `quelch 0.9.0 azure deploy` writes `image: ghcr.io/mklab-se/quelch:0.9.0` into the Bicep. This rules out operator/worker version skew.

Different startup commands distinguish the role:

- `ingest` Container Apps run `quelch ingest --deployment <name>`.
- `mcp` Container Apps run `quelch mcp --deployment <name>`.

The sliced effective config is mounted as a Container Apps secret and read on startup.

### Operating a deployed worker

```bash
quelch azure logs ingest-azure-cloud --follow
quelch azure logs mcp-azure --since 1h
```

```bash
quelch azure indexer status
quelch azure indexer run jira-issues          # trigger an immediate run
quelch azure indexer reset jira-issues        # force a full re-index next cycle
```

```bash
quelch status                      # all deployments, from quelch-meta
quelch status --deployment ingest-azure-cloud
quelch status --tui                # live dashboard
```

### Rolling forward

Every CLI release ships a matching container image. To roll all deployed workers forward:

```bash
brew upgrade quelch              # or cargo install --force quelch
quelch azure plan                 # shows that all images will move to the new tag
quelch azure deploy
```

Container Apps perform a rolling revision swap; ingest workers pick up the new image with no data loss because cursors are durable in `quelch-meta`.

### Destroying a deployment

```bash
quelch azure destroy ingest-azure-cloud
```

Removes the Container App and its revisions. Leaves Cosmos DB, AI Search, OpenAI, and shared infrastructure alone.

To delete everything in the resource group, use `az group delete --name <rg>` directly. Quelch does not offer a "nuke everything" command on purpose.

## On-premises deployment

Quelch ingest is the role you typically deploy on-prem — when the source system (Jira Data Center, Confluence Server) lives behind a corporate firewall and Azure can't reach it.

### Generating artefacts

```bash
quelch generate-deployment ingest-onprem-jira-ak --target docker --output ./deploy/ingest-onprem-jira-ak
```

Targets:

- `docker` — `docker-compose.yaml` plus `.env.example`.
- `systemd` — a unit file plus an EnvironmentFile template.
- `k8s` — a `Deployment`, a `ConfigMap` for the sliced config, a `Secret` template for credentials, and an optional Helm chart.

Every generated directory contains:

| File | Purpose |
|---|---|
| The artefact for the chosen target | docker-compose / systemd / k8s manifests |
| `effective-config.yaml` | The sliced config baked into the worker |
| `.env.example` | Every env var the worker needs (credentials, Cosmos endpoint, ...) |
| `README.md` | The three commands you run on the host |

### Running on the host

For Docker:

```bash
cd deploy/ingest-onprem-jira-ak
cp .env.example .env
# edit .env, fill in JIRA_INTERNAL_PAT, COSMOS_ENDPOINT, COSMOS_KEY, ...
docker compose up -d
```

For systemd:

```bash
sudo cp quelch-ingest-onprem-jira-ak.service /etc/systemd/system/
sudo cp quelch-ingest-onprem-jira-ak.env /etc/quelch/
# edit /etc/quelch/quelch-ingest-onprem-jira-ak.env
sudo systemctl daemon-reload
sudo systemctl enable --now quelch-ingest-onprem-jira-ak.service
```

For K8s:

```bash
kubectl apply -k deploy/ingest-onprem-jira-ak/
```

### Security

The on-prem worker needs:

- Outbound HTTPS to the Cosmos DB account (and to `quelch-meta` for cursor state).
- Outbound HTTPS or VPN to its source system.

It does **not** need inbound connectivity. There is no port for it to listen on; ingest is pull-only.

If your network policy only permits egress to a corporate proxy, set `HTTPS_PROXY` in the env file — Quelch's `reqwest` client honours it.

### Updating

There is no `quelch update` for on-prem workers. The way to update is:

1. `quelch generate-deployment ...` with a newer Quelch CLI.
2. Diff the new artefact against the deployed one (`git diff`).
3. Re-deploy via the platform-native mechanism (`docker compose pull && up -d`, `systemctl restart`, `kubectl apply -f`).

The new worker reads its cursor from `quelch-meta` on startup; no data is lost.

## Hybrid topology — typical setup

Real installations almost always combine both:

```
On-prem hosts                           Azure
─────────────                           ──────────────────────
quelch ingest (Jira DC, Confluence DC)  ─►  Cosmos DB
                                              │
                                              ▼
                                        AI Search Indexer
                                              │
                                              ▼
                                        AI Search Index
                                              ▲
                                              │
quelch ingest (Jira Cloud, Conf Cloud)  ─►  Cosmos DB
                                              │
                                              ▼
                                        Quelch MCP (Container App)
                                              ▲
                                              │ Streamable HTTP
                                              │
                                       Copilot Studio agents
```

One config file describes both halves. `quelch azure deploy` handles the cloud half. `quelch generate-deployment` plus a `git pull` + `docker compose up -d` on each host handles the on-prem half.

## Troubleshooting

### Indexer reports zero documents

Check ingest first:

```bash
quelch status --deployment ingest-onprem-jira-ak
```

If `documents_synced` is non-zero but the AI Search index is empty, the Indexer hasn't run yet:

```bash
quelch azure indexer status
quelch azure indexer run <indexer-name>
```

If `documents_synced` is zero, the worker isn't pulling. Check logs:

```bash
quelch azure logs ingest-onprem-jira-ak                  # azure deployments only
docker compose logs -f                                   # on-prem docker
journalctl -u quelch-ingest-onprem-jira-ak -f            # on-prem systemd
```

### `what-if` shows unexpected drift

Someone clicked something in the portal. Decide:

- If the portal change is correct → update `quelch.yaml` to match, `quelch azure plan` again, expect zero changes.
- If the portal change is wrong → `quelch azure deploy` will reconcile back to the config.

### MCP returns `403 Forbidden`

The container/index isn't in the deployment's `expose:` list. Update the config and redeploy.
