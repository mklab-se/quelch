# Quelch — documentation

Quelch is a single Rust binary that turns your enterprise knowledge sources (Jira, Confluence, ...) into a queryable knowledge platform that AI agents can use.

It does this by being three things at once:

1. **Quelch Ingest** (Q-Ingest) — pulls data from sources and writes it to **Cosmos DB**. Typically runs close to its data source — that often means **on-prem** when Confluence / Jira Data Center isn't reachable from Azure.
2. **Quelch MCP** (Q-MCP) — a Streamable-HTTP MCP server that lets agents query the Cosmos data over a small, well-defined tool set, blending classical filtering (Cosmos DB) with semantic search (Azure AI Search). Typically runs in **Azure** (Container Apps).
3. **The operator CLI** (`quelch ...`) — references existing Azure resources from your config, deploys the Q-MCP and Q-Ingest workers that target Azure, manages the AI Search side via the embedded [rigg](https://github.com/mklab-se/rigg) library, runs the indexers, and generates the agent-side instructions you paste into Copilot Studio / VS Code / GitHub Copilot CLI.

One binary, one config file, three modes: `quelch ingest`, `quelch mcp`, and the bare `quelch ...` CLI.

## Why two stores

Azure AI Search is excellent at semantic search and weak at exact, exhaustive, aggregable queries — and *both* are needed when an agent has to answer a real question like "how many open Jira issues are assigned to me?" or "what's planned for the next sprint?".

Quelch keeps Azure AI Search for what it's good at and adds Cosmos DB underneath as the system of record:

- **Cosmos DB** holds the raw documents and answers exact, structured, and aggregable queries (`query`, `get`, `aggregate` go here directly).
- **Azure AI Search** sits on top of Cosmos DB via its Indexer + integrated vectorisation. The MCP `search` tool routes through the **Knowledge Base** (Agentic Retrieval) layer — built-in question decomposition, reranking, and optional answer synthesis — not the raw index.
- **Quelch MCP** is the unified facade. Agents pick the right tool and Quelch routes per-tool to the right backend.

Result: agents can answer both "find issues that talk about camera connection problems" *and* "list every Story assigned to me with no exception" — in the same conversation, against the same data.

## Where to start

| If you want to … | Read |
|---|---|
| Set up Quelch from scratch — happy path | [getting-started.md](getting-started.md) |
| Understand how Quelch fits together | [architecture.md](architecture.md) |
| Write or edit the config file | [configuration.md](configuration.md) |
| Look up a specific command | [cli.md](cli.md) |
| Understand or debug incremental sync | [sync.md](sync.md) |
| Build an agent or skill that talks to Quelch | [mcp-api.md](mcp-api.md) and [agent-generation.md](agent-generation.md) |
| Deploy Quelch to Azure or on-prem | [deployment.md](deployment.md) |
| See real questions answered end-to-end | [examples.md](examples.md) |

## Five-minute overview

```bash
# 1. Scaffold a config (interactive — uses `az` to discover what you already have)
quelch init

# 2. Plan the Azure changes the config implies
quelch azure plan

# 3. Apply them
quelch azure deploy

# 4. Generate agent instructions for your platform of choice
quelch agent generate --target copilot-studio --output ./agent-bundle

# 5. Watch live state of every deployed worker
quelch status --tui
```

For local development with no Azure dependencies:

```bash
quelch dev
```

That spins up the simulator, an in-memory mock for Cosmos and AI Search, and the MCP server — all in one process — so you can iterate without touching the cloud.

## Core principles

- **One binary, one config.** `quelch` is the only thing you install. `quelch.yaml` is the only thing you version-control.
- **Config is the source of truth.** Quelch reconciles Azure to the config; never the other way around.
- **Bicep and rigg files are generated output.** Quelch synthesises Bicep (Azure resource shells) and rigg files (AI Search / Foundry configuration) from the config on every plan/deploy. You read the diff and approve. Hand-takeover is supported per file via a marker — see [architecture.md](architecture.md#provisioning-split-bicep-vs-rigg).
- **Workers are stateless.** All cursors live in the shared `quelch-meta` Cosmos container, not on local disk. Redeploys never lose state.
- **Agents see one API.** The MCP layer hides the Cosmos/AI-Search split. Agents reason about tools, not databases.

## Status

The architecture described here is what's implemented in the current shipping version. See [CHANGELOG.md](../CHANGELOG.md) for the per-release deltas.
