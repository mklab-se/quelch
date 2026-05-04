# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

## [0.11.0] - 2026-05-04

### Added

- **Cross-resource-group references**: every external resource Quelch
  references gains an optional `_resource_group` sibling field that
  overrides `azure.resource_group` for just that resource. Useful for
  enterprise setups where a shared Foundry project, Cosmos account, or
  Key Vault lives in a separate RG owned by another team.

  New optional fields:
  - `cosmos.account_resource_group`
  - `search.service_resource_group`
  - `ai.resource_group`
  - `azure.resources.container_apps_env_resource_group`
  - `azure.resources.application_insights_resource_group`
  - `azure.resources.key_vault_resource_group`

  Each defaults to `azure.resource_group` when absent. Threaded through
  `quelch init` discovery, `quelch validate` prerequisite checks, and
  the Bicep generator (which emits `scope: resourceGroup('rg-other')` on
  the relevant `existing` block). Cross-subscription references are not
  supported yet — separate ask if needed.

- **`quelch mcp-key set / rotate / show`**: first-class commands to
  manage the Q-MCP API key for a deployment. Closes the operator-UX
  hole flagged in v0.10.1 (the docs documented the manual `az keyvault
  secret set` flow but there was no first-class command).
  - `set` / `rotate` generate 32 random bytes (base64-encoded), write
    them to whichever secret store the deployment uses, and trigger a
    Container App revision restart so the new value is live in seconds.
  - `show` reads the value back (Azure deployments only — on-prem
    secret stores are local-only and unreachable from Quelch).
  - On-prem deployments print copy-pasteable docker / systemd / k8s
    commands instead of trying to remote-write.

### Changed

- **Docs — terminology rollout completed**: `architecture.md`,
  `configuration.md`, `cli.md`, `sync.md`, `examples.md`,
  `agent-generation.md`, and `docs/README.md` now lead with the
  Quelch MCP (Q-MCP) / Quelch Ingest (Q-Ingest) split that was
  introduced in v0.10.1. The architecture diagram labels the MCP server
  as Q-MCP. The sync doc notes explicitly that the algorithm it
  describes is what Q-Ingest runs on every cycle.

## [0.10.1] - 2026-05-04

### Changed

- **Default chat model**: switched from `gpt-4.1-mini` to `gpt-5-mini`
  across the wizard, templates, dev mode, and test fixtures. `gpt-5-mini`
  is in Microsoft's portal-validated subset for the Knowledge Base
  preview API and is the newer of the two same-cost-tier options.
  Existing configs are not affected — this only changes defaults for
  new `quelch init` runs and the docs' worked examples.
- **Docs — Q-MCP / Q-Ingest terminology**: standardised on "Quelch MCP"
  (Q-MCP) and "Quelch Ingest" (Q-Ingest) across README, getting-started,
  deployment, mcp-api, and CLAUDE.md. Spell out on first mention, then
  use the short form. Other docs will follow as they're touched.
- **Docs — topology emphasis**: the README architecture diagram, the
  hybrid-topology section in deployment.md, and the getting-started intro
  all now lead with "Q-MCP typically in Azure, Q-Ingest typically near
  the data source (often on-prem)" rather than treating an
  everything-in-Azure setup as the default mental model.
- **Docs — secrets scoped per target**: Key Vault is no longer described
  as a universal Quelch prerequisite. The getting-started prereqs table
  now splits "always required" (Cosmos / AI Search / AI provider) from
  "required only if any deployment has `target: azure`" (ACA env, App
  Insights, Key Vault). On-prem Q-Ingest keeps secrets in env / `.env` /
  k8s `Secret` — never touches Key Vault.

### Added

- **`mcp-api.md` "Setting and rotating the API key"** — end-to-end
  documentation of the operator flow that was previously left as a
  `// TODO: populate at deploy time` comment in the generated Bicep.
  Covers Azure-host (`az keyvault secret set` + revision restart) and
  on-prem (docker / systemd / k8s) paths, with `openssl rand` examples.

### Fixed

- **Key Vault secret name** read by `getting-started.md` matched the
  Bicep template's name (`quelch-mcp-api-key`). The doc previously read
  from `mcp-api-key`, which the template never populates.

## [0.10.0] - 2026-05-04

### BREAKING

- **Schema**: replaced the flat `openai:` block with a structured `ai:` block
  that supports both Microsoft Foundry projects and Azure OpenAI accounts and
  carries an explicit chat-completion deployment for the Knowledge Base. The
  old `openai:` shape is no longer accepted.
  ```yaml
  ai:
    provider: foundry          # foundry | azure_openai
    endpoint: <resource URI>
    embedding:
      deployment: text-embedding-3-large
      dimensions: 3072
    chat:
      deployment: gpt-4.1-mini
      model_name: gpt-4.1-mini
      retrieval_reasoning_effort: low      # minimal | low (default) | medium
      output_mode: answerSynthesis         # answerSynthesis (default) | extractedData
  ```
  All existing `quelch.yaml` files need to be updated to the new shape on
  upgrade. The wizard's interactive flow generates the new shape automatically.

- **Provisioning model**: Quelch no longer creates the Cosmos DB account, AI
  Search service, Key Vault, Container Apps environment, Application Insights
  component, or AI provider. The user must pre-provision these (see
  `docs/getting-started.md` "Prerequisites" for the `az` commands). The
  generated Bicep references them via `existing` blocks and creates only the
  Cosmos database + containers (children of the existing account), one
  user-assigned managed identity per deployment, role assignments, and the
  Container App.

- **Schema**: new optional `azure.resources:` block for the names of the
  pre-existing Container Apps environment, Application Insights component,
  and Key Vault. Each defaults to `{prefix}-{env}-<kind>` if absent.

### Added

- **Microsoft Foundry support**: `quelch init` now asks whether your model
  deployments live in Microsoft Foundry (recommended) or Azure OpenAI,
  discovers existing accounts/projects in the chosen resource group via
  `az`, and falls back to manual entry with a copy-pasteable `az` create
  command if none are found.
- **Chat (LLM) model wiring**: the AI Search Knowledge Base generated by
  `azure/rigg/generate.rs` now emits a `models[]` array bound to the
  configured chat deployment, plus `retrievalReasoningEffort` and
  `outputMode`. Previous versions emitted only `knowledgeSources`, leaving
  the KB unable to do agentic retrieval at all.
- **Prerequisite check**: a new `init/prereq.rs` module hits `az` for every
  required resource and prints a per-resource ✓/✗/? report at `quelch init`
  exit-time and on every `quelch validate`. Missing resources include a
  copy-pasteable `az` create command.
- **`Cognitive Services User` role assignment** on the AI provider, so the
  Container App's managed identity can call the embedding and chat models
  configured in `ai.endpoint`.

### Changed

- The README now surfaces a "Getting started" link in the top navigation
  strip and as a callout right after the intro paragraph.
- `getting-started.md` rewritten with a prerequisite table that includes
  copy-pasteable `az` commands for every resource.
- `configuration.md` documents the new `ai:` block, the new
  `azure.resources:` block, and the supported chat models.
- Bicep snapshot tests re-baked. Generated Bicep is ~150 lines shorter
  and clearer about what's user-managed vs Quelch-managed.

## [0.9.4] - 2026-05-04

### Security

- **MCP API-key comparison**: replaced byte-string `!=` with a constant-time
  helper to close a timing-attack vector that could let a network-adjacent
  attacker recover the configured key.
- **OData `LIKE` translation** (`mcp/filter/odata.rs`): the field name was
  embedded into `search.ismatch(...)` unescaped, so a crafted `where` clause
  could inject OData syntax. Both the field and the user-supplied pattern
  are now escaped through `escape_odata_string`.
- **`order_by.field` SQL embedding** (`mcp/tools/query.rs`): added an
  allowlist validator (`is_valid_field_path`) that rejects anything outside
  `[A-Za-z0-9_.]` before the field is interpolated into the Cosmos SQL
  string. Five new unit tests cover the rejected forms.
- **Bicep injection guard** (`config/validate.rs`): user-supplied identifiers
  flowing into generated Bicep — `azure.resource_group`, `azure.region`,
  `naming.prefix`/`environment`, Cosmos field names, AI Search service name,
  every container name, source names, and deployment names — are now
  rejected if they contain `'` or `\`.

### Fixed

- **Cosmos continuation token** (`cosmos/client.rs`): the token was being
  stored as the `Debug` representation of the header struct (`{c:?}`), which
  made every paginated query restart at page one. Token is now read via
  `Header::value().as_str()`. Required adding `azure_core` to the workspace
  dependencies.
- **MCP `tools/call` handler**: replaced four `serde_json::to_value(...).unwrap()`
  / `to_string(...).unwrap()` calls with a JSON-RPC internal-error helper
  so the server can no longer panic on a non-serialisable response.
- **Ingest non-fatal error logging**: companion fetch/upsert in
  `ingest/cycle.rs`, cursor-save-after-failure in `ingest/backfill.rs`, and
  reconciliation metadata in `ingest/reconcile.rs` were silently dropping
  errors via `let _ = ...` / `.ok()`. They now log via `tracing::warn!` so
  recurring drift is visible to operators. Behaviour is unchanged.

### Changed

- **Dead code removal**: `crates/quelch/src/copilot.rs` (35 lines, no
  callers) deleted; the stub `Sync`/`Watch`/`Setup`/`ResetIndexes`/`Sim`/
  `GenerateAgent` CLI variants and their bail handlers are gone; the unused
  `sim::run` stub is gone. `crates/quelch/src/mcp/handlers/tools_call.rs`
  picked up a `to_value_or_internal` helper to keep the dispatcher tidy.
- **Documentation**: dropped the remaining v1/v2 framing in `architecture.md`,
  `configuration.md`, `deployment.md` and replaced "What carries over from v1"
  with a current module map. Fixed `getting-started.md` deployment-name
  reference, MCP-smoke-test command, and clarified `--no-tui` is a global
  flag. Re-tagged stale `TODO(v2 follow-up)` / `TODO(phase-11)` markers in
  `mcp/tools/*` to point at what they actually track (`TODO(perf)`,
  `TODO(multi-container)`, `TODO(rigg-client)`, etc.).
- **CI/build hygiene**: `ci.yml` now sets `permissions: contents: read`,
  uses `cancel-in-progress` concurrency, and runs `cargo clippy --all-targets`
  in a single job. `release.yml` aligns `actions/upload-artifact` /
  `download-artifact` versions and fails hard if the Homebrew-tap update
  fails (it used to swallow the error). Dockerfile no longer hides stderr
  in the dependency-cache step.

## [0.9.3] - 2026-05-04

### Fixed

- **CI**: replaced the v1 `Simulator smoke test` job (ran `quelch sim`,
  which is stubbed in v2) with a v2-shaped binary smoke test that exercises
  `validate`, `effective-config`, `agent generate --target markdown`, and
  `azure plan --no-what-if`. All offline / no external services.
- **Dockerfile**: bumped `rust:1.83-slim-bookworm` → `rust:slim-bookworm`.
  Quelch's edition is `2024` which requires Rust 1.85+; pinning a stale
  major broke the ghcr.io image build.

## [0.9.2] - 2026-05-04

### Fixed

- Clippy 1.95 lints that v0.9.1's CI hit unexpectedly: `unnecessary_sort_by`,
  `field_reassign_with_default` (28 sites), `bool_assert_comparison`,
  `needless_borrow`, `await_holding_lock` (annotated where dropping the guard
  would race), plus a handful of unused imports / dead code. No behavioural
  changes.

### Changed

- MSRV bumped from 1.94.1 → 1.95.0. The CI workflows use
  `dtolnay/rust-toolchain@stable` so they auto-track latest stable; the MSRV
  is now consistent with that.

## [0.9.1] - 2026-05-04

### Fixed

- Workspace `Cargo.toml` referenced rigg-core / rigg-client via a hard-coded local
  path (`/Users/kristofer/repos/rigg/...`), which broke `cargo metadata` on any
  machine that wasn't the developer's. Switched to the now-published `rigg-core`
  and `rigg-client` 0.16.2 from crates.io. v0.9.0's release CI failed at the
  metadata step because of this; v0.9.1 is the de-facto v0.9.0 release.

## [0.9.0] - 2026-05-03

### Breaking

- Complete rewrite. v1 configs are not compatible. See [docs/](docs/) for the new architecture.
- Direct Azure AI Search write removed; data now flows source → Cosmos DB → AI Search Indexer.
- Embedding moved out of Quelch into Azure AI Search integrated vectorisation.
- v1 commands `sync`, `watch`, `setup`, `reset-indexes` removed; replaced by `ingest`, `azure plan/deploy`, `azure indexer reset`.

### Added

- Cosmos DB as the system of record (raw documents, cursors, worker state in `quelch-meta` container).
- Azure AI Search via rigg as a library — indexes, skillsets, indexers, knowledge sources, and knowledge bases all rigg-managed.
- MCP server (Streamable HTTP) with five tools: `search`, `query`, `get`, `list_sources`, `aggregate`.
- Operator CLI commands: `validate`, `effective-config`, `status`, `query`, `search`, `get`, `reset`, `ingest`, `mcp`, `dev`, `azure plan/deploy/pull/indexer/logs/destroy`, `generate-deployment`, `init`, `agent generate`.
- Six agent/skill bundle targets: `copilot-studio`, `claude-code`, `copilot-cli`, `vscode-copilot`, `codex`, `markdown`.
- On-prem deploy artefacts: docker-compose, systemd unit, Kubernetes manifest.
- Bicep generator + `az deployment group what-if` integration for Azure resource shells.
- Soft-delete via `_deleted` flag with periodic reconciliation.
- Minute-resolution sync algorithm with safety lag and backfill resume.
- Dockerfile for `ghcr.io/mklab-se/quelch:<version>`.

### Changed

- TUI refocused: was per-worker tracing-event view; now a fleet dashboard polling `quelch-meta`.

### Removed

- v1 `sync/` engine.
- v1 `azure/` AI Search write client (replaced by rigg).

## [0.8.0] - 2026-04-19

### Changed — Pushed counts are now authoritative

- **Cross-session cumulative "Pushed" counts.** Values shown in the Pushed column are now queried from Azure AI Search via `GET /docs/$count` — source-level uses the index-wide count, subsource-level uses `$filter=project eq 'X'` (Jira) or `$filter=space_key eq 'X'` (Confluence). If you've pushed 456 Jira issues across several sessions, that's what you see, not a session-local counter that resets to zero at startup.
- **Live badge.** A green `●` appears next to the Pushed cell while that subsource is fetching / embedding / pushing. Signals "authoritative + in-flight" rather than "frozen snapshot."
- **Batch-grouped live feed.** The "Pushed to Azure AI Search" pane no longer shows one row per document (noise: many rows per second with identical timestamps). Each successful push emits one row: `HH:MM:SS source/sub · batch of N · ID1, ID2, ID3, ID4, ID5, … (M more)`. Log mode gets the same payload as a single `phase = "batch_pushed"` event per batch.
- **All timestamps render in local time as `YYYY-MM-DD HH:MM:SS`.** Applies to the Sources pane's "Pushed at", the live feed, and the drilldown's "Pushed at (local)" / "Source updated (local)" rows. No more mixed 24h-only / ISO / UTC-vs-local across columns.

### Fixed

- **Per-min column decays when a subsource goes idle.** v0.7.0 summed the throughput ring without pruning, so an idle row kept showing its last-active number indefinitely. A non-mutating `per_minute_at(now)` / `chart_points_at(now)` path runs every render tick and drops samples older than the window.

### Added

- **`count_documents(index_name, filter)` on the Azure client** — thin wrapper over `GET /indexes/{name}/docs/$count?api-version=...` with optional `$filter`.
- **Mock `$count` route** with a tiny OData subset parser (`field eq 'value'` clauses joined by ` and `), mirroring Azure's contract so `quelch sim` exercises the same code path.
- **`phase = "batch_pushed"` / `"index_count"` / `"subsource_count"` tracing events** from the engine, replacing per-doc noise in structured output.

## [0.7.0] - 2026-04-20

### Changed — TUI semantic rework

Every column and counter is now keyed to **destination-side** quantities (what actually landed in Azure AI Search), not source-side (what was fetched from Jira/Confluence). v0.6.0 shipped a dashboard where the Items/Rate/Updated/Last-item columns, the latency stats, and the drilldown "recent" list all measured the wrong thing.

- **Sources pane columns replaced:** old `Source · Status · Items · Rate · Last item · Updated` becomes `Source · Stage · Pushed · Per min · Latest ID · Pushed at`. "Pushed" is the cumulative count of docs confirmed in Azure for that subsource (integer); "Per min" is pushes-per-minute as an integer; "Latest ID" is the full doc ID (no truncation); "Pushed at" is relative time since the last confirmed push.
- **New Stage column** shows the current pipeline stage per subsource: `fetching` · `embed X/Y` (live progress during embedding) · `pushing N` · `idle`. Answers "what is quelch doing right now?" at a glance.
- **New "Pushed to Azure AI Search (newest first)" pane** between Sources and Azure panel: live ticker of the last 20 docs confirmed in the destination index, with timestamp, source/subsource, and full ID — brightest at the top, fading with age.
- **Azure panel rewrite:** chart plots "Documents pushed per second" (meaningful y-axis auto-scaled to observed peak) instead of "Azure requests per second" (which pegged at 1). Counters become `Total pushed · Per minute · 4xx · 5xx · Throttled · Dropped`. Removed: `Total requests`, `median/95th latency` (not actionable).
- **Drilldown retitled** "Last pushed to Azure AI Search"; summary rows show `Pushed to Azure`, `Push rate`, `Latest ID`, `Pushed at`, `Source updated`; recent list now only shows docs confirmed pushed (old v0.6.0 list mixed in pre-push fetched docs).

### Added

- **`stage` / `doc_pushed` tracing events** from the engine. `stage` fires at `fetching` / `embedding (done/total)` / `pushing` boundaries inside each batch; `doc_pushed` fires per-doc after `azure.push_documents` returns success — the single source of truth for "this item is confirmed in the destination index."

### Fixed

- **Ctrl-C / `q` exit delay.** In v0.6.0 the engine ran its in-flight batch to completion (~20 s of SimEmbedder sleeps + Azure retries) before checking the shutdown signal. The simulator's engine loop now races `run_sync_with` against `cancel.cancelled()` in a `tokio::select!`, so quitting drops the in-flight batch immediately. State is persisted after each committed batch, so there's no loss.
- **Pre-push `doc_synced` event was misleading** — it fired between fetch and embed, before anything reached Azure. Renamed + moved the real emission to after a successful push, as `doc_pushed`. `doc_synced` is retained as an alias variant for backward compat but drives no UI state.

### Notes

This release treats the v0.6.0 TUI as a labelling mistake — the underlying plumbing was measuring the wrong quantities. The fix required adding destination-side emissions from the engine, not just relabelling columns.

## [0.6.0] - 2026-04-20

### Added
- **Interactive TUI for `quelch sim`.** Running `quelch sim` on a TTY now launches the redesigned dashboard directly, driven by the simulated world. Previously the command fell back to plain-log output because the subcommand was missing from the TUI-capable list. `--no-tui` still selects plain mode; `--snapshot-to` forces the headless renderer regardless of TTY.

### Changed
- **Ctrl-C is responsive again.** The plain-log path now sends `UiCommand::Shutdown` on the engine's command channel in addition to cancelling the token, so the engine exits within one subsource boundary (~50 ms) instead of finishing the in-flight cycle (~20 s).
- **Plain-log output is now actually useful.** Audited every log line per a new CLAUDE.md discipline. Each line must earn its place by telling the reader something concrete: cycle / source / subsource lifecycle transitions, batch results, Azure responses with latency, backoff events, sim startup. A 15-second `quelch sim --no-tui` now produces ~26 narrative lines instead of dozens of repeated retry warnings.
- **Engine event filter for the TUI layer is now `quelch=debug`** so the drilldown pane's recent-docs ring receives per-document events. Plain-log default stays at `quelch=info` so human-readable output isn't swamped.

### Removed / demoted
- **Per-cycle `[exists]` spam** from `setup_indexes` — now silent (or `debug!` when an operator wants to see it). `[created]` changed to a structured `info!` tracing event.
- **Per-retry `Request failed with ...` and `Request error: ...` warnings** from `azure::request_with_retry`. The structured `phase = "backoff_started"` event at `info!` level is the single source of truth; plain-log default (`quelch=info`) shows it once per retry, the TUI lights up the backoff banner, and the per-attempt bodies are now `debug!` for when someone needs them.
- **Per-document `phase = "doc_synced"` events** demoted from `info!` to `debug!`. TUI still captures them for the drilldown; plain-log output stays readable.

### Developer experience
- **CLAUDE.md: "Test Your Work Against the User's Actual Requirements" section.** Codifies the principle that `cargo test` passing is necessary but not sufficient — TUI, log, and UX claims must be verified by running the binary and reading the artifacts. Includes an explicit per-log-line audit checklist (audience / payload / verbosity / frequency) for anyone adding tracing output.

## [0.5.0] - 2026-04-20

### Added
- **`quelch sim` subcommand** — runs the real engine against a fully in-process simulated world (mock Jira, Confluence, Azure AI Search, and embedder) with burst-aware activity, Azure fault injection, and jittery embedder latency. Single command, no external services. Flags: `--duration`, `--seed`, `--rate-multiplier`, `--fault-rate`, `--assert-docs`, `--snapshot-to`, `--snapshot-frames`, `--snapshot-width`, `--snapshot-height`.
- **AI-verifiable TUI snapshot mode** — `quelch sim --snapshot-to FILE` renders the TUI to a headless ratatui `TestBackend` and dumps each frame as text to a file, enabling CI and AI agents to assert rendered content without a real terminal.
- **CI simulator smoke test** — new `sim-smoke-test` job in `.github/workflows/ci.yml` runs `quelch sim --duration 30s --seed 42 --no-tui --assert-docs 20` on every push.
- **`sim_headless` and `tui_snapshot` integration tests** — assert exit code, summary line, structured phase events in log mode; assert column headings, plain-English labels, chart axis labels, deduplicated footer in TUI mode; includes a narrow-terminal (100×30) variant.
- **Mock `/_sim/*` mutation endpoints** — the mock server gains `POST /_sim/jira/upsert_issue`, `POST /_sim/confluence/upsert_page`, `POST /_sim/jira/comment` so the simulator can inject realistic activity live.

### Changed
- **TUI redesign from scratch.** The v0.4.0 TUI had confusing labels, no columns, misaligned content rendering outside borders, no real time-series graphs, no clear status indicators, and a duplicated footer. Rebuilt end-to-end:
  - Sources pane uses `ratatui::Table` with explicit columns: Source · Status · Items · Rate · Last item · Updated.
  - Status shown as coloured dot glyphs (`● idle`, `◐ syncing`, `● error`, `◉ backoff`) with an animated spinner during sync.
  - Azure panel uses `ratatui::Chart`/`Dataset` with real y-axis ticks, `-60s → now` x-axis labels, and a plain-English counter strip (Total requests · Latency median/95th · Failed (4xx/5xx) · Throttled (429) · Dropped events).
  - Selected row uses inverse-video highlight; expanded/collapsed state persists across runs in `.quelch-tui-state.json`.
  - New `Drilldown` pane (opened by `Enter` on a subsource row) shows recent synced doc ids + recent errors.
  - New `HelpOverlay` modal (opened by `?`) groups keybindings by Navigation / Actions / View / Other.
  - Single deduplicated footer line.
  - Keypress latency reduced from up to 200 ms to a few milliseconds via async `crossterm::EventStream`.
  - Pause state updates optimistically in the header the moment `p` is pressed.
  - Live Azure backoff banner appears when `request_with_retry` is sleeping.
- **Engine emits additional structured events** — `source_started`, `source_finished`, `backoff_started`, `backoff_finished` phase events are now produced by the sync engine and Azure client, consumed by the `TuiLayer` for header and Azure-panel display.
- **Mock server split** — `crates/quelch/src/mock/mod.rs` refactored from a single 1031-line file into focused `jira.rs`, `confluence.rs`, `azure.rs`, `sim.rs` submodules along HTTP-route families; `mod.rs` retains only shared state, auth, router, and the integration test suite. Route paths and handler bodies unchanged.
- **`UiCommand::Shutdown` is now delivered to the engine** — `sim::run_engine_loop` receives the real command channel, so Ctrl-C exits the in-flight cycle at the next subsource/batch boundary (~50 ms) instead of waiting for the cycle to finish (~20 s).

### Removed
- **`Focus::Azure` enum variant and Tab-cycle handling** — with arrow-based navigation and drilldown as the primary interactions, the Sources/Azure focus distinction had no behaviour beyond border colour. Removed from `App`, `input.rs`, `layout.rs`, and the help overlay. `Prefs.focus` field retained for on-disk back-compat.

### Developer experience
- `tmp.txt`, `build_output.json`, and `build_output_clean.json` removed from the repo root and added to `.gitignore`.
- New workspace dependencies: `humantime`, `rand`, `tokio-util`, `futures`; new dev dep: `assert_cmd`.

## [0.4.0] - 2026-04-20

### Added
- **Interactive TUI** — ratatui-based live dashboard is the default experience for `quelch sync` / `quelch watch`. Shows per-source and per-subsource (project / space) progress cards, an Azure AI Search panel with request/error sparklines + p50/p95 latency + response counters, and a scrolling log view. Keybindings: `q` quit, `space`/`enter` collapse, `tab` focus, `r` sync-now, `p` pause/resume, `R` reset cursor (2s-confirm), `P` purge (2s-confirm), `s` toggle log view, `?` help.
- **Plain-log fallback** — `--no-tui` flag disables the TUI; non-TTY stdout auto-falls back to the plain `tracing_subscriber::fmt` subscriber (`--json` also implies plain).
- **Persistent UI prefs** — `.quelch-tui-state.json` remembers collapsed sections, focus, and log-view toggle across runs.
- **Per-subsource cursor tracking** — each Jira project and Confluence space now has its own cursor; `quelch status` shows per-subsource breakdown; `quelch reset --subsource=<KEY>` resets a single subsource.
- **Unified observability on `tracing`** — sync engine emits structured `phase=...` events consumed either by a `TuiLayer` (TUI mode) or plain `fmt` (log mode). Phase strings live in `sync::phases` so engine and TUI rename together.
- **`UiCommand` channel** — TUI pushes commands (Pause/Resume/SyncNow/ResetCursor/PurgeNow/Shutdown) back to the engine via a dedicated `mpsc` channel.
- **Backpressure-aware event stream** — TUI layer has a bounded `mpsc` + overflow buffer that drops oldest non-lifecycle events under pressure; dropped count surfaces in the footer.
- **Mock Azure AI Search** — `quelch mock` now serves Azure index/doc/search routes in-process for local testing, plus a `POST /azure/_fault` endpoint that injects 429/5xx on the next N calls.
- **Multi-subsource mock fixtures** — mock data now includes a second Jira project (DEMO) and a second Confluence space (INFRA) so the per-subsource UI is visible out of the box.
- **`Embedder` trait** — the engine now takes `&dyn Embedder`; ailloy is the production impl, `DeterministicEmbedder` is the test-only network-free impl.
- **End-to-end integration test** — full pipeline (Jira + Confluence + Azure + deterministic embedder) runs against localhost mock routes, including v1→v2 state migration and fault-injection retry coverage.

### Changed
- **State file schema v2** — `.quelch-state.json` now tracks per-subsource cursors; v1 files migrate automatically on first load (the legacy source-wide cursor is copied into each configured subsource — safe because Azure push is upsert).
- **`SourceConnector` trait** — `subsources()`, `fetch_changes(subsource, ...)`, `fetch_all_ids(subsource)`. Internal API change only.
- **Engine loop restructured** — iterates per-subsource with command polling at every loop boundary; per-subsource failures no longer abort sibling subsources in the same source.
- **`SearchClient` instruments Azure responses** — every request emits a `phase = "azure_response"` tracing event so the TUI can render live throughput and latency.
- **Terminal guard** — TUI restores raw mode and the main screen on clean exit OR on panic.

### Fixed
- Shutdown and mid-subsource interrupt paths now emit `cycle_finished` / `subsource_finished` phase events so the TUI never shows a stuck "syncing" state after Ctrl-C.

## [0.3.1] - 2026-04-16

### Added
- **`generate-agent` command** — Generates Copilot Studio agent configuration (OnKnowledgeRequested topics, agent instructions, setup guide) tailored to your quelch.yaml config
- **Copilot Studio documentation** — `docs/copilot-studio-onknowledgerequested.md` explaining the OnKnowledgeRequested trigger for custom knowledge sources

### Changed
- Updated dependencies (axum, clap, tokio, hyper-rustls, rustls-webpki, bitflags)

## [0.3.0] - 2026-04-14

### Added
- **Orphan purge** — `sync --purge` deletes documents from Azure that no longer exist in the source; automatic in watch mode

### Changed
- Increased search truncation limits for better result display
- Bumped MSRV to 1.94.1
- Updated upload-artifact action to v5 in release workflow

## [0.2.0] - 2026-04-14

### Added
- **Config module** — YAML config loading with environment variable substitution (`${VAR}`)
- **Jira connector** — Supports both Cloud (v3 API with ADF) and Data Center (v2 API, versions 9.12-10.x)
- **Confluence connector** — Cloud and Data Center support with heading-based page chunking
- **Azure AI Search client** — Index creation, document push with mergeOrUpload, retry with exponential backoff
- **Vector search** — Embeddings via ailloy (Azure OpenAI), HNSW index with scalar quantization, semantic reranking
- **Sync engine** — Incremental sync with cursor-based high-water marks, crash-safe state persistence
- **Labeled content** — Structured content field with field labels (Assignee:, Reporter:, Status:, etc.) for better semantic search
- **CLI commands** — sync, watch, setup, status, reset, reset-indexes, validate, init, search, mock, ai
- **Search command** — Semantic search from the terminal with colored output, relevance bars, and clickable URLs
- **Mock server** — Built-in Jira DC and Confluence DC mock server with 17 issues and 8 pages about quelch
- **AI integration** — `quelch ai` command for embedding model configuration via ailloy
- **Dual auth** — Cloud (email + API token, Basic Auth) and Data Center (PAT, Bearer) authentication
- **Index management** — `setup` creates indexes with vector search config, `reset-indexes` deletes and clears state
- CI workflow (check, test, clippy, fmt)
- Release workflow (cross-platform binaries, GitHub Release, Homebrew tap, crates.io)
- 84 tests (77 unit + 7 integration with wiremock)

## [0.1.0] - 2026-04-13

### Added
- Initial project scaffold with CLI subcommands
- CI and release workflows
