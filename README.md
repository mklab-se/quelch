<p align="center">
  <img src="https://raw.githubusercontent.com/mklab-se/quelch/main/media/quelch.png" alt="quelch" width="600">
</p>

<h1 align="center">Quelch</h1>

<p align="center">
  Ingest data from Jira and Confluence into Cosmos DB, serve it through an MCP API,<br>
  and manage Azure AI Search indexes — all from one declarative config.
</p>

<p align="center">
  <a href="https://github.com/mklab-se/quelch/actions/workflows/ci.yml"><img src="https://github.com/mklab-se/quelch/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://crates.io/crates/quelch"><img src="https://img.shields.io/crates/v/quelch.svg" alt="crates.io"></a>
  <a href="https://github.com/mklab-se/quelch/releases/latest"><img src="https://img.shields.io/github/v/release/mklab-se/quelch" alt="GitHub Release"></a>
  <a href="https://github.com/mklab-se/homebrew-tap/blob/main/Formula/quelch.rb"><img src="https://img.shields.io/badge/dynamic/regex?url=https%3A%2F%2Fraw.githubusercontent.com%2Fmklab-se%2Fhomebrew-tap%2Fmain%2FFormula%2Fquelch.rb&search=%5Cd%2B%5C.%5Cd%2B%5C.%5Cd%2B&label=homebrew&prefix=v&color=orange" alt="Homebrew"></a>
  <a href="https://github.com/mklab-se/quelch/blob/main/LICENSE"><img src="https://img.shields.io/crates/l/quelch.svg" alt="License"></a>
</p>

<p align="center">
  <a href="docs/getting-started.md"><strong>Getting started</strong></a> ·
  <a href="docs/README.md"><strong>Full documentation</strong></a> ·
  <a href="CHANGELOG.md"><strong>Changelog</strong></a>
</p>

---

## What is Quelch?

Quelch is a knowledge-platform operator tool for teams using Jira and Confluence. It ingests data into **Cosmos DB** as the system of record, uses **Azure AI Search** (via the embedded [rigg](https://github.com/mklab-se/rigg) library) for hybrid semantic retrieval, and exposes a **five-tool MCP API** that agents (Copilot Studio, VS Code Copilot, Claude, Codex) can call directly.

One Rust binary, one YAML config file, three runtime roles: `quelch ingest`, `quelch mcp`, and the operator CLI.

> **New here?** Start with **[docs/getting-started.md](docs/getting-started.md)** — a step-by-step happy-path walkthrough from `brew install` to a deployed MCP server an agent can talk to.

## Architecture overview

```
                                              ┌─────────────────────────────┐
                                              │  Azure AI Search            │
                            ┌─────────────┐   │   ├─ Indexer (auto-vector)  │
Sources ──quelch ingest──►  │  Cosmos DB  │◄──┤   └─ Knowledge Base         │
                            │ (raw JSON)  │   │      (Agentic Retrieval)    │
                            └──────┬──────┘   └─────────────┬───────────────┘
                                   │ query · get             │ search
                                   │ aggregate               │ (semantic)
                                   ▼                         ▼
                                 ┌────────────────────────────┐
                                 │        quelch mcp          │
                                 │ (per-tool routing; 5 tools)│
                                 └─────────────┬──────────────┘
                                               │  MCP Streamable HTTP
                                               ▼
                              Agent (Copilot Studio / VS Code / Claude / Codex / …)
```

The MCP server fans out **per tool**: `query`/`get`/`aggregate` hit Cosmos DB directly (exact, exhaustive); `search` routes through the Azure AI Search **Knowledge Base** (Agentic Retrieval — question decomposition, reranking, optional answer synthesis); `list_sources` answers from a cached schema catalog without any backend call. See [docs/architecture.md](docs/architecture.md) for full details.

## Features

- **Cosmos DB as system of record** — exact queries, counts, exhaustive listings, and cursor-based pagination without hitting search
- **Azure AI Search via rigg** — indexes, skillsets, indexers, knowledge sources, and knowledge bases all managed from `quelch.yaml`
- **Five-tool MCP API** — `search` (Knowledge Base agentic retrieval), `query` (Cosmos SQL), `get` (point-read), `list_sources`, `aggregate`
- **Incremental sync** — minute-resolution windows with safety lag, backfill resume, soft-delete reconciliation
- **Agent bundle generator** — `quelch agent generate` produces grounded bundles for Copilot Studio, Claude Code, VS Code Copilot, Copilot CLI, Codex, and Markdown
- **On-prem artefacts** — `quelch generate-deployment` writes docker-compose, systemd, or Kubernetes manifests; Quelch never SSHes anywhere
- **Operator CLI** — `azure plan`, `azure deploy`, `azure indexer`, `azure logs` with Bicep + `az` shell-outs
- **Rich TUI** — fleet dashboard showing live ingest state per worker, polling `quelch-meta`

## Installation

### Homebrew (macOS/Linux)

```bash
brew install mklab-se/tap/quelch
```

### Cargo

```bash
cargo install quelch
```

### Binary download

Download pre-built binaries from the [latest release](https://github.com/mklab-se/quelch/releases/latest).

## Quick check

After installing, confirm the binary works and try Quelch entirely offline against the in-process simulator:

```bash
quelch --version
quelch dev          # offline — sim + Cosmos mock + ingest + MCP, all in one process
```

The TUI fleet dashboard appears; press `q` to exit. No Azure account or source credentials needed for `quelch dev`.

## Getting started

When you're ready to run Quelch against real Jira / Confluence and deploy to Azure, follow [docs/getting-started.md](docs/getting-started.md) — a step-by-step happy-path walkthrough covering prerequisites, `quelch init`, planning, deploying, and connecting an agent.

## CLI surface

Run `quelch --help` for the live command list. See [docs/cli.md](docs/cli.md) for every command and flag with examples and discussion.

## Documentation

| Doc | Purpose |
|-----|---------|
| [docs/README.md](docs/README.md) | Vision and 5-minute overview |
| [docs/architecture.md](docs/architecture.md) | Components, data flow, topology |
| [docs/configuration.md](docs/configuration.md) | `quelch.yaml` reference |
| [docs/cli.md](docs/cli.md) | Every command + flag |
| [docs/sync.md](docs/sync.md) | Sync correctness algorithm |
| [docs/mcp-api.md](docs/mcp-api.md) | Five MCP tools, schemas, pagination |
| [docs/deployment.md](docs/deployment.md) | Azure plan/deploy + on-prem artefacts |
| [docs/agent-generation.md](docs/agent-generation.md) | `quelch agent generate` targets |
| [docs/examples.md](docs/examples.md) | End-to-end agent usage walkthroughs |

## License

MIT
