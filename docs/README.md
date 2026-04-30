# Quelch — documentation

Quelch is a single Rust binary that turns your enterprise knowledge sources (Jira, Confluence, ...) into a queryable knowledge platform that AI agents can use.

It does this by being three things at once:

1. **An ingestion service** that pulls data from sources and writes it to **Cosmos DB**.
2. **An MCP server** (Streamable HTTP) that lets agents query that data over a small, well-defined tool set, blending classical filtering (Cosmos DB) with semantic search (Azure AI Search).
3. **An operator CLI** that provisions the Azure resources, deploys the workers and the MCP, runs the indexers, and generates the agent-side instructions you paste into Copilot Studio / VS Code / GitHub Copilot CLI.

One binary, one config file, three modes: `quelch ingest`, `quelch mcp`, and the bare `quelch ...` CLI.

## Why Quelch v2 looks different from v1

Quelch v1 wrote directly from sources into Azure AI Search. That was simple, but it has a sharp ceiling: Azure AI Search is excellent at semantic search and weak at exact, exhaustive, aggregable queries — and *both* are needed when an agent has to answer a real question like "how many open Jira issues are assigned to me?" or "what's planned for the next sprint?".

Quelch v2 keeps Azure AI Search for what it's good at (hybrid semantic search) and adds Cosmos DB underneath as the system of record:

- **Cosmos DB** holds the raw documents and answers exact, structured, and aggregable queries.
- **Azure AI Search** sits on top of Cosmos DB via its built-in Indexer + integrated vectorization, and answers semantic queries.
- **Quelch MCP** is the unified facade. Agents pick the right tool (`search`, `query`, `get`, `list_sources`, `aggregate`) and Quelch routes to the right backend.

Result: agents can answer both "find issues that talk about camera connection problems" *and* "list every Story assigned to me with no exception" — in the same conversation, against the same data.

## Where to start

| If you want to … | Read |
|---|---|
| Understand how Quelch fits together | [architecture.md](architecture.md) |
| Write or edit the config file | [configuration.md](configuration.md) |
| Look up a specific command | [cli.md](cli.md) |
| Build an agent that talks to Quelch | [mcp-api.md](mcp-api.md) and [agents.md](agents.md) |
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
- **Bicep is generated output.** Quelch synthesises Bicep from the config on every plan/deploy. You read the diff and approve. You never hand-edit Bicep.
- **Workers are stateless.** All cursors live in the shared `quelch-meta` Cosmos container, not on local disk. Redeploys never lose state.
- **Agents see one API.** The MCP layer hides the Cosmos/AI-Search split. Agents reason about tools, not databases.

## Status

This document describes the **target architecture** for Quelch v2. The implementation is in progress; v1 (direct-to-Azure-AI-Search) is the current shipping version. See the changelog for what's actually live.
