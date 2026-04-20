# Quelch TUI Redesign + Local Simulator — Design Spec

**Date:** 2026-04-20
**Status:** Approved (pending implementation plan)
**Scope:** Adds a `quelch sim` subcommand that runs quelch's real engine against a fully in-process simulated world (mock Jira, Confluence, Azure AI Search, and embedder) with realistic activity generation, and redesigns the TUI to fix the layout, labeling, charting, status-indication, and keybinding issues in v0.4.0.
**Supersedes:** none. **Builds on:** `docs/superpowers/specs/2026-04-19-quelch-tui-design.md` (the initial TUI + engine refactor).

## 1. Goals

1. A single command — `quelch sim` — that spins up the full simulated world and runs the real quelch engine against it, with realistic Jira and Confluence activity (create/update events at a few-per-minute cadence with bursts and lulls), simulated Azure AI Search faults, and simulated embedder latency.
2. Both TUI and plain-log modes must work under `sim`. CI runs `quelch sim --duration 30s --seed 42 --no-tui --assert-docs 20` on every push.
3. The TUI must address each concrete complaint raised against v0.4.0: confusing labels, no column headings, misaligned content rendering outside borders, missing time-series graphs, no clear running/paused indicator, duplicated or missing keybindings in the footer, unintuitive interaction.
4. No real Jira, Confluence, Azure AI Search, or LLM in test or dogfood paths — everything runs locally.

### Non-goals

- Cross-platform TUI polish (Windows Terminal, older Terminal.app) — manual test only.
- Mouse support.
- Long-run leak detection for the simulator.
- Proving that simulated Azure latency matches real Azure — the jitter model is synthetic.
- An Application Insights exporter — still deferred.

## 2. Architecture

### 2.1 Module layout

New modules under `crates/quelch/src/`:

```
sim/                           NEW — simulator subsystem
├── mod.rs                     pub entry: run(opts) -> Result<()>
├── opts.rs                    SimOpts struct
├── world.rs                   starter corpus seeder
├── scheduler.rs               burst-aware activity scheduler
├── jira_gen.rs                Jira-specific mutations
├── confluence_gen.rs          Confluence-specific mutations
├── azure_faults.rs            ties scheduler to the mock's /_fault endpoint
└── embedder.rs                SimEmbedder (jittery wrapper over DeterministicEmbedder)

mock/                          EXTENDED — gains /_sim/* mutation endpoints
├── mod.rs                     (existing file expanded)
└── data.rs                    (unchanged)

tui/                           REDESIGNED
├── mod.rs                     (updated entry with EventStream for keypress latency)
├── app.rs                     (new fields: spinner, recent_docs ring, drilldown_open)
├── layout.rs                  (REWRITTEN: table-based sources, full-width Azure chart)
├── status.rs                  NEW — "what should the header say right now"
├── spinner.rs                 NEW — tiny frame-based spinner
├── widgets/
│   ├── source_table.rs        NEW — replaces source_card.rs
│   ├── azure_panel.rs         REWRITTEN — real Chart widget
│   ├── drilldown.rs           NEW — per-subsource detail pane
│   ├── help_overlay.rs        NEW — modal keybinding reference
│   └── log_view.rs            UPDATED — column layout
└── input.rs                   UPDATED — Enter opens drilldown, Esc closes overlays, ? help
```

### 2.2 Data flow

```
sim::scheduler (tokio task)
  ├─ burst-aware Poisson tick: 100–2000ms dwell
  └─ picks source by weighted RNG, POSTs to mock /_sim/* endpoint
                 │
                 ▼
           mock axum server (one process, random port)
           ├─ updates in-memory store; bumps "updated" timestamp
           └─ returns updated item that surfaces on next cursor fetch

quelch engine (unchanged!) polls mock Jira/Confluence over HTTP
  → SimEmbedder (Embedder trait: jittery sleep + deterministic vector)
  → mock Azure AI Search (with /_fault injection)
  → emits real tracing events → TuiLayer → TUI renders
```

All orchestration in the `quelch sim` process. No external services.

## 3. Simulator

### 3.1 `SimOpts`

```rust
#[derive(Debug, Clone)]
pub struct SimOpts {
    pub duration: Option<Duration>,       // None = run until Ctrl-C
    pub seed: Option<u64>,                // OS-random if None
    pub rate_multiplier: f64,             // default 1.0
    pub fault_rate: f64,                  // 0.0–1.0, default 0.03
    pub assert_docs: Option<u64>,         // CI smoke-test threshold
    pub mock_port: Option<u16>,           // None = random loopback port
}
```

### 3.2 Starter corpus

On `sim::run(opts)`:

1. Bind the mock axum server to a random loopback port (or `--mock-port` if set).
2. Seed the mock's in-memory store:
   - **Jira QUELCH**: 40 issues (existing 17 from `mock/data.rs` + 23 generated).
   - **Jira DEMO**: 15 issues (existing 2 + 13 generated).
   - **Confluence QUELCH**: 20 pages (existing 8 + 12 generated).
   - **Confluence INFRA**: 8 pages (existing 2 + 6 generated).
3. Construct a `Config` pointing at the local mock + a `SimEmbedder`.
4. Spawn the activity scheduler, the quelch engine, and (if TUI) the TUI runner.
5. Wait on Ctrl-C or `opts.duration` elapsed; then shut down cleanly.

### 3.3 Activity scheduler

Burst-aware Poisson. Three modes (`Normal`, `Burst`, `Lull`) with seeded transitions:

| Mode | Dwell between mutations |
|---|---|
| Normal (default) | 2 000 – 8 000 ms |
| Burst | 100 – 500 ms |
| Lull | 5 000 – 15 000 ms |

Transitions: every ~90s there's a 30% chance of entering `Burst` for 30–60 s; every ~120 s, 20% chance of `Lull` for 60–90 s. `rate_multiplier` scales all dwell times (`dwell / multiplier`).

Per mutation, the scheduler:

- Picks Jira (~65% weight) or Confluence (~35% weight).
- Picks subsource: QUELCH (70%) or DEMO/INFRA (30%) — weighted by corpus size so realism scales.
- Picks action: **create** vs **update**. Jira: 30% / 70%. Confluence: 15% / 85%.
- For **update**, picks a random existing item from the mock's current state and bumps its `summary`/`description`/`body` with generated filler, then bumps `updated`.
- For **create**, generates a new unique key/id and inserts it.

### 3.4 Mock mutation endpoints (added to `crates/quelch/src/mock/mod.rs`)

All under the `/_sim/` path prefix so they're clearly out-of-band from real client APIs.

| Route | Method | Body | Effect |
|---|---|---|---|
| `/_sim/jira/upsert_issue` | POST | `{project, key, summary, description}` | insert or update; set `updated = now` |
| `/_sim/confluence/upsert_page` | POST | `{space, id, title, body}` | insert or update; bump `version.when` |
| `/_sim/jira/comment` | POST | `{key, body, author}` | append comment; bump `updated` |

All routes are `pub` only within the mock module (registered via `build_router()` which is itself gated — see §7.4).

### 3.5 `SimEmbedder`

```rust
pub struct SimEmbedder {
    inner: DeterministicEmbedder,
    rng: Mutex<StdRng>,
}

impl Embedder for SimEmbedder {
    async fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let jitter_ms = {
            let mut r = self.rng.lock().unwrap();
            if r.gen::<f32>() < 0.01 {
                r.gen_range(300..=500)  // 1% long tail
            } else {
                r.gen_range(20..=150)   // normal
            }
        };
        tokio::time::sleep(Duration::from_millis(jitter_ms)).await;
        self.inner.embed_one(text).await
    }
}
```

Makes the Azure panel's p50/p95 chart meaningfully different.

### 3.6 Azure fault injection

The simulator periodically (expected one fault per ~30 Azure requests at `fault_rate = 0.03`) posts to the mock's existing `POST /azure/_fault` with `{count: 1, status: 429}` or `{count: 1, status: 503}`. The engine's retry logic in `azure/mod.rs::request_with_retry` catches these and emits `BackoffStarted` / `BackoffFinished` tracing events the TUI displays.

### 3.7 CLI surface

New subcommand in `cli.rs`:

```rust
/// Run quelch against a fully simulated environment for local testing and CI.
Sim {
    #[arg(long)]
    duration: Option<humantime::Duration>,
    #[arg(long)]
    seed: Option<u64>,
    #[arg(long, default_value = "1.0")]
    rate_multiplier: f64,
    #[arg(long, default_value = "0.03")]
    fault_rate: f64,
    #[arg(long)]
    assert_docs: Option<u64>,
},
```

Inherits the existing global flags (`--no-tui`, `--json`, `--quiet`, `-v`). Defaults: run forever, OS-random seed, realistic activity, 3% faults. `humantime` crate added to workspace deps; `--duration 30s`, `--duration 2m`, `--duration 1h` all parse cleanly.

### 3.8 Log verbosity in sim mode

When the resolved command is `Sim`, the default plain-log filter becomes `quelch=warn,sim=info`. `-v` bumps to `quelch=info,sim=debug`; `-vv` to `quelch=debug,sim=trace,reqwest=debug`. TUI mode unaffected.

### 3.9 Shutdown protocol

One top-level `tokio_util::sync::CancellationToken`. `tokio::select!` waits on: Ctrl-C, `opts.duration` elapsed, TUI quit, or unexpected panic from a child task. On trigger:

1. Cancel token.
2. Scheduler exits its loop.
3. Engine's `poll_commands` picks up a synthetic `Shutdown` and exits.
4. TUI exits (if running) and saves prefs.
5. Mock server is dropped with the runtime.
6. Evaluate `--assert-docs` if set; fail with exit 1 if not met.
7. Print a one-line summary: `sim: 30.0s, 47 docs synced, 128 azure requests, 3 faults injected`.

## 4. TUI redesign

### 4.1 Overall frame

Single rounded outer border. Three sections separated by `├─ title ─┤` dividers (not inner boxes). Header is a strip, Sources + Azure AI Search are stacked vertically, Footer is a strip.

```
┌ quelch 0.4.0 · [engine status] · cycle N · last Ns ago · next Ns   uptime H:MM:SS ┐
│                                                                                   │
├─ Sources ────────────────────────────────────────────────────────────────────────┤
│  Source             Status       Items   Rate      Last item         Updated     │
│  ─────────────────  ───────────  ──────  ────────  ────────────────  ────────    │
│  ▾ my-jira          ● idle       87      —         PROJ-1432         23s ago     │
│    ▸ QUELCH         ● idle       62      0.3/min   PROJ-1432         23s ago     │
│      DEMO           ● idle       25      0.0/min   DEMO-2            1m 12s ago  │
│  ▾ my-confluence    ◐ syncing    41      4.2/min   page 985421       now         │
│      QUELCH         ● idle       33      0.0/min   page 100008       3m ago      │
│    ▸ INFRA          ◐ syncing    8       4.2/min   page 200002       now         │
├─ Azure AI Search ────────────────────────────────────────────────────────────────┤
│  Requests per second (last 60s)                              max 12 req/s        │
│   12 ┤                                                                           │
│    8 ┤       ▂▅▇█▇▅▂                                                             │
│    4 ┤     ▃▇█████▇▅▂        ▂▅▇▇▅▂                                              │
│    0 ┼──────█████████──────██████─▂▅█▇▅▂───▂▅▇▇▅▂──────────────────              │
│      -60s                                                          now           │
│  Total requests  128    Latency     median 52 ms · 95th 287 ms                   │
│  Failed (4xx)    0      Throttled (429)  2                                       │
│  Failed (5xx)    1      Dropped events   0                                       │
├──────────────────────────────────────────────────────────────────────────────────┤
│ ↑↓ select · ←/→ collapse · enter details · r sync now · p pause · s logs · ? help · q quit │
└──────────────────────────────────────────────────────────────────────────────────┘
```

Rendered via careful `Rect` math so every widget's drawable area is exactly `Block::inner(area)` of its container — never writes outside.

### 4.2 Header (`tui/status.rs`)

Centralises header content. Single line. Content by state:

| Engine state | Header |
|---|---|
| Idle, never synced | `○ Ready · no cycles yet · press r to sync now` |
| Idle, caught up | `● Watching · cycle N · last Ns ago · next in Ns` |
| Syncing | `◐ Syncing · my-jira/QUELCH · batch 3/7 · 10.1 docs/min` (spinner animates) |
| Paused | `⏸ Paused · cycle N · press p to resume` (yellow) |
| Backing off | `◉ Azure client backing off · 30s remaining` (amber) |
| Shutdown | `⏹ Shutting down` (dim) |

Uptime tracked from process start; shown right-aligned.

### 4.3 Sources pane (`tui/widgets/source_table.rs`)

Uses ratatui's `Table` widget (NOT `Paragraph`). Column widths declared at build time:

| Column | Width | Content |
|---|---|---|
| Source | 20 cols | `▾/▸` + name (bold at depth 0) |
| Status | 13 cols | coloured glyph + word |
| Items | 8 cols | right-aligned u64 |
| Rate | 10 cols | `X.X/min` or `—` |
| Last item | 18 cols | `last_sample_id` or `—` |
| Updated | remaining | "now", "Ns ago", "Nm ago" |

**Colour/glyph by state:**

| State | Glyph + colour |
|---|---|
| Idle | `●` green |
| Syncing | `◐` cyan (spinner rotates via `Spinner::glyph()`, 3 Hz) |
| Error | `●` red |
| Backoff | `◉` amber |
| Paused | `◌` dim |

**Collapsed source:** renders `◂ name` as the source row, no child rows. Expanded: `▾ name` plus indented child rows. Selected row uses `Style::default().add_modifier(Modifier::REVERSED)` for inverse-video highlight; the `▸` caret points at the selected row.

### 4.4 Azure panel (`tui/widgets/azure_panel.rs`)

Uses ratatui's `Chart` + `Dataset` (Canvas-backed braille plotting). Single line series over 60 seconds, y-axis auto-scaled, max value shown in the pane subtitle (`max 12 req/s`). X-axis labelled `-60s` and `now`.

Thin red tick-mark row appears ABOVE the x-axis for seconds where any 5xx/429 occurred.

Below the chart, a two-column counter strip:

```
Total requests  128    Latency     median 52 ms · 95th 287 ms
Failed (4xx)    0      Throttled (429)  2
Failed (5xx)    1      Dropped events   0
```

All labels plain English. Values coloured by meaning: zeros dim, failures red, throttled amber.

### 4.5 Spinner (`tui/spinner.rs`)

```rust
pub struct Spinner { tick: u32 }

impl Spinner {
    const FRAMES: &[char] = &['◐', '◓', '◑', '◒'];
    pub fn glyph(&self) -> char { Self::FRAMES[(self.tick as usize / 2) % 4] }
    pub fn tick(&mut self) { self.tick = self.tick.wrapping_add(1); }
}
```

`tui::run` calls `app.spinner.tick()` every redraw (5 Hz → 4 frames per ~1.6 s → smooth visible rotation).

### 4.6 Drilldown pane (`tui/widgets/drilldown.rs`)

Triggered by `Enter` on a focused subsource row. Takes over the right ~50% of the Sources pane; the table narrows to compensate. `Esc` closes.

```
├─ Sources ─────────────────────────────┬─ QUELCH (my-jira) ──────────────────┤
│ Source        Status       Items      │ Status         ● idle                │
│ ▾ my-jira     ● idle       87         │ Docs synced    62                    │
│   ▸ QUELCH    ● idle       62  ←row   │ Rate (60s)     0.3 per minute        │
│     DEMO      ● idle       25         │ Cursor         2026-04-19 14:32:01Z  │
│ …                                     │ Last item      PROJ-1432             │
│                                       │                                      │
│                                       │ Recent (10)                          │
│                                       │   ● 14:32:01  PROJ-1432              │
│                                       │   ● 14:31:58  PROJ-1431              │
│                                       │   …                                  │
│                                       │ Recent errors (last 3)               │
│                                       │   (none)                             │
```

Data sourced from a new per-subsource `recent_docs: VecDeque<RecentDoc>` (cap 10) populated in `App::apply` for every `DocSynced` event. Errors read from the existing `last_errors` ring.

### 4.7 Help overlay (`tui/widgets/help_overlay.rs`)

Triggered by `?`. Modal: centred bordered box; background content dimmed. Content grouped under Navigation / Actions / View / Other. Dismissed by `?` or `Esc`.

### 4.8 Log view (`tui/widgets/log_view.rs`)

Toggled by `s`. Replaces the Sources pane (Azure panel stays visible). Columns: `LEVEL` (5 cols) · `TIME` (8 cols) · `TARGET` (24 cols) · `MESSAGE` (remaining). Level colour-coded. Auto-scrolls to bottom.

### 4.9 Input handling (`tui/input.rs`)

Complete keymap:

| Key | Context | Action |
|---|---|---|
| `↑` `↓` | Sources focused | Move selection |
| `←` | Subsource | Move up to source row |
| `←` | Source expanded | Collapse |
| `→` | Source collapsed | Expand |
| `→` | Source expanded | Move into first subsource |
| `Tab` | any | Cycle focus between panes |
| `Enter` | subsource focused | Open drilldown |
| `Esc` | overlay open | Close |
| `r` | any | `UiCommand::SyncNow` |
| `p` | any | `UiCommand::Pause`/`Resume` + optimistic header |
| `R` (shift-r) | any | Arm; press again within 2s to reset cursor |
| `P` (shift-p) | source focused | Arm; press again within 2s to purge |
| `s` | any | Toggle log view |
| `c` | any | Clear footer flash |
| `?` | any | Toggle help overlay |
| `q`, `Ctrl-C` | any | Graceful shutdown |

### 4.10 Key-poll latency fix

Current implementation polls `crossterm::event::poll(Duration::from_millis(0))` once per 200ms redraw tick, giving up to 200ms keypress latency. Switch `tui::run` to use `crossterm::event::EventStream` (async) in a `tokio::select!` alongside the 5 Hz render tick. Keypresses drive an immediate render. Reduces latency to single-digit milliseconds.

## 5. Data plumbing additions

### 5.1 Engine emissions (fills gaps the v0.4.0 spec left)

- `SourceStarted` / `SourceFinished` (new phase strings `source_started` / `source_finished`): emitted at the start and end of each source's subsource loop in `sync/mod.rs::sync_with_connector`. Includes `docs_synced` and `duration_ms` on finish.
- `BackoffStarted` / `BackoffFinished` (new phase strings `backoff_started` / `backoff_finished`): emitted in `azure/mod.rs::request_with_retry` on either side of a `tokio::time::sleep` after a 429/5xx response. Field `source = "azure"` (the Azure client is the thing backing off, not a specific source).
- Phase constants added to `sync/phases.rs`:

```rust
pub const SOURCE_STARTED: &str = "source_started";
pub const SOURCE_FINISHED: &str = "source_finished";
pub const BACKOFF_STARTED: &str = "backoff_started";
pub const BACKOFF_FINISHED: &str = "backoff_finished";
```

### 5.2 TuiLayer mapping additions

`tracing_layer.rs` adds `match` arms for the four new phase strings above, producing the corresponding `QuelchEvent` variants.

### 5.3 `Prefs` additions

```rust
pub struct Prefs {
    pub version: u32,
    pub collapsed_sources: HashSet<String>,
    pub collapsed_subsources: HashMap<String, HashSet<String>>,
    pub log_view_on: bool,
    pub focus: String,
    // NEW fields — all with #[serde(default)]:
    pub selected_source: Option<String>,
    pub selected_subsource: Option<(String, String)>,
    pub drilldown_open: bool,
}
```

`CURRENT_PREFS_VERSION` stays at 1; all new fields default to `None`/`false`, so existing files load without migration.

### 5.4 Per-subsource recent-docs ring

```rust
pub struct SubsourceView {
    // ... existing fields ...
    pub recent_docs: VecDeque<RecentDoc>,   // cap 10
}

pub struct RecentDoc {
    pub ts: DateTime<Utc>,
    pub id: String,
}
```

Populated in `App::apply` for every `QuelchEvent::DocSynced`.

### 5.5 Azure chart samples helper

`metrics.rs::Throughput` already tracks 60 1-second buckets. Expose:

```rust
pub fn chart_points(&self) -> Vec<(f64, f64)> {
    self.buckets.iter().enumerate().map(|(i, (_, n))| (i as f64, *n as f64)).collect()
}
```

Consumed by `Chart::dataset`.

## 6. CI integration

New job in `.github/workflows/ci.yml`, runs after `test`:

```yaml
sim-smoke-test:
  name: Simulator smoke test
  runs-on: ubuntu-latest
  needs: test
  steps:
    - uses: actions/checkout@v5
    - uses: dtolnay/rust-toolchain@stable
    - run: cargo build --release -p quelch
    - name: Run simulator
      run: |
        ./target/release/quelch sim \
          --duration 30s \
          --seed 42 \
          --no-tui \
          --assert-docs 20
      timeout-minutes: 2
    - name: Upload sim log on failure
      if: failure()
      uses: actions/upload-artifact@v5
      with:
        name: sim-log
        path: sim.log
```

`--seed 42 --assert-docs 20` is calibrated against the starter corpus + 30 s of default activity. Tightening this threshold over time catches regressions (stalls, state-file bugs, cursor resets).

## 7. Testability

### 7.1 Unit tests

- `sim::scheduler` — deterministic given seed: with `tokio::time::pause()` + `advance`, seed 42 produces a specific sequence of mutation calls over 10 simulated seconds.
- `sim::world` — starter-corpus loads expected counts (40 + 15 Jira, 20 + 8 Confluence).
- `sim::embedder` — same vector as `DeterministicEmbedder` for identical input; reports latency > 20 ms per call.
- `tui::widgets::source_table` — renders fixture to `TestBackend`; assert heading row, column positions, `▾/▸` glyphs, selected-row inverse-video.
- `tui::widgets::drilldown` — renders expected values from a seeded subsource fixture.
- `tui::widgets::help_overlay` — renders; `?` and `Esc` dismiss.
- `tui::app` — `BackoffStarted` → `Backoff` state; `DocSynced` appends/caps `recent_docs`; `Enter` toggles `drilldown_open`.
- `tui::spinner` — glyph cycles through all four frames in 8 ticks.

### 7.2 Integration tests

- `crates/quelch/tests/sim_headless.rs` — new file. Spawns `quelch sim --duration 3s --seed 42 --no-tui --assert-docs 5` via `assert_cmd::Command::cargo_bin("quelch")`. Asserts exit status 0, stdout contains expected phase lines, state file ends up v2-valid. Requires adding `assert_cmd` to dev-deps.
- Extend `tests/end_to_end.rs` with `sim_produces_events_in_expected_order`: drives the scheduler for 1 second on a small corpus, asserts `CycleStarted` precedes `SubsourceBatch` in the event stream, and `BackoffStarted` appears at least once given `--fault-rate 1.0`.

### 7.3 Manual dogfood

- `quelch sim` — forever, TUI.
- `quelch sim --no-tui` — forever, plain log.
- `quelch sim --seed 42` — reproducible.
- `quelch sim --rate-multiplier 10 --duration 10s` — stress test.
- `quelch sim --fault-rate 0.5` — heavy fault injection for backoff display.

### 7.4 Security boundary for mock `/_sim/` routes

The mock server's `/_sim/*` routes are registered only inside `build_router()` which is already `pub`. The simulator is the only code that POSTs to these routes. A misbehaving actor on the local network would need to already reach localhost on the randomly-chosen mock port to exploit them. Acceptable risk for a local-testing mock; no authentication is added.

### 7.5 What we DO NOT test

- Cross-platform TUI colour rendering (Windows Terminal, older macOS Terminal.app).
- Long-run memory growth during hours-long `quelch sim` runs.
- Real-world Azure latency distribution fidelity.

## 8. Files touched

**New:**
- `crates/quelch/src/sim/` (whole module as §2.1)
- `crates/quelch/src/tui/status.rs`
- `crates/quelch/src/tui/spinner.rs`
- `crates/quelch/src/tui/widgets/source_table.rs` (replaces `source_card.rs`)
- `crates/quelch/src/tui/widgets/drilldown.rs`
- `crates/quelch/src/tui/widgets/help_overlay.rs`
- `crates/quelch/tests/sim_headless.rs`

**Modified:**
- `crates/quelch/src/cli.rs` — `Sim { .. }` subcommand.
- `crates/quelch/src/main.rs` — `Commands::Sim` dispatch to `sim::run`; sim-mode log filter defaults.
- `crates/quelch/src/lib.rs` — `pub mod sim;`.
- `crates/quelch/src/mock/mod.rs` — `/_sim/*` mutation endpoints.
- `crates/quelch/src/azure/mod.rs` — emit `BackoffStarted`/`BackoffFinished` in `request_with_retry`.
- `crates/quelch/src/sync/mod.rs` — emit `SourceStarted`/`SourceFinished`.
- `crates/quelch/src/sync/phases.rs` — 4 new constants.
- `crates/quelch/src/tui/events.rs` — (variants already declared, no change).
- `crates/quelch/src/tui/tracing_layer.rs` — match arms for new phases.
- `crates/quelch/src/tui/app.rs` — `Spinner`, `recent_docs` per subsource, drilldown state.
- `crates/quelch/src/tui/layout.rs` — rewritten layout with dividers, no inner boxes.
- `crates/quelch/src/tui/prefs.rs` — 3 new fields, all `#[serde(default)]`.
- `crates/quelch/src/tui/input.rs` — Enter opens drilldown; Esc closes overlays; `?` toggles help.
- `crates/quelch/src/tui/widgets/log_view.rs` — column layout.
- `crates/quelch/src/tui/widgets/azure_panel.rs` — rewritten for `Chart` + labelled counters.
- `crates/quelch/src/tui/mod.rs` — `EventStream`-based input; invoke help overlay / drilldown.
- `crates/quelch/src/tui/metrics.rs` — expose `chart_points()`.
- `.github/workflows/ci.yml` — add `sim-smoke-test` job.
- `Cargo.toml` — add `humantime`, `rand` (if not already), `tokio-util` (for `CancellationToken`), `assert_cmd` to dev-deps.

**Deleted:**
- `crates/quelch/src/tui/widgets/source_card.rs` — replaced by `source_table.rs`.

## 9. Rollout

- **No breaking CLI changes.** `sim` is additive. `--no-tui`, `reset --subsource`, etc. preserved.
- **Prefs file back-compat.** v1 prefs files continue to load; new fields default. No migration.
- **State file back-compat.** No state-file schema change in this spec; still v2.
- **Binary size impact.** +150–250 KB stripped (sim module + rng + tokio-util).
- **Release notes.** v0.5.0 highlights: `quelch sim` for local dogfood + CI, TUI redesigned (columns, real chart, live spinner, drilldown, help overlay).

## 10. Open hooks for future work

- Per-source mini-charts in the source table (once the full-width Azure chart is battle-tested).
- `quelch sim --scenario <name>` with canned scripted scenarios (catchup, noisy-space, flaky-network).
- Long-run leak surveillance (`quelch sim --duration 2h` in nightly CI).
- Cross-platform TUI snapshot testing.
