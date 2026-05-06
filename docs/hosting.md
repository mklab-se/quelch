# Hosting Q-Ingest and Q-MCP

This document is a recipe book for running **Quelch Ingest** (Q-Ingest) and **Quelch MCP** (Q-MCP) processes on your infrastructure of choice. Pick the platform you already use; the snippets are copy-pasteable starting points.

## What Quelch generates

**Quelch generates exactly one thing on the hosting side: a per-instance config file.**

```bash
quelch instance config <name> --kind ingest|mcp --output <path>
```

Quelch does **not** publish a container image, generate `docker-compose.yaml` files, write systemd units, emit Kubernetes manifests, scaffold Container Apps, produce Bicep, or anything else. The snippets below are **examples** — copy them, adapt them, or use whatever shape your environment already standardises on.

The same `quelch` binary runs both Q-Ingest (`quelch ingest --config ...`) and Q-MCP (`quelch mcp --config ...`); the command is what selects the role.

The per-instance config references credentials via env-var placeholders (`${QUELCH_MCP_API_KEY}`, `${JIRA_PAT_X}`, etc.). You wire those env vars into the host's secret store; Quelch never touches secret material on your behalf.

---

## Where the binary comes from

Three install paths, depending on host:

- **Released binary** (recommended for systemd / VMs) — download the matching tarball from [GitHub Releases](https://github.com/mklab-se/quelch/releases), drop the `quelch` binary into `/usr/local/bin`.
- **`cargo install quelch`** — works anywhere with a Rust toolchain. Slow on first build, fine for CI / dev / one-off VMs.
- **Container image you build yourself** — for Docker / Kubernetes / Container Apps. The repo ships a `Dockerfile` you build and push to your own registry (see below).

### Building a container image

If you're hosting Q-Ingest or Q-MCP in a container runtime, you build the image yourself. The repo's `Dockerfile` is a multi-stage build (Rust builder → distroless runtime) that produces a small static-ish image. Build and push:

```bash
git clone https://github.com/mklab-se/quelch.git
cd quelch
git checkout v<version>          # match your CLI version

docker build -t <your-registry>/quelch:<version> .
docker push    <your-registry>/quelch:<version>
```

`<your-registry>` is whatever you control: `myorg.azurecr.io`, `ghcr.io/myorg`, `docker.io/myorg`, a private registry behind a corporate proxy, etc. Pin the tag to the CLI version you're running on the operator side so the running binary and the image stay in lockstep.

The shipped `Dockerfile` is a starting point — adapt it (different base image, your org's TLS roots, your build cache strategy, multi-arch). All the snippets below assume `<your-registry>/quelch:<version>` is reachable from your host.

---

## What you supply

A typical Q-Ingest host needs:

- The per-instance config file (`q-ingest-X.yaml`) at a known path.
- Env vars for the source-system credentials referenced in that file (`JIRA_PAT_X`, `CONFLUENCE_PAT`, etc.).
- Network egress to the source system and to your Cosmos DB account.
- Azure credentials so the worker can authenticate to Cosmos. Three options:
  - **Workload Identity Federation / Managed Identity** — recommended where it's available (AKS workload identity, Container Apps managed identity, Azure VM managed identity). The `DefaultAzureCredential` chain picks it up automatically.
  - **Service-principal env vars** (`AZURE_CLIENT_ID` / `AZURE_TENANT_ID` / `AZURE_CLIENT_SECRET`) — for hosts where managed identity isn't available.
  - **Cosmos primary key** — if you don't want to set up Entra-side identity, skip the above and provide `AZURE_COSMOS_KEY` instead. Less granular, simpler to bootstrap.

A typical Q-MCP host needs all of the above (modulo source credentials, which Q-MCP doesn't use), plus:

- An env var for the MCP API key (typically `QUELCH_MCP_API_KEY`).
- Inbound HTTPS exposure on the chosen listen port — Q-MCP is a server, agents call it.

The Cosmos data plane needs the `Cosmos DB Built-in Data Contributor` role on the account; AI Search needs `Search Index Data Reader` (Q-MCP only — Q-Ingest doesn't touch Search).

---

## Running under Docker

Single-host Q-Ingest with `docker run` (assumes you've already built and pushed `<your-registry>/quelch:<version>` per "Building a container image" above):

```bash
docker run -d --name q-ingest-jira \
  --restart unless-stopped \
  -e JIRA_PAT_X="$JIRA_PAT_X" \
  -e AZURE_CLIENT_ID="..." \
  -e AZURE_TENANT_ID="..." \
  -e AZURE_CLIENT_SECRET="..." \
  -v "$PWD/q-ingest-jira.yaml:/etc/quelch/config.yaml:ro" \
  <your-registry>/quelch:<version> \
  ingest --config /etc/quelch/config.yaml
```

Or with `docker-compose.yaml`:

```yaml
version: "3.8"
services:
  q-ingest-jira:
    image: <your-registry>/quelch:<version>
    command: ingest --config /etc/quelch/config.yaml
    restart: unless-stopped
    environment:
      JIRA_PAT_X: ${JIRA_PAT_X}
      AZURE_CLIENT_ID: ${AZURE_CLIENT_ID}
      AZURE_TENANT_ID: ${AZURE_TENANT_ID}
      AZURE_CLIENT_SECRET: ${AZURE_CLIENT_SECRET}
    volumes:
      - ./q-ingest-jira.yaml:/etc/quelch/config.yaml:ro

  q-mcp:
    image: <your-registry>/quelch:<version>
    command: mcp --config /etc/quelch/config.yaml
    restart: unless-stopped
    ports:
      - "8080:8080"
    environment:
      QUELCH_MCP_API_KEY: ${QUELCH_MCP_API_KEY}
      AZURE_CLIENT_ID: ${AZURE_CLIENT_ID}
      AZURE_TENANT_ID: ${AZURE_TENANT_ID}
      AZURE_CLIENT_SECRET: ${AZURE_CLIENT_SECRET}
    volumes:
      - ./q-mcp.yaml:/etc/quelch/config.yaml:ro
```

`docker compose up -d`, `docker compose logs -f`, done.

---

## Running under systemd

No image needed — install the released `quelch` binary directly. Either download the platform tarball from [GitHub Releases](https://github.com/mklab-se/quelch/releases) and drop the binary into `/usr/local/bin/quelch`, or `cargo install quelch` on the host.

Unit file (`/etc/systemd/system/q-ingest-jira.service`):

```ini
[Unit]
Description=Quelch Ingest (jira-internal)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=quelch
Group=quelch
EnvironmentFile=/etc/quelch/q-ingest-jira.env
ExecStart=/usr/local/bin/quelch ingest --config /etc/quelch/q-ingest-jira.yaml
Restart=on-failure
RestartSec=10s

[Install]
WantedBy=multi-user.target
```

Env file (`/etc/quelch/q-ingest-jira.env`, mode `0600`):

```ini
JIRA_PAT_X=...
AZURE_CLIENT_ID=...
AZURE_TENANT_ID=...
AZURE_CLIENT_SECRET=...
```

Install:

```bash
sudo install -d /etc/quelch
sudo install -m 0644 q-ingest-jira.yaml /etc/quelch/
sudo install -m 0600 -o quelch -g quelch q-ingest-jira.env /etc/quelch/
sudo install -m 0644 q-ingest-jira.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now q-ingest-jira.service
```

Logs: `journalctl -u q-ingest-jira -f`. The same shape works for Q-MCP — change `ingest` to `mcp` in `ExecStart`, add `QUELCH_MCP_API_KEY` to the env file, and open the listen port in your firewall.

---

## Running under Kubernetes

`Deployment` + `ConfigMap` + `Secret` is the canonical layout. Per-instance config goes in a ConfigMap; credentials go in a Secret. Image is whatever you built and pushed in "Building a container image".

```yaml
# Per-instance config — emitted by `quelch instance config ...`.
apiVersion: v1
kind: ConfigMap
metadata:
  name: q-ingest-jira-config
data:
  config.yaml: |
    azure:
      cosmos:
        endpoint: https://my-cosmos.documents.azure.com
        database: quelch
        containers:
          jira_issues: jira-issues
          jira_sprints: jira-sprints
          jira_fix_versions: jira-fix-versions
          jira_projects: jira-projects
        meta_container: quelch-meta
    source_connections:
      - name: jira-internal-pat-x
        type: jira
        base_url: https://jira.internal/
        auth: { kind: pat, token: ${JIRA_PAT_X} }
        projects: [DO, ANNA]
    instances:
      - name: ingest-jira-internal
        kind: ingest
        connections: [jira-internal-pat-x]
        cycle_interval: 5m

---
apiVersion: v1
kind: Secret
metadata:
  name: q-ingest-jira-secrets
type: Opaque
stringData:
  JIRA_PAT_X: "..."
  AZURE_CLIENT_ID: "..."
  AZURE_TENANT_ID: "..."
  AZURE_CLIENT_SECRET: "..."

---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: q-ingest-jira
spec:
  replicas: 1
  selector:
    matchLabels: { app: q-ingest-jira }
  template:
    metadata:
      labels: { app: q-ingest-jira }
    spec:
      containers:
        - name: quelch
          image: <your-registry>/quelch:<version>
          args: ["ingest", "--config", "/etc/quelch/config.yaml"]
          envFrom:
            - secretRef: { name: q-ingest-jira-secrets }
          volumeMounts:
            - name: config
              mountPath: /etc/quelch
              readOnly: true
      volumes:
        - name: config
          configMap:
            name: q-ingest-jira-config
```

For Q-MCP add a `Service` exposing port 8080 plus your usual ingress (`Ingress`, `Gateway`, `Service type=LoadBalancer`, etc.). For Azure-native authentication, prefer **Azure AD workload identity** (`azure.workload.identity/use: "true"` annotation on the ServiceAccount) over the service-principal env vars.

Replicas should be `1` for ingest instances — Q-Ingest's cursor-ownership check refuses concurrent writes from a different instance. Q-MCP is read-only and can scale horizontally.

---

## Running as an Azure Container App

Q-MCP is a natural fit for Container Apps — long-running HTTP server, scale-to-zero, public ingress. Q-Ingest works equally well there for Atlassian Cloud sources reachable from Azure. Image is whatever you built and pushed in "Building a container image" (typically pushed to an Azure Container Registry in the same subscription).

```bash
RG=rg-quelch-prod
ACAE=quelch-cae
APP=q-mcp-prod
IMAGE=<your-registry>/quelch:<version>

# Create the Container Apps environment first if you don't already have one:
# az containerapp env create -n "$ACAE" -g "$RG" -l swedencentral

# Store the per-instance config as an ACA secret (multi-line value).
az containerapp create \
  -n "$APP" -g "$RG" \
  --environment "$ACAE" \
  --image "$IMAGE" \
  --command "/usr/local/bin/quelch" \
  --args "mcp --config /etc/quelch/config.yaml" \
  --ingress external --target-port 8080 \
  --secrets \
      mcp-api-key=<value> \
      config-yaml=@q-mcp.yaml \
  --env-vars \
      QUELCH_MCP_API_KEY=secretref:mcp-api-key \
      QUELCH_CONFIG=secretref:config-yaml \
  --system-assigned                               # managed identity for Cosmos / Search RBAC
```

If your image lives in a private Azure Container Registry, also pass `--registry-server <acr>.azurecr.io --registry-identity system` (or `--registry-username/--registry-password` for non-managed-identity flows).

The `QUELCH_CONFIG` pattern (config in an env var rather than a file) is one option — Container Apps secrets max out at 64 KiB which is fine for a per-instance file. Alternatively mount it via a volume:

```bash
# Volume-based:
--secrets config-yaml=@q-mcp.yaml \
--secret-volumes config:config-volume \
--volume-mounts config-volume:/etc/quelch
```

Grant the Container App's managed identity the appropriate roles on Cosmos / AI Search:

```bash
PRINCIPAL_ID=$(az containerapp show -n "$APP" -g "$RG" \
  --query identity.principalId -o tsv)

# Cosmos data plane:
az cosmosdb sql role assignment create \
  --account-name my-cosmos -g "$RG" \
  --role-definition-id 00000000-0000-0000-0000-000000000002 \
  --principal-id "$PRINCIPAL_ID" \
  --scope "/"

# AI Search admin (Q-MCP only needs read; granting reader is enough):
az role assignment create \
  --role "Search Index Data Reader" \
  --assignee "$PRINCIPAL_ID" \
  --scope "/subscriptions/.../providers/Microsoft.Search/searchServices/my-search"
```

This is illustrative. Standardise on whatever role-assignment pattern the rest of your tenant uses.

---

## Generating and rotating the Q-MCP API key

Full walk-through: [api-key.md](api-key.md). Short version:

- Generate: `openssl rand -base64 32`.
- Store in your host's secret store (Docker `.env`, k8s `Secret`, systemd `EnvironmentFile`, Container Apps secret, AKV reference, etc.).
- Reference it from the per-instance Q-MCP config as `api_key: ${QUELCH_MCP_API_KEY}`.
- Rotate by generating a new value, updating the secret store, restarting Q-MCP — there is no atomic two-key window today; rotation is an immediate cutover.

---

## Putting it together

Most installations end up with:

- **Q-MCP** in Azure Container Apps (long-running HTTPS service; scale-to-zero saves money during idle hours).
- **Q-Ingest for Atlassian Cloud sources** in Container Apps too, alongside Q-MCP.
- **Q-Ingest for Jira / Confluence Data Center** on-prem (Docker, systemd, or k8s — wherever the Data Center installs are reachable from).

All instances write / read the same Cosmos account; Quelch's static conflict prevention (at `quelch validate`) plus the dynamic cursor-ownership check (every cursor in `quelch-meta` carries an `owner_instance` field) keep them from stepping on each other.

For monitoring across hosts, `quelch status [--tui]` reads `quelch-meta` directly and shows live state regardless of where each worker actually runs.

---

## Troubleshooting

**Indexer reports zero documents.**
Check ingest first: `quelch status --instance <name>`. If `documents_synced_total > 0` but the AI Search index is empty, the indexer hasn't run yet — `quelch azure indexer run <name>` to kick it. If `documents_synced_total == 0`, check the host's own logs (`docker logs`, `journalctl`, `kubectl logs`, Container Apps log streaming).

**Cursor ownership refused.**
Q-Ingest aborts with a `cursor owner mismatch` error when a different instance already owns the cursor it's about to write. Either you have two instances accidentally pointing at the same source slice (fix the YAML), or you intentionally renamed the instance and need ownership transfer (`quelch reset --instance NEW_NAME --source ... --subsource ... --take-ownership`).

**MCP returns `403 forbidden`.**
The data source isn't in the MCP instance's `expose:` list. Update `quelch.yaml`, run `quelch azure apply`, and re-emit the per-instance Q-MCP config.

**Drift in `quelch azure plan`.**
Someone edited an index / indexer / KB through the portal. Decide: if the portal edit is correct, fold it into `quelch.yaml`; if not, `quelch azure apply` reverts to the YAML-defined state. Quelch and the standalone `rigg` tool are mutually exclusive per resource — if you need fine-grained manual control over a specific index, remove it from `quelch.yaml` and manage it with `rigg` separately.
