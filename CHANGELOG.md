# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

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
