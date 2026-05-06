# Q-MCP API key

**Quelch MCP** (Q-MCP) authenticates incoming agent requests with an API key. The agent presents it as a bearer token; Q-MCP compares it to the value the host supplied via env var. If they match, the request proceeds; otherwise Q-MCP returns `unauthenticated`.

This document covers generating, storing, referencing, and rotating that key.

## Why an API key?

Q-MCP is a long-running HTTP server that exposes data from your Cosmos DB and AI Search account. Anyone who can reach it on the network needs to prove they're allowed to talk to it. Today that proof is a static API key in `Authorization: Bearer ...`. Microsoft Entra ID is on the roadmap; until it ships, the API key is the only auth method.

If `QUELCH_MCP_API_KEY` (or whatever env var the per-instance config references) is **not** set, Q-MCP runs in **unauthenticated dev mode** and accepts every request. That's fine for `quelch dev` and local testing; it is never what you want in production. Always set the env var on a real host.

## Generating a key

Any high-entropy random string works. The recommended one-liner:

```bash
openssl rand -base64 32
```

Output is 32 random bytes encoded as ~44 characters of base64. Use whatever your org standardises on if it's stronger; this is plenty by default.

## Where to put it

The key lives as an **env var on the host that runs Q-MCP**. The variable name is up to you — the per-instance Q-MCP config is what binds it. Typical name: `QUELCH_MCP_API_KEY`.

In `quelch.yaml` (and the per-instance Q-MCP config slice):

```yaml
instances:
  - name: mcp-prod
    kind: mcp
    expose: [...]
    api_key: ${QUELCH_MCP_API_KEY}      # env-var reference, never a literal
    knowledge_base: quelch-prod-kb
    listen: 0.0.0.0:8080
```

Quelch never writes the literal value to disk. The runtime resolves `${QUELCH_MCP_API_KEY}` at config-load time; missing env vars fail with a precise error from `quelch validate --config <file>`.

## Per-host how-tos

### Docker / docker-compose

> The image referenced below is one you build and push yourself from the repo's `Dockerfile`. See [hosting.md "Building a container image"](hosting.md#building-a-container-image).

```yaml
# docker-compose.yaml
services:
  q-mcp:
    image: <your-registry>/quelch:<version>
    command: mcp --config /etc/quelch/config.yaml
    environment:
      QUELCH_MCP_API_KEY: ${QUELCH_MCP_API_KEY}    # from your shell or .env
    # ...
```

Or with `docker run`:

```bash
docker run -d -e QUELCH_MCP_API_KEY="$(openssl rand -base64 32)" \
  <your-registry>/quelch:<version> \
  mcp --config /etc/quelch/config.yaml
```

### systemd

```ini
# /etc/quelch/q-mcp.env (mode 0600)
QUELCH_MCP_API_KEY=<paste value here>
```

```ini
# /etc/systemd/system/q-mcp.service
[Service]
EnvironmentFile=/etc/quelch/q-mcp.env
ExecStart=/usr/local/bin/quelch mcp --config /etc/quelch/q-mcp.yaml
```

### Kubernetes

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: q-mcp-secrets
type: Opaque
stringData:
  QUELCH_MCP_API_KEY: <paste value here>

---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: q-mcp
spec:
  template:
    spec:
      containers:
        - name: quelch
          image: <your-registry>/quelch:<version>
          args: ["mcp", "--config", "/etc/quelch/config.yaml"]
          envFrom:
            - secretRef: { name: q-mcp-secrets }
```

For tighter integration use a `SecretProviderClass` backed by Azure Key Vault / Vault / etc.; the worker only needs the env var to be set however your platform exposes secrets.

### Azure Container Apps

```bash
NEW_KEY=$(openssl rand -base64 32)

az containerapp secret set \
  -n q-mcp-prod -g rg-quelch-prod \
  --secrets mcp-api-key="$NEW_KEY"

az containerapp update \
  -n q-mcp-prod -g rg-quelch-prod \
  --set-env-vars QUELCH_MCP_API_KEY=secretref:mcp-api-key
```

For an Azure Key Vault reference instead of an inline secret value, use `--secrets mcp-api-key=keyvaultref:<vault-uri>,identityref:<managed-identity-id>` — the Container App's managed identity needs `Key Vault Secrets User` on the vault. Container Apps then re-fetches the secret on every revision; rotation in AKV propagates without manual env-var updates.

## Rotating the key

There is no atomic two-key window today. Rotation is an immediate cutover: the new value replaces the old, and any agent still presenting the old value gets `unauthenticated` until you update its config.

The procedure:

1. Generate a new value: `NEW_KEY=$(openssl rand -base64 32)`.
2. Update the secret store on the Q-MCP host (whichever of the platforms above applies).
3. Restart Q-MCP so it re-reads the env var:
   - Docker: `docker restart q-mcp` (or `docker compose up -d`).
   - systemd: `sudo systemctl restart q-mcp`.
   - k8s: `kubectl rollout restart deploy/q-mcp`.
   - Container Apps: `az containerapp revision restart` (or `--force-revision`).
4. Update every agent / client config that referenced the old value.

Rolling rotation (old key still works for a grace period) is on the roadmap; not implemented today.

## Configuring the agent side

Each `quelch agent generate` bundle includes a `.mcp.json` snippet that references the key as an env var on the agent host:

```jsonc
{
  "mcpServers": {
    "quelch": {
      "type": "streamable-http",
      "url": "https://q-mcp.example.com",
      "headers": {
        "Authorization": "Bearer ${QUELCH_API_KEY}"
      }
    }
  }
}
```

Set `QUELCH_API_KEY` on the agent host to the same value Q-MCP accepts. The bundle's README walks through the agent-platform-specific install (Copilot Studio, Claude Code, VS Code Copilot, etc.); see [agent-generation.md](agent-generation.md).
