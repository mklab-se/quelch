# Quelch v2 — re-architecture design spec

**Date:** 2026-04-30
**Status:** Approved during brainstorming. Awaiting written-spec review.
**Canonical documentation:** [/docs](../../../docs/) — this spec is the brainstorming output that ties the doc set together.

## Goal

Transform Quelch from a direct-to-Azure-AI-Search ingest tool into a small operator-driven knowledge platform built on:

- **Cosmos DB** — system of record. Holds raw documents and answers exact, structured, aggregable queries.
- **Azure AI Search** — secondary, indexes Cosmos DB via its built-in Indexer + integrated vectorisation. Answers semantic / hybrid queries.
- **Quelch MCP server** — unified facade. Exposes a five-tool MCP API to agents.
- **Quelch CLI** — single operator binary. Reconciles Azure to a version-controlled config.

One Rust binary, one YAML config file, three runtime roles (`quelch ingest`, `quelch mcp`, the CLI itself).

## Background

Quelch v1 (currently shipping at 0.8.0) writes data from Jira and Confluence directly into Azure AI Search. The v1 design has a sharp ceiling: AI Search is excellent at semantic search but weak at exact, exhaustive, and aggregable queries — both of which are needed for real agent use cases ("how many open issues are assigned to me?", "list every Story in this sprint", "what's the next sprint?").

Real-world questions from a Copilot agent require:

- Exact, exhaustive listing.
- Exact counts and sums.
- Hybrid semantic + structured filtering ("connection problems in cameras").
- Resolution of domain concepts ("the next sprint", "the last iXX firmware").

These cannot be served by AI Search alone.

## Non-goals

- Backwards compatibility with v1. No migration path required.
- Multi-tenant SaaS. Each Quelch installation is a single-tenant deployment.
- Vector backends other than Azure AI Search.
- Embedding model choice — Azure AI Search owns vectorisation via integrated vectorisation skillsets in v2.
- Source systems other than Jira and Confluence in v1 (the trait stays open for future connectors).

## High-level architecture

See [/docs/architecture.md](../../../docs/architecture.md) for the full picture, ASCII diagrams, and module-level breakdown.

In short:

```
Sources → quelch ingest → Cosmos DB → AI Search Indexer → AI Search Index
                              ↑                                ↑
                              └─ quelch-meta (cursors)         │
                              ↑                                │
                       quelch mcp ────────────────────────────┘
                              ↑
                       Agent (Copilot Studio / VS Code MCP / etc.)
```

## Key design decisions

The decisions below are the output of the brainstorming dialogue. Each has its rationale recorded.

### 1. Embeddings live in Azure AI Search, not in the ingest worker

Decision: Quelch ingest writes raw JSON to Cosmos DB. Azure AI Search Indexer + integrated vectorisation skillsets (calling Azure OpenAI) compute embeddings during indexing.

Rationale:

- Ingest workers stay dumb and dependency-light (no embedding model required at the ingest tier).
- Re-embedding the corpus is a Search-side operation (skillset rerun + indexer reset), not an ingest-side one.
- The `ailloy` dependency stays for *future* AI features, but is not on the v2 ingest path.

### 2. Cosmos container layout — defaults plus per-source override

Decision: one container per source-type by default (`jira-issues`, `confluence-pages`, plus companion containers `jira-sprints`, `jira-fix-versions`, `jira-projects`, `confluence-spaces`). Each source instance can override its target container in config.

Rationale:

- Default is "just works" for simple installations.
- Override unlocks multi-instance setups (`jira-issues-internal` vs `jira-issues-cloud`) without forcing them.
- Companion containers exist so agents can resolve domain concepts ("next sprint", "last fix version") with a query rather than out-of-band knowledge.

### 3. The config is the source of truth; Bicep + rigg are generated output

Decision: a single `quelch.yaml` (version-controlled) describes the entire solution. From it Quelch generates two artefacts:

1. **Bicep** for Azure resource shells (Cosmos, AI Search service, Container Apps, Key Vault, OpenAI references). Lives at `.quelch/azure/<deployment>.bicep`, applied via `az deployment group create`. Drift detected via `az deployment group what-if`.
2. **rigg files** for AI Search and Microsoft Foundry *configuration* (indexes, skillsets, indexers, knowledge sources, knowledge bases, Foundry agents). Lives under `rigg/`, applied via the embedded `rigg-client` library. Drift detected via `rigg diff`.

`quelch azure plan` shows a combined diff of both halves; `quelch azure deploy` applies both. See [/docs/architecture.md](../../../docs/architecture.md#provisioning-split-bicep-vs-rigg).

Rationale:

- Single source of truth = no out-of-band tribal knowledge.
- Bicep is reviewable in PRs; so are rigg files.
- `what-if` is Azure-native dry-run; no need to reinvent.
- Hand-editing Bicep is forbidden; it's regenerated every plan.

### 4. The MCP API is five tools, speaks data sources, and routes per-tool

Decision: `search`, `query`, `get`, `list_sources`, `aggregate`. Tools take `data_source` (or `data_sources`) as the addressable unit — a logical name like `jira_issues`, `jira_sprints`. The MCP server resolves each logical data source to one-or-more physical Cosmos containers (and matching AI Search indexes/knowledge sources) per the `mcp.data_sources` config.

The backend each tool uses is a property of the tool's **semantics**, not user config:

- `search` → Azure AI Search **Knowledge Base** (Agentic Retrieval — Layer 1). Built-in question decomposition, sub-query planning, reranking. Materially better answers for fuzzy questions.
- `query`, `get`, `aggregate`, `list_sources` → direct backend (Cosmos SQL / point-read / cached metadata — Layer 0). Exact, exhaustive, structured.

The only operator knob is `mcp.search.disable_agentic` (cost opt-out — falls back to direct hybrid search). Agent's view doesn't change.

`search` accepts an `include_content` argument — `"snippet"` (default), `"full"`, or `"agentic_answer"` — which controls how much per-hit material comes back. `"full"` eliminates the search-then-fetch loop for "summarise across many documents" use cases (one round-trip, agent synthesises with all content in hand). `"agentic_answer"` returns a Knowledge-Base-synthesised paragraph plus citations, for quick-answer cases where calling-agent voice doesn't matter. We do *not* add a separate `summarise` MCP tool — synthesis is the calling agent's strength, and putting it in the MCP would couple us to a synthesis style we have no context for.

A full Foundry agent in front of the MCP (Layer 2) is explicitly out of scope: it duplicates the calling agent's reasoning, breaks MCP contract semantics (latency, determinism), and adds cost without clear benefit. If a user wants a hosted Foundry agent that uses Quelch, they point one at Quelch's MCP — they don't bake one in.

See [/docs/mcp-api.md](../../../docs/mcp-api.md) and [/docs/architecture.md](../../../docs/architecture.md#two-layers-of-names).

Rationale:

- Each tool has a single, obvious purpose. Agents pick correctly when descriptions are clear.
- A single "smart router" hides which database is doing the work and is bad for trust and debugging.
- All listing tools support cursor pagination so "all" is always reachable.
- `expose:` server-side filter prevents agents from touching unlisted data sources.
- **Two-layer naming.** Storage (containers, indexes) is operator-only; agents only ever see logical data sources. This is what makes multi-instance setups (e.g. one logical `jira_issues` backed by both `jira-issues-internal` and `jira-issues-cloud`) transparent to the agent — the MCP server fans out and merges.

### 5. State for distributed ingest lives in Cosmos

Decision: cursors and per-worker state live in a shared `quelch-meta` container in the same Cosmos DB. Local-file fallback for `quelch dev`.

Rationale:

- Container Apps have ephemeral local disk; durable state requires somewhere external.
- Quelch already has Cosmos in the loop; introducing another service is overkill.
- One container = one query for `quelch status` to read live state of every worker.
- Workers are designed to be **disjoint by config**, not coordinated at runtime.

### 6. Hybrid deploy mechanism: Bicep for provisioning, `az` shell-outs for runtime

Decision: Bicep + `az deployment` for provisioning; direct `az` calls for runtime ops (`indexer run`, `logs`, etc.).

Rationale:

- Provisioning is naturally declarative; runtime ops are naturally imperative.
- `az` is already what an operator would debug with; same tooling.
- Avoids the rough edges of the Rust Azure SDK at v1.

### 7. Workloads run on Azure Container Apps; image ships from `ghcr.io`

Decision: ingest and MCP run as Container Apps. Image is `ghcr.io/mklab-se/quelch:<version>`, built by the existing release workflow. CLI version pins the image tag.

Rationale:

- Container Apps scale to zero (cheap idle MCP), have managed identity, are simpler than AKS.
- Pinning image to CLI version eliminates operator/worker drift.

### 8. On-prem deployment is generation, not management

Decision: `quelch generate-deployment` produces docker-compose / systemd / k8s artefacts. The user runs them. Quelch never SSHes anywhere.

Rationale:

- Many on-prem environments have policies that forbid agentic deploy from laptops.
- Generation is composable with whatever CI/CD the user already has.
- Reduces Quelch's blast radius.

### 9. Auth: API key now, Entra ID later

Decision: MCP supports API key (v1) and Entra ID (v1.x). API key is the default initially because it requires no AAD app registration.

Rationale:

- Lets the user start without standing up an AAD app.
- Entra is the right long-term answer; agent platforms (Copilot Studio, VS Code MCP) all speak it.

### 10. Existing assets — TUI, sim, mock — survive

Decision: keep `tui/`, `sim/`, `mock/`. Refocus the TUI on `quelch dev` and `quelch status --tui` (fleet dashboard reading `quelch-meta`). Sim and mock drive `quelch dev` and CI.

Rationale:

- They work; they're well-tested; replacing them is pure cost.
- They map onto v2 cleanly with no rework.

### 11. Agent generation is first-class

Decision: `quelch agent generate --target [copilot-studio|copilot-cli|vscode-copilot|claude-code|codex|markdown]` produces grounded, deployment-specific bundles (system prompt, tool descriptions, schema cheatsheet, connection details, example prompts). Output form (agent vs skill) is target-defaulted with `--format` override; skill is the default for CLI/IDE targets, agent is the default for Copilot Studio.

Rationale:

- Agents perform much better with grounded prompts than generic ones.
- Quelch already has the deployment metadata; not generating it would be a wasted asset.
- Repurposes the existing `copilot.rs` work as one target.

### 12. Sync correctness — minute-resolution intervals with safety lag

Decision: incremental sync uses closed minute-resolution intervals with a fixed safety lag (default 2 minutes). The cursor `last_complete_minute` advances atomically only on full window success. Backfill resumes via a `(updated, key)` checkpoint with a fixed `backfill_target`. Deletions detected by periodic full reconciliation; soft-deleted in Cosmos via a `_deleted` flag honoured by the AI Search Indexer's soft-delete column policy.

Rationale:

- Atlassian APIs filter at minute resolution but emit per-second timestamps; Atlassian indexes lag — naive "remember the latest seen, query ≥" algorithms are wrong on every one of these mismatches and were a real source of v1 bugs.
- Closed minute intervals are symmetric with the filter precision: never advance past a minute we haven't fully covered.
- Safety lag absorbs Atlassian-side indexing lag and clock drift.
- Idempotent upserts mean repeated coverage of the same window is harmless.
- Full reconciliation is the only way to detect deletes (Atlassian doesn't surface them in the change feed).
- Soft delete via `_deleted` flag matches the canonical Azure Cosmos → AI Search Indexer pattern and gives auditability + recovery.

Full algorithm — JQL/CQL formats, per-cycle pseudocode, backfill resume protocol, operator FAQs — documented in [/docs/sync.md](../../../docs/sync.md).

### 13. Rigg as embedded library for AI Search and Foundry configuration

Decision: Quelch depends on `rigg-core` and `rigg-client` as Cargo workspace dependencies. AI Search index/skillset/indexer/data-source/synonym-map/alias/knowledge-source/knowledge-base configuration and Microsoft Foundry agents are managed by rigg, not by Bicep. Quelch generates rigg files under `rigg/` from `quelch.yaml`, then plans/pushes them via the embedded library. Users can hand-take-over individual rigg files (`# rigg:managed-by-user` marker) for fine-grained tuning while keeping the rest generated.

Rationale:

- AI Search internals are iterated frequently (analyzers, scoring profiles, knowledge-base rerankers); Bicep is the wrong tool for fast iteration.
- Rigg already models all of these resources, ships an MCP server for AI-tool integration, and supports pull/push/diff against live Azure.
- Embedding rigg as a library means users install only `quelch`. No second tool, no version-skew between operator tooling.
- Same author (MKLab) for rigg, ailloy, and quelch — version bumps coordinated; no compatibility burden.

## Modules — keep, replace, new

| Module | Fate |
|---|---|
| `sources/jira.rs`, `sources/confluence.rs`, `sources/mod.rs` | **Keep & extend.** Add companion-container fetches (sprints, fix versions, spaces). |
| `config/` | **Keep & extend.** Add `cosmos`, `openai`, `deployments`, `mcp`, `rigg`, `state` sections. |
| `tui/` | **Keep, refocus.** Default UX of `quelch dev`; powers `quelch status --tui`. |
| `sim/`, `mock/` | **Keep.** Drive `quelch dev` and CI. |
| `ai.rs` (ailloy) | **Keep as dep.** Reserved for future AI features. Not on v2 ingest path. |
| `copilot.rs` | **Keep, refactor.** Becomes the `copilot-studio` target of `quelch agent generate`. |
| `azure/mod.rs` | **Remove entirely.** AI Search reads/writes go through `rigg-client` (config) and the MCP `search` tool. |
| `sync/mod.rs` | **Replace.** New `ingest/` module: pull → write Cosmos → write cursor. |
| `sync/embedder.rs` | **Remove.** AI Search owns embeddings (managed via rigg-generated skillsets). |
| Commands `sync`, `watch`, `setup`, `reset-indexes`, `search`, `generate-agent` | **Replace.** New command tree per [/docs/cli.md](../../../docs/cli.md). |
| **New** `cosmos/` | Cosmos DB client (writes, point-reads, SQL queries). |
| **New** `mcp/` | Streamable HTTP server, five tool implementations, expose filter, pagination, knowledge-base routing for `search`. |
| **New** `azure/deploy/` | Bicep generator + `az` shell-outs + `what-if` parser, for the resource-shell layer. |
| **New** `azure/rigg/` | Generates `rigg/` files from `quelch.yaml`; thin wrapper around `rigg-core` + `rigg-client` for plan/diff/push. |
| **New** `agent/` | Agent and skill bundle generators per target. |
| **New** `config/deployments.rs` | Slicing logic — full config → per-deployment effective config. |
| **External lib** `rigg-core` + `rigg-client` | AI Search and Foundry resource models, REST clients, diff. |
| **External lib** `ailloy` | Reserved for future AI features in Quelch. |

## Documentation deliverables

A doc set under `/docs/`, plus this spec:

| File | Purpose |
|---|---|
| [README.md](../../../docs/README.md) | Vision and 5-min overview. |
| [architecture.md](../../../docs/architecture.md) | Components, data flow, document model, state, topology, what changes. |
| [configuration.md](../../../docs/configuration.md) | `quelch.yaml` reference, every section. |
| [cli.md](../../../docs/cli.md) | Every command + flag with examples. |
| [sync.md](../../../docs/sync.md) | Sync correctness — minute-resolution algorithm, backfill, deletions, operator FAQ. |
| [mcp-api.md](../../../docs/mcp-api.md) | The five tools, schemas, pagination, expose filter. |
| [deployment.md](../../../docs/deployment.md) | Azure plan/deploy + on-prem generated artefacts. |
| [agent-generation.md](../../../docs/agent-generation.md) | `quelch agent generate`, targets, bundle contents (agent + skill forms). |
| [examples.md](../../../docs/examples.md) | Sixteen end-to-end walkthroughs covering exhaustive listing, counting, hybrid search, summarisation, recency, staleness, sprint planning, release notes, dependency tracing, and discovery. |

## Open items deferred to implementation planning

- Filter-grammar exact spec (translation rules to Cosmos SQL and AI Search OData) — sketched in [/docs/mcp-api.md](../../../docs/mcp-api.md), to be locked down with concrete tests during implementation.
- MCP Streamable HTTP transport — verify against the latest published MCP specification at implementation time.
- Bicep modules — to be designed during implementation; preference is one Bicep file per deployment, with a top-level shared module for Cosmos / AI Search / OpenAI / Key Vault.
- Schema migrations in Cosmos DB — out of scope for v1; documents are append-only and the indexer can be reset.
- Multi-region — explicitly out of scope; single-region deployments only in v1.

### Deferred: queue-based ingestion

**Decision:** v1 ingest is a direct loop — `SourceConnector → in-process channel → CosmosWriter`. No external queue (Service Bus, Storage Queue, Event Hubs).

**Why deferred, not declined:** the wins of a queue (decoupled producer/consumer rates, parallel writers, native retry/DLQ, fan-out, webhook path) are real, but none apply to v1's scope:

- Source APIs (Jira, Confluence) already serve as durable, replayable change logs — cursor-based incremental sync gives us at-least-once delivery for free, with simpler semantics than queue-with-ack.
- Workers are designed disjoint by config (Jira A–K vs L–Z), so parallelism inside one subsource isn't a goal.
- Modest event rates from Jira/Confluence; one worker keeps up.
- Adding a queue means another Azure resource, another set of role assignments, another permission flow, another section in deployment.md — real user-facing complexity.
- Inserting a queue would split cursor-advance semantics (advance on enqueue vs advance on durable write); not impossible to handle, but not free.

**Design constraint to preserve future-insertability:** the ingest implementation must keep a clean seam between source-pull and Cosmos-write. Both sides communicate via `SourceDocument` (already defined in `sources/mod.rs`); the in-process `mpsc` channel between them must be the only point of contact. The day we want a queue, only the channel changes — neither side moves.

**Trigger conditions to revisit (any one is sufficient):**

1. We add webhook ingestion (push from source). A queue between the HTTP receiver and the writer becomes essentially mandatory at that point.
2. A single source produces more change-events per second than one worker can write to Cosmos. Unlikely for Jira/Confluence; conceivable for higher-volume sources we add later.
3. A genuine fan-out need emerges (e.g. ingest also feeds an audit pipeline or a notification stream).

## Implementation sequencing — preview only

This spec covers vision and architecture. The implementation plan (forthcoming) will sequence the work; rough phasing:

1. **Cosmos write path** — replace `azure/` write client + `sync/` engine with `ingest/` + `cosmos/`. Update existing tests.
2. **AI Search Indexer wiring** — Bicep for index/indexer/skillset; reconcile.
3. **MCP server** — Streamable HTTP transport + five tools. Tests via `quelch dev`.
4. **Operator commands** — `azure plan`, `azure deploy`, `azure indexer`, `azure logs`. Bicep generator.
5. **Agent bundle generator** — refactor `copilot.rs` into the target framework, add new targets.
6. **On-prem artefacts** — `generate-deployment` for docker / systemd / k8s.
7. **Config wizard** — `quelch init` interactive, with `az` discovery.
8. **TUI refocus** — `status --tui` fleet dashboard reading `quelch-meta`.

Each phase is independently shippable behind a Cargo feature flag or simply by leaving the new commands hidden until ready. v2 ships as a single major-version bump (v1.0.0) when all eight phases are landed.

## Approval

Architecture approved during brainstorming dialogue on 2026-04-30. The doc set under `/docs/` is the canonical spec; this file is the brainstorming-output anchor.

Implementation plan to follow in a separate `docs/superpowers/plans/` document.
