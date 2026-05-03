# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

## [2.0.0-alpha] - Unreleased

### Breaking

This is a complete re-architecture; v1 configs are not compatible. See
[/docs/](docs/) for the new architecture and the migration guide
(forthcoming) for upgrade steps.

### In progress

Implementation in progress; see
[docs/superpowers/plans/2026-05-03-quelch-v2-implementation.md](docs/superpowers/plans/2026-05-03-quelch-v2-implementation.md)
for the phase-by-phase plan.

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
