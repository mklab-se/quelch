# Quelch TUI & Monitorability — Design Spec

**Date:** 2026-04-19
**Status:** Approved (pending implementation plan)
**Scope:** This spec covers observability and the TUI only. A separate follow-up spec covers concurrency, rate-limit handling, and parallel worker pools.

## 1. Goals

1. Make it obvious at a glance what Quelch is doing — per source, per subsource (Jira project / Confluence space), and against Azure AI Search.
2. Default experience (`quelch sync` / `quelch watch`) is a live ratatui-based TUI. Plain-log mode remains available behind a flag and is auto-selected when stdout isn't a TTY.
3. The TUI is interactive — users can collapse/expand sections (state persists across runs), pause sync, force a sync-now, reset cursors, trigger a purge, and hot-switch to an in-pane log view.
4. Unify the observability stream on `tracing` so that the TUI, plain logs, and a future Application Insights exporter are all just subscribed layers — the sync engine emits structured events once and doesn't know who's listening.
5. Everything is runnable and testable locally with no real Jira, Confluence, Azure AI Search, or LLM backends.

### Non-goals (explicit, to keep scope sharp)

- Parallel fetching across sources or subsources.
- Shared worker pools for embedding or Azure writes.
- Adaptive backoff policies. (Events are emitted from day one; policies belong in the follow-up spec.)
- An Application Insights exporter. (Architecture makes adding one a pure additive layer.)
- Mouse support.

## 2. Architecture

### 2.1 Module layout

New modules under `crates/quelch/src/`:

```
tui/
├── mod.rs              pub entry: run(events_rx, cmd_tx) -> Result<()>
├── app.rs              App state: sources, subsources, azure stats, ui prefs
├── layout.rs           Stacked layout (header / sources / azure / footer)
├── widgets/
│   ├── source_card.rs  Collapsible source + subsource rows
│   ├── azure_panel.rs  Sparklines + counters
│   └── log_view.rs     `s`-hotkey in-pane tracing log
├── input.rs            Crossterm event loop → UiCommand
├── events.rs           QuelchEvent enum (engine → ui)
├── prefs.rs            .quelch-tui-state.json load/save
└── tracing_layer.rs    tracing::Subscriber::Layer that emits QuelchEvent
```

### 2.2 Data flow

```
sync engine ── tracing::info!/warn!/error! ──▶ tracing_layer ── mpsc ──▶ TUI app state ── render ──▶ ratatui
     ▲                                                                        │
     └──────────────── mpsc::Receiver<UiCommand> ─────────────────────────────┘
                  (Pause / Resume / SyncNow / ResetCursor / PurgeNow / Shutdown)
```

- The engine never imports from `tui::`. It emits structured `tracing` events and reads commands from an `mpsc::Receiver<UiCommand>` passed into the sync loop.
- `tracing_layer` replaces `tracing_subscriber::fmt` when TUI mode is on. In plain-log mode, `fmt` is installed instead. A future Application Insights layer adds itself alongside without touching the engine.
- Non-TTY auto-falls back to plain-log mode regardless of flags.

### 2.3 Binary layering

- `main.rs` decides the observability mode:
  - `--no-tui` set, OR stdout is not a TTY, OR `--json` is set → plain-log path.
  - Otherwise → TUI path.
- `lib.rs` exposes a new `sync::run_sync_with(cmd_rx, ...)` variant that the TUI path uses. The existing `run_sync` becomes a thin wrapper that passes a never-firing `Receiver`.

## 3. Event & command protocol

### 3.1 `QuelchEvent` (engine → ui)

```rust
pub enum QuelchEvent {
    CycleStarted { cycle: u64, at: DateTime<Utc> },
    CycleFinished { cycle: u64, duration: Duration },

    SourceStarted  { source: SourceId },
    SourceFinished { source: SourceId, docs_synced: u64, duration: Duration },
    SourceFailed   { source: SourceId, error: String },

    SubsourceStarted  { source: SourceId, subsource: SubsourceId },
    SubsourceFinished { source: SourceId, subsource: SubsourceId, cursor: DateTime<Utc> },
    SubsourceFailed   { source: SourceId, subsource: SubsourceId, error: String },
    SubsourceBatch    { source: SourceId, subsource: SubsourceId,
                        fetched: u64, cursor: DateTime<Utc>, sample_id: String },

    DocSynced { source: SourceId, subsource: SubsourceId, id: String, updated: DateTime<Utc> },
    DocFailed { source: SourceId, subsource: SubsourceId, id: String, error: String },

    AzureRequest  { at: Instant, method: String, path: String },
    AzureResponse { at: Instant, status: u16, latency: Duration, throttled: bool },

    BackoffStarted  { source: SourceId, until: DateTime<Utc>, reason: String },
    BackoffFinished { source: SourceId },

    Log { level: Level, target: String, message: String, ts: DateTime<Utc> },
}
```

The tracing layer maps span/event fields to these variants. Events the layer can't classify become `Log { … }`.

### 3.2 `UiCommand` (ui → engine)

```rust
pub enum UiCommand {
    Pause,
    Resume,
    SyncNow,                                                 // kicks watch out of its sleep
    ResetCursor { source: SourceId, subsource: Option<SubsourceId> },
    PurgeNow { source: SourceId },
    Shutdown,
}
```

### 3.3 Channel sizing & backpressure

- Events `mpsc` at capacity 1024. Commands `mpsc` at capacity 16.
- If the event channel is full, the tracing layer drops the **oldest non-lifecycle event** (i.e., drop old `DocSynced`/`Log` first; never drop `CycleStarted`/`CycleFinished`/`Source*`/`Subsource*`/`Backoff*`/`AzureResponse`).
- Dropped event count is tracked in the layer and surfaced in the TUI footer (`drops: N`). The engine never blocks on the layer.

## 4. Per-subsource engine refactor

### 4.1 Connector trait

```rust
pub trait SourceConnector: Sync {
    fn source_type(&self) -> &str;
    fn source_name(&self) -> &str;
    fn index_name(&self) -> &str;

    fn subsources(&self) -> &[String];                       // NEW

    async fn fetch_changes(                                  // CHANGED
        &self,
        subsource: &str,
        cursor: Option<&SyncCursor>,
        batch_size: usize,
    ) -> Result<FetchResult>;

    async fn fetch_all_ids(&self, subsource: &str) -> Result<Vec<String>>;   // CHANGED
}
```

- Jira: `subsources()` returns project keys; JQL becomes `project = "PROJ" AND updated >= ...`.
- Confluence: `subsources()` returns space keys; CQL becomes `space.key = "ENG" AND lastmodified >= ...`.

### 4.2 Engine loop (still sequential in this spec)

```text
for source in config.sources:
    for subsource in source.subsources():
        loop batches:
            check ui commands (non-blocking poll)
            fetch_changes(subsource, cursor)
            embed_batch
            push_batch
            state.update(source, subsource, cursor, count)
            persist state
```

Command polling happens at every loop boundary: between sources, between subsources, between batches, and before each `await sleep` in watch mode. `Pause` holds at the top of the source/subsource loop (current in-flight batch finishes first). `SyncNow` cancels the watch sleep; it's a no-op in one-shot `sync`. `Shutdown` exits the nearest loop cleanly, flushes state, and returns.

### 4.3 State file schema (v2)

```json
{
  "version": 2,
  "sources": {
    "my-jira-cloud": {
      "last_sync_at": "2026-04-19T...",
      "sync_count": 12,
      "subsources": {
        "PROJ": { "last_cursor": "...", "last_sync_at": "...", "documents_synced": 89, "last_sample_id": "PROJ-1432" },
        "HR":   { "last_cursor": "...", "last_sync_at": "...", "documents_synced": 54, "last_sample_id": "HR-221" }
      }
    }
  }
}
```

### 4.4 Migration v1 → v2

On load, if `version == 1`, copy the old per-source `last_cursor` into **every** configured subsource of that source. The old cursor was the min across all projects/spaces by construction, so this is safe (at worst it re-syncs a minute's worth of docs — harmless, since Azure push is upsert). Write out as v2 on first save. Log one `info!` message.

### 4.5 Reset semantics

- `quelch reset <source>` stays source-level (clears all subsources).
- New `--subsource=<KEY>` flag clears a single subsource's cursor.
- TUI `ResetCursor { source, subsource: Option<...> }` matches.

### 4.6 Purge

- `fetch_all_ids` runs per subsource.
- Index-side: query filtered by the subsource's stable id prefix (e.g., `id startswith 'test-jira-PROJ-'`). Fallback if per-subsource filtering is expensive: fetch all index ids once, partition client-side by id prefix.

## 5. TUI state model, UI prefs, keybindings

### 5.1 Layout (stacked)

```
┌─ quelch vX.Y.Z  ● watching · cycle N · up Hh Mm ──────────── ? help ─┐
│                                                                       │
│ ┌─ Sources ───────────────────────────────────────────────────────┐  │
│ │ ▼ my-jira-cloud      [idle]     last 2m ago  +143 docs  2.4/min │  │
│ │   ▶ PROJ                                  +89  docs  PROJ-1432  │  │
│ │   ▶ HR                                    +54  docs  HR-221     │  │
│ │ ▼ my-confluence     [syncing]  batch 3/7          10.1/min      │  │
│ │   ▶ ENG                                   +312 docs  985421     │  │
│ │   ▶ OPS             [error]    connection refused · retry 30s   │  │
│ └─────────────────────────────────────────────────────────────────┘  │
│                                                                       │
│ ┌─ Azure AI Search ───────────────────────────────────────────────┐  │
│ │ ▂▄█▆▃▂▃▇█▆▄▃▂▃▄▅▇█▆▄▃  req/s (60s)                             │  │
│ │ ·_·__·___·__·____·__·  5xx/s                                    │  │
│ │ total 12,483 · p50 84ms · p95 312ms · 4xx 0 · 5xx 2 · throttled 1 │
│ └─────────────────────────────────────────────────────────────────┘  │
│                                                                       │
└─ q quit  space collapse  r sync-now  p pause  s logs  tab focus ─────┘
```

### 5.2 `App` state

```rust
pub struct App {
    pub sources: Vec<SourceView>,
    pub azure: AzurePanel,
    pub prefs: Prefs,
    pub status: EngineStatus,         // Idle | Syncing{cycle,since} | Paused | Shutdown
    pub focus: Focus,
    pub footer: FooterMsg,
    pub drops: u64,
    pub log_tail: VecDeque<LogLine>,  // ring, cap 500
}

pub struct SourceView {
    pub name: String,
    pub kind: SourceKind,
    pub state: SourceState,           // Idle | Syncing | Error(String) | Backoff{until}
    pub subsources: Vec<SubsourceView>,
    pub collapsed: bool,
    pub aggregate: Throughput,
}

pub struct SubsourceView {
    pub key: String,
    pub state: SubsourceState,
    pub last_cursor: Option<DateTime<Utc>>,
    pub last_sample_id: Option<String>,
    pub docs_synced_total: u64,
    pub last_errors: VecDeque<String>, // cap 3
}
```

### 5.3 Throughput primitive

Ring buffer, 60 entries @ 1s granularity. Each `SubsourceBatch` pushes `fetched` into the current-second bucket; `docs/min` is the sum across the ring window. The Azure panel uses the same primitive for req/s. Source-level aggregate throughput is the sum of its subsources' throughputs, not a separate counter.

### 5.4 UI prefs (`.quelch-tui-state.json`)

```json
{
  "version": 1,
  "collapsed_sources": ["my-jira-cloud"],
  "collapsed_subsources": {"my-confluence": ["OPS"]},
  "log_view_on": false,
  "focus": "sources"
}
```

Loaded on startup; saved atomically (temp-write + rename) on every prefs mutation and on clean shutdown. Parse failure → warn and fall back to defaults.

### 5.5 Keybindings

| Key | Action |
|---|---|
| `↑ / ↓ / j / k` | move focus |
| `space` / `enter` | collapse/expand focused source or subsource |
| `tab` | cycle focus (sources → azure → sources) |
| `r` | `UiCommand::SyncNow` |
| `p` | toggle pause/resume |
| `R` (shift-r) | reset cursor on focused source/subsource (confirm-within-2s) |
| `P` (shift-p) | purge-now on focused source (confirm-within-2s) |
| `s` | toggle in-pane log view |
| `c` | clear footer flash / dismiss last-error |
| `?` | help modal overlay |
| `q` / `Ctrl-C` | `UiCommand::Shutdown`, save prefs, restore terminal |

### 5.6 Frame rate

5 Hz redraw (200 ms). Events coalesce between frames. Panic during render is caught by a `Drop` guard that restores the terminal before the panic propagates.

## 6. Azure panel

- Two ring buffers: 60s at 1s granularity (throughput) and 5000-event bounded deque (latency percentiles).
- Sparkline uses `ratatui::widgets::Sparkline`. Two lines: top line shaded by response status, bottom line shows 5xx spikes in red.
- Latency p50/p95 computed by sort over the last 5000 `AzureResponse` events (cheap, bounded).
- Counter strip: `total | p50 | p95 | 4xx | 5xx | throttled | drops`.
- `drops` comes from the tracing layer's backpressure counter.

## 7. Error handling

- Transient footer flash per error (2s fade, dismissable with `c`).
- Persistent `last_errors` on the source/subsource card for as long as state is `Error(…)`; retained across cycles until the next successful batch.
- TUI render panics: `Drop` guard restores the terminal, panic propagates with the restored state.
- Event channel closes unexpectedly (engine task died): TUI shows a full-pane red banner "sync engine exited: {reason}" and waits for `q`.

## 8. Local testability

No test may hit real Jira, Confluence, Azure AI Search, or an LLM.

### 8.1 Extend the `mock` module with Azure AI Search

Add routes to the existing axum server in `crates/quelch/src/mock/mod.rs`:

| Route | Method | Purpose |
|---|---|---|
| `/azure/indexes/{name}` | `GET` | exists check |
| `/azure/indexes/{name}` | `PUT` | create index |
| `/azure/indexes/{name}` | `DELETE` | delete index |
| `/azure/indexes/{name}/docs/index` | `POST` | upload batch (in-memory) |
| `/azure/indexes/{name}/docs/search` | `POST` | naive substring search over stored docs |
| `/azure/_fault` | `POST` | inject 429/5xx/latency on the next N calls (test-only) |

Storage: `Arc<Mutex<HashMap<String, IndexStore>>>` for the server's lifetime. `SearchClient` is unchanged — the user points the endpoint at `http://localhost:9999/azure`.

### 8.2 `Embedder` trait

```rust
pub trait Embedder: Send + Sync {
    async fn embed_one(&self, text: &str) -> Result<Vec<f32>>;
}

impl Embedder for ailloy::Client { ... }   // real path
```

The engine holds `&dyn Embedder`. `DeterministicEmbedder` (test-only) hashes the input text to a stable `[f32; N]`. No network.

Production wiring is unchanged — `main.rs` still calls `ailloy::Client::for_capability("embedding")` and passes it as `&dyn Embedder`.

### 8.3 Integration test (`crates/quelch/tests/end_to_end.rs`)

Spins up the axum server (Jira + Confluence + Azure) on a random port, wires the engine with `DeterministicEmbedder`, and runs the full `run_sync` pipeline. Asserts:
- Event order captured off the `mpsc` receiver.
- State file contents after a cycle (v2 schema).
- Migration v1 → v2 (seed a v1 file, run, check v2 shape).
- Plain-log mode produces expected output for the same inputs (capture stdout).
- 429 injection triggers `BackoffStarted`/`BackoffFinished` events.

### 8.4 TUI unit tests

`ratatui::backend::TestBackend`:
- Source card collapses/expands on `space`/`enter`.
- `s` toggles log view.
- Azure panel renders the sparkline after N sample events.
- Prefs round-trip.
- Non-TTY refusal to start.

### 8.5 Mock data

`mock/data.rs` gains a second Jira project and a second Confluence space so per-subsource UI is visible out of the box. `quelch mock` printout updates to match.

## 9. CLI changes (complete list)

- `--no-tui` (new, global): force plain-log mode. TUI is default for `sync`/`watch` otherwise. Non-TTY auto-falls back regardless.
- `--json` (existing): unchanged. Implies plain-log (structured). `--no-tui` is implied when `--json` is set.
- `--quiet` (existing): unchanged.
- `reset`: gains `--subsource=<KEY>` (optional).

No other CLI surface changes.

## 10. Rollout

- On first run after upgrade: v1 state file auto-migrated to v2. One `info!` log line, no user action.
- On first TUI run: no `.quelch-tui-state.json` → everything expanded, focus on first source.
- `.gitignore`: add `.quelch-tui-state.json` and `.superpowers/`.

## 11. Files touched

**New:**
- `crates/quelch/src/tui/` (whole module as listed in §2.1)
- `crates/quelch/src/sync/embedder.rs` (trait + impls + `DeterministicEmbedder`)
- `crates/quelch/tests/end_to_end.rs`

**Modified:**
- `crates/quelch/src/main.rs` — observability-mode decision, TUI vs plain-log wiring.
- `crates/quelch/src/cli.rs` — `--no-tui` flag, `reset --subsource`.
- `crates/quelch/src/lib.rs` — re-export `tui`.
- `crates/quelch/src/sources/mod.rs` — `SourceConnector` trait changes.
- `crates/quelch/src/sources/jira.rs` — per-project JQL, implement `subsources()`.
- `crates/quelch/src/sources/confluence.rs` — per-space CQL, implement `subsources()`.
- `crates/quelch/src/sync/mod.rs` — loop restructured for per-subsource cursors; accepts `mpsc::Receiver<UiCommand>`; emits structured tracing events with the fields the layer maps.
- `crates/quelch/src/sync/state.rs` — v2 schema + migration.
- `crates/quelch/src/mock/mod.rs` — Azure routes + `_fault` endpoint.
- `crates/quelch/src/mock/data.rs` — second project, second space.
- `.gitignore` — new entries.
- `Cargo.toml` — no new deps. `ratatui` and `crossterm` are already in the workspace dependency list.

## 12. Open hooks for the follow-up concurrency spec

- `BackoffStarted`/`BackoffFinished` events exist but are unused until spec B implements adaptive backoff.
- The `Embedder` trait is the natural seam for a pooled embedder in spec B.
- The per-subsource engine loop can be trivially parallelised in spec B — the data model and UI already treat subsources as independent.
