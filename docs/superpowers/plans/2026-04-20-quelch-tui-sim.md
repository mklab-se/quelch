# Quelch TUI Redesign + `quelch sim` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a redesigned TUI that addresses v0.4.0's concrete usability failures (confusing labels, no columns, misaligned borders, missing graphs, no status indicators, duplicate footer) and a new `quelch sim` subcommand that runs the real engine against an in-process simulated world (mock Jira, Confluence, Azure, embedder) with realistic bursty activity.

**Architecture:** Engine unchanged in core — just adds missing tracing emissions (`source_started`, `source_finished`, `backoff_started`, `backoff_finished`). TUI replaces the `source_card` paragraph with a proper `Table` + dedicated panes for drilldown, help, real time-series chart. Simulator is a new `sim/` module orchestrated by a new `Commands::Sim` subcommand — spins up the existing mock axum server plus a burst-aware Poisson scheduler + a jittery `SimEmbedder`, running in one tokio runtime.

**Tech Stack:** Rust 2024, tokio, ratatui + crossterm, axum (existing mock), tokio-util (`CancellationToken`), rand (seeded RNG), humantime (CLI duration), assert_cmd (integration test).

**Pre-commit check (CLAUDE.md):** every task's final step runs `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`. Never skip.

**Reference spec:** `docs/superpowers/specs/2026-04-20-quelch-tui-sim-design.md`.

**Notes for executors:**
- The repo has three stray files from a previous session (`tmp.txt`, `build_output.json`, `build_output_clean.json`) at the root. Task 24 deletes them.
- `Prefs` uses name-based selection fields (spec §5.3). `App` continues to use `usize` indices internally for navigation; conversion happens at load/save boundaries.
- Where a widget file already exists (`azure_panel.rs`, `log_view.rs`), the task **rewrites** it rather than creating a new one — show the full new content.

---

## Task 1: Engine emissions — SourceStarted/Finished + BackoffStarted/Finished

Fills the four `QuelchEvent` variants declared but never emitted, enabling the TUI header, the per-source state transitions, and the Azure backoff banner.

**Files:**
- Modify: `crates/quelch/src/sync/phases.rs`
- Modify: `crates/quelch/src/sync/mod.rs` (emit source_started/finished around subsource loop)
- Modify: `crates/quelch/src/azure/mod.rs` (emit backoff_started/finished around retry sleep)
- Modify: `crates/quelch/src/tui/tracing_layer.rs` (map the four new phases)

- [ ] **Step 1: Write failing tests**

Add to `crates/quelch/src/tui/tracing_layer.rs`'s existing `mod tests`:

```rust
    #[tokio::test]
    async fn emits_source_started_and_finished() {
        use crate::sync::phases;
        use tracing_subscriber::prelude::*;

        let (layer, mut rx, _drops) = layer_and_receiver();
        let subscriber = tracing_subscriber::registry().with(layer);
        let _g = tracing::subscriber::set_default(subscriber);

        tracing::info!(phase = phases::SOURCE_STARTED, source = "my-jira", "start");
        tracing::info!(
            phase = phases::SOURCE_FINISHED,
            source = "my-jira",
            docs_synced = 42u64,
            duration_ms = 1234u64,
            "done"
        );

        let first = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match first {
            QuelchEvent::SourceStarted { source } => assert_eq!(source, "my-jira"),
            other => panic!("expected SourceStarted, got {other:?}"),
        }

        let second = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match second {
            QuelchEvent::SourceFinished {
                source,
                docs_synced,
                duration,
            } => {
                assert_eq!(source, "my-jira");
                assert_eq!(docs_synced, 42);
                assert_eq!(duration.as_millis(), 1234);
            }
            other => panic!("expected SourceFinished, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn emits_backoff_events() {
        use crate::sync::phases;
        use tracing_subscriber::prelude::*;

        let (layer, mut rx, _drops) = layer_and_receiver();
        let subscriber = tracing_subscriber::registry().with(layer);
        let _g = tracing::subscriber::set_default(subscriber);

        tracing::warn!(
            phase = phases::BACKOFF_STARTED,
            source = "azure",
            reason = "HTTP 429",
            delay_ms = 1000u64,
            "backoff"
        );
        tracing::info!(
            phase = phases::BACKOFF_FINISHED,
            source = "azure",
            "resumed"
        );

        let first = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(first, QuelchEvent::BackoffStarted { .. }));

        let second = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match second {
            QuelchEvent::BackoffFinished { source } => assert_eq!(source, "azure"),
            other => panic!("expected BackoffFinished, got {other:?}"),
        }
    }
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p quelch tui::tracing_layer::tests::emits_source_started_and_finished tui::tracing_layer::tests::emits_backoff_events`
Expected: FAIL — `phases::SOURCE_STARTED` etc. undefined.

- [ ] **Step 3: Add phase constants**

Edit `crates/quelch/src/sync/phases.rs` — append to the file:

```rust
pub const SOURCE_STARTED: &str = "source_started";
pub const SOURCE_FINISHED: &str = "source_finished";
pub const BACKOFF_STARTED: &str = "backoff_started";
pub const BACKOFF_FINISHED: &str = "backoff_finished";
```

- [ ] **Step 4: Map phases in TuiLayer**

In `crates/quelch/src/tui/tracing_layer.rs` `FieldVisitor`, add fields to pick up `docs_synced`, `duration_ms`, `reason`, `delay_ms`:

```rust
#[derive(Default)]
struct FieldVisitor {
    phase: Option<String>,
    source: Option<String>,
    subsource: Option<String>,
    doc_id: Option<String>,
    updated: Option<String>,
    cursor: Option<String>,
    fetched: Option<u64>,
    sample_id: Option<String>,
    status: Option<u64>,
    message: Option<String>,
    error: Option<String>,
    latency_ms: Option<u64>,
    throttled: Option<u64>,
    cycle: Option<u64>,
    duration_ms: Option<u64>,
    // NEW:
    docs_synced: Option<u64>,
    reason: Option<String>,
    delay_ms: Option<u64>,
}
```

Add to `record_str`:

```rust
    "reason" => self.reason = Some(v),
```

Add to `record_u64`:

```rust
    "docs_synced" => self.docs_synced = Some(value),
    "delay_ms" => self.delay_ms = Some(value),
```

Add to the `match v.phase.as_deref()` in `on_event`, before the fallback arm:

```rust
            Some(p) if p == crate::sync::phases::SOURCE_STARTED => {
                v.source.clone().map(|s| QuelchEvent::SourceStarted { source: s })
            }
            Some(p) if p == crate::sync::phases::SOURCE_FINISHED => v.source.clone().map(|s| {
                QuelchEvent::SourceFinished {
                    source: s,
                    docs_synced: v.docs_synced.unwrap_or(0),
                    duration: std::time::Duration::from_millis(v.duration_ms.unwrap_or(0)),
                }
            }),
            Some(p) if p == crate::sync::phases::BACKOFF_STARTED => {
                v.source.clone().map(|s| QuelchEvent::BackoffStarted {
                    source: s,
                    until: chrono::Utc::now()
                        + chrono::Duration::milliseconds(v.delay_ms.unwrap_or(0) as i64),
                    reason: v.reason.clone().unwrap_or_default(),
                })
            }
            Some(p) if p == crate::sync::phases::BACKOFF_FINISHED => v
                .source
                .clone()
                .map(|s| QuelchEvent::BackoffFinished { source: s }),
```

- [ ] **Step 5: Emit from engine**

In `crates/quelch/src/sync/mod.rs::sync_with_connector`, wrap the per-subsource loop with start/finish emissions. Find the existing `info!(source = source_name, "Starting source");` and `info!(source = source_name, "Finished source");` at the top and bottom of `sync_with_connector`. Replace with:

```rust
let source_started = std::time::Instant::now();
let mut total_docs_this_cycle: u64 = 0;
info!(
    phase = phases::SOURCE_STARTED,
    source = source_name,
    "Starting source"
);

for subsource_key in connector.subsources() {
    // ... existing poll_commands + sync_single_subsource with error catch ...
}

info!(
    phase = phases::SOURCE_FINISHED,
    source = source_name,
    docs_synced = total_docs_this_cycle,
    duration_ms = source_started.elapsed().as_millis() as u64,
    "Finished source"
);
```

Since `sync_single_subsource` returns `Result<()>` without exposing the doc count, add a return value. Change its signature to `Result<u64>` where u64 is `total_synced`, and accumulate in `sync_with_connector`:

```rust
match sync_single_subsource(...).await {
    Ok(n) => total_docs_this_cycle += n,
    Err(e) => {
        tracing::error!(
            phase = phases::SUBSOURCE_FAILED,
            source = source_name,
            subsource = subsource_key,
            error = %e,
            "Subsource failed"
        );
    }
}
```

Inside `sync_single_subsource`, before the final `Ok(())`, change to `Ok(total_synced)`.

- [ ] **Step 6: Emit backoff from Azure client**

In `crates/quelch/src/azure/mod.rs::request_with_retry`, find the existing `warn!("Retrying after {:?} (attempt {}/{})", ...)`. Replace with:

```rust
            if attempt > 0 {
                let delay = std::time::Duration::from_secs(1 << attempt);
                tracing::warn!(
                    phase = crate::sync::phases::BACKOFF_STARTED,
                    source = "azure",
                    reason = "HTTP 429 or 5xx",
                    delay_ms = delay.as_millis() as u64,
                    "Retrying after backoff"
                );
                tokio::time::sleep(delay).await;
                tracing::info!(
                    phase = crate::sync::phases::BACKOFF_FINISHED,
                    source = "azure",
                    "Backoff finished"
                );
            }
```

- [ ] **Step 7: Run tests + verify green**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

All new and existing tests must pass. The tracing-layer mapping tests from Step 1 now pass.

- [ ] **Step 8: Commit**

```bash
git add crates/quelch/src/sync/phases.rs crates/quelch/src/sync/mod.rs crates/quelch/src/azure/mod.rs crates/quelch/src/tui/tracing_layer.rs
git commit -m "$(cat <<'EOF'
Emit SourceStarted/Finished and BackoffStarted/Finished events

Fills engine-side gaps: the TUI header and per-source states depend on
these, and the Azure retry-backoff path had no observable signal. All
four variants were declared in QuelchEvent but never produced.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Spinner module + Prefs additions

Tiny supporting primitives. Commit together since they're both foundations for Task 3.

**Files:**
- Create: `crates/quelch/src/tui/spinner.rs`
- Modify: `crates/quelch/src/tui/prefs.rs` (add 3 fields)
- Modify: `crates/quelch/src/tui/mod.rs` (register `spinner`)

- [ ] **Step 1: Write failing tests**

Create `crates/quelch/src/tui/spinner.rs`:

```rust
//! Tiny frame-based spinner that cycles through four glyphs.

#[derive(Debug, Default)]
pub struct Spinner {
    tick: u32,
}

impl Spinner {
    const FRAMES: [char; 4] = ['◐', '◓', '◑', '◒'];

    /// Call once per redraw tick. At 5 Hz, glyph changes every two ticks
    /// — a full rotation every ~1.6 seconds.
    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn glyph(&self) -> char {
        Self::FRAMES[(self.tick as usize / 2) % 4]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles_through_all_frames() {
        let mut s = Spinner::default();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..8 {
            seen.insert(s.glyph());
            s.tick();
        }
        assert_eq!(seen.len(), 4);
    }

    #[test]
    fn glyph_changes_every_two_ticks() {
        let mut s = Spinner::default();
        let a = s.glyph();
        s.tick();
        assert_eq!(s.glyph(), a);
        s.tick();
        assert_ne!(s.glyph(), a);
    }
}
```

Extend the existing tests module in `crates/quelch/src/tui/prefs.rs` with:

```rust
    #[test]
    fn new_fields_default_to_none_and_false() {
        let p = Prefs::default();
        assert!(p.selected_source.is_none());
        assert!(p.selected_subsource.is_none());
        assert!(!p.drilldown_open);
    }

    #[test]
    fn old_file_without_new_fields_loads_cleanly() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("ui.json");
        std::fs::write(
            &path,
            r#"{"version":1,"collapsed_sources":[],"collapsed_subsources":{},"log_view_on":false,"focus":"sources"}"#,
        )
        .unwrap();
        let p = Prefs::load(&path).unwrap();
        assert_eq!(p.version, 1);
        assert!(p.selected_source.is_none());
    }
```

- [ ] **Step 2: Run fail**

Run: `cargo test -p quelch tui::spinner tui::prefs`
Expected: spinner tests fail to compile (no module); prefs tests fail (fields don't exist).

- [ ] **Step 3: Add Prefs fields**

In `crates/quelch/src/tui/prefs.rs`, extend `Prefs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefs {
    pub version: u32,
    #[serde(default)]
    pub collapsed_sources: HashSet<String>,
    #[serde(default)]
    pub collapsed_subsources: HashMap<String, HashSet<String>>,
    #[serde(default)]
    pub log_view_on: bool,
    #[serde(default = "default_focus")]
    pub focus: String,
    // NEW:
    #[serde(default)]
    pub selected_source: Option<String>,
    #[serde(default)]
    pub selected_subsource: Option<(String, String)>,
    #[serde(default)]
    pub drilldown_open: bool,
}
```

Update `Default for Prefs`:

```rust
impl Default for Prefs {
    fn default() -> Self {
        Self {
            version: CURRENT_PREFS_VERSION,
            collapsed_sources: HashSet::new(),
            collapsed_subsources: HashMap::new(),
            log_view_on: false,
            focus: default_focus(),
            selected_source: None,
            selected_subsource: None,
            drilldown_open: false,
        }
    }
}
```

- [ ] **Step 4: Register spinner module**

In `crates/quelch/src/tui/mod.rs`, add `pub mod spinner;` in the module list (alphabetical position between `prefs` and `tracing_layer`).

- [ ] **Step 5: Run + verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/quelch/src/tui/spinner.rs crates/quelch/src/tui/prefs.rs crates/quelch/src/tui/mod.rs
git commit -m "Add Spinner + persistent selection/drilldown Prefs fields

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: App additions — spinner, recent_docs per subsource, drilldown state

**Files:**
- Modify: `crates/quelch/src/tui/app.rs`

- [ ] **Step 1: Write failing tests**

Append to `crates/quelch/src/tui/app.rs`'s existing `mod tests`:

```rust
    #[test]
    fn doc_synced_appends_to_recent_docs_capped_at_ten() {
        let mut a = App::new(&cfg(), Prefs::default());
        for i in 0..15 {
            a.apply(QuelchEvent::DocSynced {
                source: "my-jira".into(),
                subsource: "DO".into(),
                id: format!("DO-{i}"),
                updated: Utc::now(),
            });
        }
        let recent = &a.sources[0].subsources[0].recent_docs;
        assert_eq!(recent.len(), 10);
        assert_eq!(recent.back().unwrap().id, "DO-14");
        assert_eq!(recent.front().unwrap().id, "DO-5");
    }

    #[test]
    fn enter_toggles_drilldown_open() {
        let mut a = App::new(&cfg(), Prefs::default());
        a.move_selection_down();
        assert_eq!(a.focused_subsource_name(), Some("DO"));
        assert!(!a.drilldown_open);
        a.toggle_drilldown();
        assert!(a.drilldown_open);
        a.toggle_drilldown();
        assert!(!a.drilldown_open);
    }

    #[test]
    fn spinner_glyph_available_on_app() {
        let a = App::new(&cfg(), Prefs::default());
        // glyph is a char from the spinner rotation
        let g = a.spinner_glyph();
        assert!(['◐', '◓', '◑', '◒'].contains(&g));
    }
```

- [ ] **Step 2: Run fail**

Run: `cargo test -p quelch tui::app`
Expected: FAIL — `recent_docs`, `drilldown_open`, `toggle_drilldown`, `spinner_glyph` don't exist.

- [ ] **Step 3: Implement**

At the top of `crates/quelch/src/tui/app.rs`, add:

```rust
use super::spinner::Spinner;
```

Extend `SubsourceView`:

```rust
pub struct SubsourceView {
    pub key: String,
    pub state: SubsourceState,
    pub last_cursor: Option<DateTime<Utc>>,
    pub last_sample_id: Option<String>,
    pub docs_synced_total: u64,
    pub last_errors: VecDeque<String>,
    pub throughput: Throughput,
    pub recent_docs: VecDeque<RecentDoc>, // NEW — cap 10
}

#[derive(Debug, Clone)]
pub struct RecentDoc {
    pub ts: DateTime<Utc>,
    pub id: String,
}

const RECENT_DOCS_CAP: usize = 10;
```

Initialize in `App::new`'s subsource construction, replace the existing `SubsourceView { ... }` with:

```rust
                        .map(|k| SubsourceView {
                            key: k,
                            state: SubsourceState::Idle,
                            last_cursor: None,
                            last_sample_id: None,
                            docs_synced_total: 0,
                            last_errors: VecDeque::new(),
                            throughput: Throughput::default(),
                            recent_docs: VecDeque::new(),
                        })
```

Add to `App`:

```rust
pub struct App {
    pub sources: Vec<SourceView>,
    pub azure: AzurePanel,
    pub prefs: Prefs,
    pub status: EngineStatus,
    pub focus: Focus,
    pub footer: String,
    pub log_tail: VecDeque<LogLine>,
    pub drops: u64,
    pub selected_source: usize,
    pub selected_subsource: Option<usize>,
    // NEW:
    pub spinner: Spinner,
    pub drilldown_open: bool,
    pub backoff_reason: Option<String>,
    pub backoff_until: Option<DateTime<Utc>>,
}
```

Initialize in `App::new` after the `selected_subsource: None,` line:

```rust
            spinner: Spinner::default(),
            drilldown_open: prefs_drilldown,
            backoff_reason: None,
            backoff_until: None,
```

Read `prefs_drilldown` from prefs before building the struct:

```rust
        let prefs_drilldown = prefs.drilldown_open;
```

(Put it right after `pub fn new(config: &Config, prefs: Prefs) -> Self {` opens, before the `let sources = ...`.)

Also, restore `selected_source`/`selected_subsource` from prefs names:

```rust
        // After `let sources = ...` collected, resolve name-based prefs to indices.
        let mut selected_source = 0usize;
        let mut selected_subsource: Option<usize> = None;
        if let Some(sel) = &prefs.selected_source
            && let Some(idx) = sources.iter().position(|s: &SourceView| &s.name == sel)
        {
            selected_source = idx;
            if let Some((src_name, sub_name)) = &prefs.selected_subsource
                && src_name == sel
                && let Some(src) = sources.get(idx)
                && let Some(sub_idx) = src.subsources.iter().position(|ss| &ss.key == sub_name)
            {
                selected_subsource = Some(sub_idx);
            }
        }
```

And use these in `Self { ... }` instead of the hardcoded `0` / `None`.

Implement new methods inside the `impl App`:

```rust
    pub fn spinner_glyph(&self) -> char {
        self.spinner.glyph()
    }

    pub fn tick_spinner(&mut self) {
        self.spinner.tick();
    }

    pub fn toggle_drilldown(&mut self) {
        self.drilldown_open = !self.drilldown_open;
        self.prefs.drilldown_open = self.drilldown_open;
    }
```

In `App::apply`, add a branch for `QuelchEvent::DocSynced` (currently ignored):

```rust
            QuelchEvent::DocSynced { source, subsource, id, updated } => {
                if let Some(sub) = self.find_subsource_mut(&source, &subsource) {
                    if sub.recent_docs.len() >= RECENT_DOCS_CAP {
                        sub.recent_docs.pop_front();
                    }
                    sub.recent_docs.push_back(RecentDoc { ts: updated, id });
                }
            }
```

In `App::apply`, add branches for the now-handled `BackoffStarted` / `BackoffFinished`:

```rust
            QuelchEvent::BackoffStarted { source: _, until, reason } => {
                self.backoff_reason = Some(reason);
                self.backoff_until = Some(until);
            }
            QuelchEvent::BackoffFinished { source: _ } => {
                self.backoff_reason = None;
                self.backoff_until = None;
            }
```

Remove `BackoffStarted`/`BackoffFinished` from the catch-all arm at the bottom that ignores them. And update the existing catch-all so it only ignores `AzureRequest | DocFailed`:

```rust
            QuelchEvent::AzureRequest { .. } | QuelchEvent::DocFailed { .. } => {}
```

Also — the existing `Self` initialization currently sets `focus` based on `prefs.focus`. Keep that. But update the save path: when prefs change, the app should mutate `prefs.selected_source` and `prefs.selected_subsource` as selection moves. Add a helper:

```rust
    fn sync_selection_to_prefs(&mut self) {
        self.prefs.selected_source = self
            .sources
            .get(self.selected_source)
            .map(|s| s.name.clone());
        self.prefs.selected_subsource = self.prefs.selected_source.as_ref().and_then(|src| {
            let src_idx = self.sources.iter().position(|s| &s.name == src)?;
            let sub_idx = self.selected_subsource?;
            let sub_name = self.sources[src_idx].subsources.get(sub_idx)?.key.clone();
            Some((src.clone(), sub_name))
        });
    }
```

Call `self.sync_selection_to_prefs()` at the end of each of `move_selection_down`, `move_selection_up`, `move_selection_left`, `move_selection_right`, and `toggle_selected_collapsed`.

- [ ] **Step 4: Run + verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/quelch/src/tui/app.rs
git commit -m "App: spinner, recent_docs ring, drilldown + backoff state

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: metrics::chart_points helper

Tiny addition — exposes the throughput ring buffer as `(x, y)` points for ratatui `Chart`.

**Files:**
- Modify: `crates/quelch/src/tui/metrics.rs`

- [ ] **Step 1: Failing test**

Append to existing `mod tests` in `crates/quelch/src/tui/metrics.rs`:

```rust
    #[test]
    fn chart_points_returns_ordered_xy_pairs() {
        let mut t = Throughput::default();
        let t0 = Instant::now();
        t.add(t0, 3);
        t.add(t0 + Duration::from_secs(2), 5);
        let pts = t.chart_points();
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[0].1, 3.0);
        assert_eq!(pts[1].1, 5.0);
        assert!(pts[0].0 < pts[1].0);
    }
```

- [ ] **Step 2: Fail**

Run: `cargo test -p quelch tui::metrics::tests::chart_points_returns_ordered_xy_pairs`
Expected: FAIL — method undefined.

- [ ] **Step 3: Implement**

In `Throughput` impl block, add:

```rust
    /// Return `(bucket_index, count)` points suitable for `ratatui::Chart`.
    /// The oldest bucket has the smallest x-value; the most recent has the largest.
    pub fn chart_points(&self) -> Vec<(f64, f64)> {
        self.buckets
            .iter()
            .enumerate()
            .map(|(i, (_, n))| (i as f64, *n as f64))
            .collect()
    }
```

- [ ] **Step 4: Verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/quelch/src/tui/metrics.rs
git commit -m "metrics: expose Throughput::chart_points for ratatui Chart

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: source_table widget (replaces source_card)

Replaces `source_card.rs` with a proper `Table`-based widget. Deletes the old file.

**Files:**
- Create: `crates/quelch/src/tui/widgets/source_table.rs`
- Delete: `crates/quelch/src/tui/widgets/source_card.rs`
- Modify: `crates/quelch/src/tui/widgets/mod.rs`
- Modify: `crates/quelch/src/tui/widgets/test.rs` (update the one test that references `SourceCard`)

- [ ] **Step 1: Failing test**

Replace `crates/quelch/src/tui/widgets/test.rs` content entirely with:

```rust
#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use crate::config::{
        AuthConfig, AzureConfig, Config, JiraSourceConfig, SourceConfig, SyncConfig,
    };
    use crate::tui::app::App;
    use crate::tui::prefs::Prefs;
    use crate::tui::widgets::source_table::SourceTable;

    fn cfg() -> Config {
        Config {
            azure: AzureConfig { endpoint: "x".into(), api_key: "k".into() },
            sources: vec![SourceConfig::Jira(JiraSourceConfig {
                name: "my-jira".into(),
                url: "x".into(),
                auth: AuthConfig::DataCenter { pat: "p".into() },
                projects: vec!["DO".into(), "HR".into()],
                index: "i".into(),
            })],
            sync: SyncConfig::default(),
        }
    }

    fn rendered_text(app: &App, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            f.render_widget(SourceTable { app }, f.area());
        })
        .unwrap();
        let buf = term.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_column_headings() {
        let app = App::new(&cfg(), Prefs::default());
        let text = rendered_text(&app, 100, 10);
        assert!(text.contains("Source"), "missing Source heading:\n{text}");
        assert!(text.contains("Status"), "missing Status heading");
        assert!(text.contains("Items"), "missing Items heading");
        assert!(text.contains("Rate"), "missing Rate heading");
        assert!(text.contains("Last item"), "missing Last item heading");
        assert!(text.contains("Updated"), "missing Updated heading");
    }

    #[test]
    fn renders_source_row_and_expanded_subsources() {
        let app = App::new(&cfg(), Prefs::default());
        let text = rendered_text(&app, 100, 10);
        assert!(text.contains("my-jira"));
        assert!(text.contains("DO"));
        assert!(text.contains("HR"));
    }

    #[test]
    fn collapsed_source_hides_subsources() {
        let mut app = App::new(&cfg(), Prefs::default());
        app.prefs.toggle_source_collapsed("my-jira");
        let text = rendered_text(&app, 100, 10);
        // With the source collapsed, subsource keys should NOT appear.
        // Count only in subsource-positioned rows, but simpler: assert DO/HR
        // absent AND my-jira present.
        assert!(text.contains("my-jira"));
        assert!(!text.contains("  DO"));
        assert!(!text.contains("  HR"));
    }
}
```

- [ ] **Step 2: Fail**

Run: `cargo test -p quelch tui::widgets::test`
Expected: compile error — `SourceTable` undefined.

- [ ] **Step 3: Create the widget**

Create `crates/quelch/src/tui/widgets/source_table.rs`:

```rust
//! Table-based Sources pane: columns + headings, tree-indented rows,
//! per-row state glyph, selected-row inverse-video highlight.

use chrono::Utc;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Text,
    widgets::{Cell, Row, Table, Widget},
};

use crate::tui::app::{App, SourceState, SourceView, SubsourceState, SubsourceView};

pub struct SourceTable<'a> {
    pub app: &'a App,
}

impl Widget for SourceTable<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut rows: Vec<Row> = vec![header_row()];
        rows.push(rule_row());

        let sel_src = self.app.selected_source;
        let sel_sub = self.app.selected_subsource;

        for (si, src) in self.app.sources.iter().enumerate() {
            let collapsed = self.app.prefs.is_source_collapsed(&src.name);
            let is_src_selected = si == sel_src && sel_sub.is_none();
            rows.push(source_row(src, collapsed, is_src_selected, self.app.spinner_glyph()));

            if !collapsed {
                for (ssi, sub) in src.subsources.iter().enumerate() {
                    let is_sub_selected = si == sel_src && sel_sub == Some(ssi);
                    rows.push(subsource_row(sub, is_sub_selected, self.app.spinner_glyph()));
                }
            }
        }

        let widths = [
            Constraint::Length(22),
            Constraint::Length(14),
            Constraint::Length(8),
            Constraint::Length(11),
            Constraint::Length(18),
            Constraint::Min(10),
        ];

        Table::new(rows, widths)
            .column_spacing(1)
            .render(area, buf);
    }
}

fn header_row() -> Row<'static> {
    Row::new(vec![
        Cell::from("Source"),
        Cell::from("Status"),
        Cell::from(Text::from("Items").alignment(ratatui::layout::Alignment::Right)),
        Cell::from("Rate"),
        Cell::from("Last item"),
        Cell::from("Updated"),
    ])
    .style(Style::default().fg(Color::DarkGray))
}

fn rule_row() -> Row<'static> {
    Row::new(vec![
        Cell::from("──────────────────"),
        Cell::from("────────────"),
        Cell::from("──────"),
        Cell::from("─────────"),
        Cell::from("────────────────"),
        Cell::from("────────"),
    ])
    .style(Style::default().fg(Color::DarkGray))
}

fn source_row(src: &SourceView, collapsed: bool, selected: bool, spin: char) -> Row<'static> {
    let name_col = format!(
        "{arrow} {name}",
        arrow = if collapsed { "▸" } else { "▾" },
        name = src.name,
    );
    let total_docs: u64 = src.subsources.iter().map(|s| s.docs_synced_total).sum();
    let row = Row::new(vec![
        Cell::from(name_col).style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from(format_state_src(&src.state, spin)),
        Cell::from(Text::from(total_docs.to_string()).alignment(ratatui::layout::Alignment::Right)),
        Cell::from("—"),
        Cell::from("—"),
        Cell::from("—"),
    ]);
    if selected {
        row.style(Style::default().add_modifier(Modifier::REVERSED))
    } else {
        row
    }
}

fn subsource_row(sub: &SubsourceView, selected: bool, spin: char) -> Row<'static> {
    let name_col = format!("    {name}", name = sub.key);
    let items = sub.docs_synced_total.to_string();
    let rate_label = if sub.docs_synced_total == 0 && !matches!(sub.state, SubsourceState::Syncing)
    {
        "—".to_string()
    } else {
        let per_min: u64 = sub.throughput.samples().iter().sum();
        format!("{:.1}/min", per_min as f32)
    };
    let last_item = sub.last_sample_id.as_deref().unwrap_or("—").to_string();
    let updated = sub
        .last_cursor
        .map(format_relative)
        .unwrap_or_else(|| "—".into());

    let row = Row::new(vec![
        Cell::from(name_col),
        Cell::from(format_state_sub(&sub.state, spin)),
        Cell::from(Text::from(items).alignment(ratatui::layout::Alignment::Right)),
        Cell::from(rate_label),
        Cell::from(last_item),
        Cell::from(updated),
    ]);
    if selected {
        row.style(Style::default().add_modifier(Modifier::REVERSED))
    } else {
        row
    }
}

fn format_state_src(state: &SourceState, spin: char) -> Text<'static> {
    match state {
        SourceState::Idle => Text::from("● idle").style(Style::default().fg(Color::Green)),
        SourceState::Syncing => {
            Text::from(format!("{spin} syncing")).style(Style::default().fg(Color::Cyan))
        }
        SourceState::Error(_) => Text::from("● error").style(Style::default().fg(Color::Red)),
        SourceState::Backoff { .. } => {
            Text::from("◉ backoff").style(Style::default().fg(Color::Yellow))
        }
    }
}

fn format_state_sub(state: &SubsourceState, spin: char) -> Text<'static> {
    match state {
        SubsourceState::Idle => Text::from("● idle").style(Style::default().fg(Color::Green)),
        SubsourceState::Syncing => {
            Text::from(format!("{spin} syncing")).style(Style::default().fg(Color::Cyan))
        }
        SubsourceState::Error(_) => Text::from("● error").style(Style::default().fg(Color::Red)),
    }
}

fn format_relative(ts: chrono::DateTime<Utc>) -> String {
    let diff = Utc::now().signed_duration_since(ts);
    let secs = diff.num_seconds();
    if secs < 5 {
        "now".into()
    } else if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m {}s ago", secs / 60, secs % 60)
    } else {
        format!("{}h ago", secs / 3600)
    }
}
```

Update `crates/quelch/src/tui/widgets/mod.rs`:

```rust
pub mod azure_panel;
pub mod log_view;
pub mod source_table;

#[cfg(test)]
mod test;
```

Delete `crates/quelch/src/tui/widgets/source_card.rs`.

- [ ] **Step 4: Update layout.rs callsite**

`layout.rs` imports `SourceCard`; switch to use the new `SourceTable`. Replace the entire `draw_sources` function in `crates/quelch/src/tui/layout.rs` with:

```rust
fn draw_sources(f: &mut Frame, area: Rect, app: &App) {
    use crate::tui::widgets::source_table::SourceTable;

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(if matches!(app.focus, Focus::Sources) {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        })
        .title("Sources");
    let inner = outer.inner(area);
    outer.render(area, f.buffer_mut());

    if app.sources.is_empty() {
        f.render_widget(Paragraph::new("No sources configured"), inner);
        return;
    }

    f.render_widget(SourceTable { app }, inner);
}
```

And remove the `SourceCard` import from the top of `layout.rs`:

```rust
use super::widgets::{azure_panel::AzurePanelWidget, log_view::LogView};
```

- [ ] **Step 5: Verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add -A crates/quelch/src/tui/widgets/ crates/quelch/src/tui/layout.rs
git commit -m "Replace source_card with source_table (columns + headings)

Addresses v0.4.0 feedback: proper Table widget with headings (Source,
Status, Items, Rate, Last item, Updated), tree-indented rows, coloured
state glyphs + spinner, selected-row inverse-video highlight.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: azure_panel rewrite with Chart widget

**Files:**
- Rewrite: `crates/quelch/src/tui/widgets/azure_panel.rs`

- [ ] **Step 1: Failing test**

Append to `crates/quelch/src/tui/widgets/test.rs`'s `mod tests`:

```rust
    use crate::tui::widgets::azure_panel::AzurePanelWidget;

    #[test]
    fn azure_panel_shows_plain_english_labels() {
        let app = App::new(&cfg(), Prefs::default());
        let mut term = Terminal::new(TestBackend::new(100, 12)).unwrap();
        term.draw(|f| {
            f.render_widget(
                AzurePanelWidget {
                    panel: &app.azure,
                    drops: 0,
                    focused: false,
                    backoff_reason: None,
                },
                f.area(),
            );
        })
        .unwrap();
        let buf = term.backend().buffer();
        let text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| buf[(x, y)].symbol().to_string()))
            .collect::<Vec<_>>()
            .join("");
        assert!(text.contains("Total requests"), "rendered: {text}");
        assert!(text.contains("median"), "rendered: {text}");
        assert!(text.contains("Failed (4xx)"));
        assert!(text.contains("Failed (5xx)"));
        assert!(text.contains("Throttled"));
    }
```

- [ ] **Step 2: Fail**

Run: `cargo test -p quelch tui::widgets::test::azure_panel_shows_plain_english_labels`
Expected: compile error — `backoff_reason` field doesn't exist on widget.

- [ ] **Step 3: Rewrite widget**

Replace the entire content of `crates/quelch/src/tui/widgets/azure_panel.rs` with:

```rust
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph, Widget},
};

use crate::tui::metrics::AzurePanel;

pub struct AzurePanelWidget<'a> {
    pub panel: &'a AzurePanel,
    pub drops: u64,
    pub focused: bool,
    pub backoff_reason: Option<&'a str>,
}

impl Widget for AzurePanelWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title("Azure AI Search");
        let inner = block.inner(area);
        block.render(area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),           // backoff banner OR subtitle
                Constraint::Min(5),              // chart
                Constraint::Length(3),           // counter strip (3 rows)
            ])
            .split(inner);

        // --- Row 1: backoff banner OR chart subtitle with max ---
        let subtitle_max = self
            .panel
            .requests_per_sec
            .samples()
            .iter()
            .copied()
            .max()
            .unwrap_or(0);
        let subtitle = if let Some(reason) = self.backoff_reason {
            Paragraph::new(
                Line::from(vec![
                    Span::styled("◉ Azure client backing off", Style::default().fg(Color::Yellow)),
                    Span::raw("  "),
                    Span::styled(reason.to_string(), Style::default().fg(Color::Yellow)),
                ]),
            )
        } else {
            Paragraph::new(Line::from(vec![
                Span::styled(
                    "Requests per second (last 60s)",
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw("    "),
                Span::styled(
                    format!("max {subtitle_max} req/s"),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        };
        subtitle.render(chunks[0], buf);

        // --- Row 2: chart ---
        let points: Vec<(f64, f64)> = self.panel.requests_per_sec.chart_points();
        let y_max = (subtitle_max as f64).max(1.0);
        let dataset = Dataset::default()
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Cyan))
            .data(&points);
        Chart::new(vec![dataset])
            .x_axis(
                Axis::default()
                    .bounds([0.0, 60.0])
                    .labels(vec!["-60s".into(), "now".into()])
                    .style(Style::default().fg(Color::DarkGray)),
            )
            .y_axis(
                Axis::default()
                    .bounds([0.0, y_max])
                    .labels(vec!["0".into(), format!("{}", y_max as u64).into()])
                    .style(Style::default().fg(Color::DarkGray)),
            )
            .render(chunks[1], buf);

        // --- Row 3: counter strip ---
        let (p50, p95) = self.panel.p50_p95();
        let color_of = |n: u64, ok: Color, bad: Color| {
            if n == 0 { Color::DarkGray } else { bad }
        };
        let _ = ok_color_placeholder();
        let rows = vec![
            Line::from(vec![
                Span::styled("Total requests  ", Style::default().fg(Color::DarkGray)),
                Span::raw(format!("{:<8}", self.panel.total)),
                Span::styled("Latency      ", Style::default().fg(Color::DarkGray)),
                Span::raw(format!(
                    "median {} ms · 95th {} ms",
                    p50.as_millis(),
                    p95.as_millis()
                )),
            ]),
            Line::from(vec![
                Span::styled("Failed (4xx)    ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{:<8}", self.panel.count_4xx),
                    Style::default().fg(color_of(self.panel.count_4xx, Color::DarkGray, Color::Red)),
                ),
                Span::styled("Throttled (429)  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}", self.panel.count_throttled),
                    Style::default().fg(color_of(self.panel.count_throttled, Color::DarkGray, Color::Yellow)),
                ),
            ]),
            Line::from(vec![
                Span::styled("Failed (5xx)    ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{:<8}", self.panel.count_5xx),
                    Style::default().fg(color_of(self.panel.count_5xx, Color::DarkGray, Color::Red)),
                ),
                Span::styled("Dropped events   ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}", self.drops),
                    Style::default().fg(color_of(self.drops, Color::DarkGray, Color::Yellow)),
                ),
            ]),
        ];
        Paragraph::new(rows).render(chunks[2], buf);

        // Inverse-video caret on focus
        if self.focused {
            let _bold = Modifier::REVERSED; // style applied via border already
        }
    }
}

fn ok_color_placeholder() -> Color {
    Color::DarkGray
}
```

- [ ] **Step 4: Update layout.rs callsite**

Change the `AzurePanelWidget` construction in `crates/quelch/src/tui/layout.rs`:

```rust
    f.render_widget(
        AzurePanelWidget {
            panel: &app.azure,
            drops: app.drops,
            focused: matches!(app.focus, Focus::Azure),
            backoff_reason: app.backoff_reason.as_deref(),
        },
        areas[2],
    );
```

- [ ] **Step 5: Verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/quelch/src/tui/widgets/azure_panel.rs crates/quelch/src/tui/widgets/test.rs crates/quelch/src/tui/layout.rs
git commit -m "azure_panel: real Chart widget + plain-English counters + backoff banner

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: drilldown widget

**Files:**
- Create: `crates/quelch/src/tui/widgets/drilldown.rs`
- Modify: `crates/quelch/src/tui/widgets/mod.rs`
- Extend: `crates/quelch/src/tui/widgets/test.rs`

- [ ] **Step 1: Failing test**

Append to `crates/quelch/src/tui/widgets/test.rs`'s `mod tests`:

```rust
    use crate::tui::widgets::drilldown::Drilldown;

    #[test]
    fn drilldown_shows_subsource_details() {
        let mut app = App::new(&cfg(), Prefs::default());
        // Populate a subsource with some data
        app.apply(crate::tui::events::QuelchEvent::SubsourceBatch {
            source: "my-jira".into(),
            subsource: "DO".into(),
            fetched: 5,
            cursor: chrono::Utc::now(),
            sample_id: "DO-42".into(),
        });
        for i in 0..3 {
            app.apply(crate::tui::events::QuelchEvent::DocSynced {
                source: "my-jira".into(),
                subsource: "DO".into(),
                id: format!("DO-{i}"),
                updated: chrono::Utc::now(),
            });
        }
        app.move_selection_down(); // focus DO

        let mut term = Terminal::new(TestBackend::new(60, 20)).unwrap();
        term.draw(|f| {
            f.render_widget(Drilldown { app: &app }, f.area());
        })
        .unwrap();
        let buf = term.backend().buffer();
        let text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| buf[(x, y)].symbol().to_string()))
            .collect::<Vec<_>>()
            .join("");
        assert!(text.contains("Docs synced"));
        assert!(text.contains("Recent"));
        assert!(text.contains("DO-2"));
    }
```

- [ ] **Step 2: Fail**

Run: `cargo test -p quelch tui::widgets::test::drilldown_shows_subsource_details`
Expected: compile error — `Drilldown` undefined.

- [ ] **Step 3: Create widget**

Create `crates/quelch/src/tui/widgets/drilldown.rs`:

```rust
//! Drilldown pane: per-subsource detail view triggered by Enter.

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use crate::tui::app::{App, SubsourceState, SubsourceView};

pub struct Drilldown<'a> {
    pub app: &'a App,
}

impl Widget for Drilldown<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let sub = match focused_subsource(self.app) {
            Some(s) => s,
            None => {
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title("Drilldown")
                    .border_style(Style::default().fg(Color::DarkGray));
                block.render(area, buf);
                return;
            }
        };

        let src_name = self
            .app
            .sources
            .get(self.app.selected_source)
            .map(|s| s.name.clone())
            .unwrap_or_default();

        let title = format!("{key} ({src_name})", key = sub.key);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(area);
        block.render(area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(7),   // summary lines
                Constraint::Min(4),      // recent docs
                Constraint::Length(5),   // recent errors
            ])
            .split(inner);

        // Summary
        let status = match &sub.state {
            SubsourceState::Idle => ("● idle", Color::Green),
            SubsourceState::Syncing => ("◐ syncing", Color::Cyan),
            SubsourceState::Error(_) => ("● error", Color::Red),
        };
        let cursor = sub
            .last_cursor
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| "—".into());
        let last = sub.last_sample_id.as_deref().unwrap_or("—");
        let rate: u64 = sub.throughput.samples().iter().sum();
        let summary = vec![
            label_line("Status         ", status.0, status.1),
            plain_line("Docs synced    ", &sub.docs_synced_total.to_string()),
            plain_line("Rate (60s)     ", &format!("{rate} per minute")),
            plain_line("Cursor         ", &cursor),
            plain_line("Last item      ", last),
            Line::from(""),
            plain_line("Recent (up to 10)", ""),
        ];
        Paragraph::new(summary).render(chunks[0], buf);

        // Recent docs
        let mut recent_lines: Vec<Line> = Vec::new();
        for doc in sub.recent_docs.iter().rev() {
            let time = doc.ts.format("%H:%M:%S").to_string();
            recent_lines.push(Line::from(vec![
                Span::styled("  ● ", Style::default().fg(Color::Green)),
                Span::raw(time),
                Span::raw("  "),
                Span::raw(doc.id.clone()),
            ]));
        }
        if recent_lines.is_empty() {
            recent_lines.push(Line::from(Span::styled(
                "  (none yet)",
                Style::default().fg(Color::DarkGray),
            )));
        }
        Paragraph::new(recent_lines).render(chunks[1], buf);

        // Recent errors
        let mut err_lines: Vec<Line> =
            vec![Line::from(Span::styled("Recent errors (last 3)", Style::default().fg(Color::DarkGray)))];
        if sub.last_errors.is_empty() {
            err_lines.push(Line::from(Span::styled(
                "  (none)",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for e in sub.last_errors.iter().rev() {
                err_lines.push(Line::from(vec![
                    Span::styled("  × ", Style::default().fg(Color::Red)),
                    Span::raw(e.clone()),
                ]));
            }
        }
        Paragraph::new(err_lines).render(chunks[2], buf);
    }
}

fn focused_subsource<'a>(app: &'a App) -> Option<&'a SubsourceView> {
    let src = app.sources.get(app.selected_source)?;
    let idx = app.selected_subsource?;
    src.subsources.get(idx)
}

fn label_line(label: &'static str, value: &'static str, colour: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(label, Style::default().fg(Color::DarkGray)),
        Span::styled(value, Style::default().fg(colour)),
    ])
}

fn plain_line(label: &'static str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(label, Style::default().fg(Color::DarkGray)),
        Span::raw(value.to_string()),
    ])
}
```

Register in `crates/quelch/src/tui/widgets/mod.rs`:

```rust
pub mod azure_panel;
pub mod drilldown;
pub mod log_view;
pub mod source_table;

#[cfg(test)]
mod test;
```

- [ ] **Step 4: Verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/quelch/src/tui/widgets/drilldown.rs crates/quelch/src/tui/widgets/mod.rs crates/quelch/src/tui/widgets/test.rs
git commit -m "Add drilldown widget for per-subsource detail view

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: help_overlay widget

**Files:**
- Create: `crates/quelch/src/tui/widgets/help_overlay.rs`
- Modify: `crates/quelch/src/tui/widgets/mod.rs`

- [ ] **Step 1: Failing test**

Append to `crates/quelch/src/tui/widgets/test.rs`:

```rust
    use crate::tui::widgets::help_overlay::HelpOverlay;

    #[test]
    fn help_overlay_lists_key_bindings() {
        let mut term = Terminal::new(TestBackend::new(70, 30)).unwrap();
        term.draw(|f| {
            f.render_widget(HelpOverlay {}, f.area());
        })
        .unwrap();
        let buf = term.backend().buffer();
        let text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| buf[(x, y)].symbol().to_string()))
            .collect::<Vec<_>>()
            .join("");
        assert!(text.contains("Keyboard shortcuts"));
        assert!(text.contains("sync now"));
        assert!(text.contains("pause"));
        assert!(text.contains("quit"));
    }
```

- [ ] **Step 2: Fail**

Run: `cargo test -p quelch tui::widgets::test::help_overlay_lists_key_bindings`
Expected: compile error.

- [ ] **Step 3: Create widget**

Create `crates/quelch/src/tui/widgets/help_overlay.rs`:

```rust
//! Help overlay — modal list of key bindings.

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

pub struct HelpOverlay;

impl Widget for HelpOverlay {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let modal_w = 52u16.min(area.width.saturating_sub(4));
        let modal_h = 22u16.min(area.height.saturating_sub(2));
        let h_pad = (area.width.saturating_sub(modal_w)) / 2;
        let v_pad = (area.height.saturating_sub(modal_h)) / 2;
        let outer = Rect {
            x: area.x + h_pad,
            y: area.y + v_pad,
            width: modal_w,
            height: modal_h,
        };

        Clear.render(outer, buf);
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Keyboard shortcuts")
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(outer);
        block.render(outer, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        let body = vec![
            heading("Navigation"),
            kv("↑ ↓", "move up / down"),
            kv("← →", "collapse / expand"),
            kv("Tab", "cycle focus panes"),
            kv("Enter", "open drilldown"),
            Line::from(""),
            heading("Actions"),
            kv("r", "sync now"),
            kv("p", "pause / resume"),
            kv("R", "reset cursor (press twice)"),
            kv("P", "purge source (press twice)"),
            Line::from(""),
            heading("View"),
            kv("s", "toggle log view"),
            kv("c", "clear footer flash"),
            Line::from(""),
            heading("Other"),
            kv("?", "this help"),
            kv("q or ^C", "quit"),
        ];
        Paragraph::new(body).render(chunks[0], buf);
        Paragraph::new(Line::from(Span::styled(
            "press ? or Esc to dismiss",
            Style::default().fg(Color::DarkGray),
        )))
        .render(chunks[1], buf);
    }
}

fn heading(s: &'static str) -> Line<'static> {
    Line::from(Span::styled(s, Style::default().fg(Color::Yellow)))
}

fn kv(k: &'static str, v: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {k:<10}"), Style::default().fg(Color::Cyan)),
        Span::raw(v),
    ])
}
```

Register in `widgets/mod.rs`:

```rust
pub mod azure_panel;
pub mod drilldown;
pub mod help_overlay;
pub mod log_view;
pub mod source_table;

#[cfg(test)]
mod test;
```

- [ ] **Step 4: Verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/quelch/src/tui/widgets/help_overlay.rs crates/quelch/src/tui/widgets/mod.rs crates/quelch/src/tui/widgets/test.rs
git commit -m "Add help overlay widget with grouped key bindings

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 9: log_view column layout

**Files:**
- Rewrite: `crates/quelch/src/tui/widgets/log_view.rs`

- [ ] **Step 1: Failing test**

Append to `widgets/test.rs`:

```rust
    use crate::tui::app::LogLine;
    use crate::tui::widgets::log_view::LogView;
    use std::collections::VecDeque;

    #[test]
    fn log_view_renders_column_headings() {
        let mut lines = VecDeque::new();
        lines.push_back(LogLine {
            ts: chrono::Utc::now(),
            level: tracing::Level::INFO,
            target: "quelch::sync".into(),
            message: "Cycle starting".into(),
        });
        let view = LogView { lines: &lines, focused: false };
        let mut term = Terminal::new(TestBackend::new(100, 10)).unwrap();
        term.draw(|f| f.render_widget(view, f.area())).unwrap();
        let buf = term.backend().buffer();
        let text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| buf[(x, y)].symbol().to_string()))
            .collect::<Vec<_>>()
            .join("");
        assert!(text.contains("LEVEL"));
        assert!(text.contains("TIME"));
        assert!(text.contains("TARGET"));
        assert!(text.contains("MESSAGE"));
        assert!(text.contains("Cycle starting"));
    }
```

- [ ] **Step 2: Fail**

Run: `cargo test -p quelch tui::widgets::test::log_view_renders_column_headings`
Expected: Fail — current LogView doesn't show headings.

- [ ] **Step 3: Rewrite**

Replace `crates/quelch/src/tui/widgets/log_view.rs` entirely:

```rust
use std::collections::VecDeque;

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Row, Table, Widget},
};

use crate::tui::app::LogLine;

pub struct LogView<'a> {
    pub lines: &'a VecDeque<LogLine>,
    pub focused: bool,
}

impl Widget for LogView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border = if self.focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .title("Log (tail)");
        let inner = block.inner(area);
        block.render(area, buf);

        let rows_visible = inner.height.saturating_sub(2) as usize;
        let start = self.lines.len().saturating_sub(rows_visible);

        let header = Row::new(vec![
            Cell::from("LEVEL"),
            Cell::from("TIME"),
            Cell::from("TARGET"),
            Cell::from("MESSAGE"),
        ])
        .style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD));

        let rule = Row::new(vec![
            Cell::from("─────"),
            Cell::from("────────"),
            Cell::from("────────────────────────"),
            Cell::from("────────────────────────"),
        ])
        .style(Style::default().fg(Color::DarkGray));

        let mut rows = vec![header, rule];
        for line in self.lines.iter().skip(start) {
            let lvl = format!("{:>5}", format!("{}", line.level));
            let time = line.ts.format("%H:%M:%S").to_string();
            let target = line.target.clone();
            rows.push(Row::new(vec![
                Cell::from(Span::styled(lvl, Style::default().fg(level_colour(&line.level)))),
                Cell::from(time),
                Cell::from(target),
                Cell::from(line.message.clone()),
            ]));
        }

        Table::new(
            rows,
            [
                Constraint::Length(5),
                Constraint::Length(8),
                Constraint::Length(24),
                Constraint::Min(20),
            ],
        )
        .column_spacing(1)
        .render(inner, buf);
    }
}

fn level_colour(l: &tracing::Level) -> Color {
    match *l {
        tracing::Level::ERROR => Color::Red,
        tracing::Level::WARN => Color::Yellow,
        tracing::Level::INFO => Color::Green,
        tracing::Level::DEBUG => Color::Cyan,
        tracing::Level::TRACE => Color::Gray,
    }
}
```

Update `layout.rs` — change the LogView call to pass `&app.log_tail`:

```rust
    if app.prefs.log_view_on {
        f.render_widget(
            LogView {
                lines: &app.log_tail,
                focused: matches!(app.focus, Focus::Sources),
            },
            areas[1],
        );
    }
```

(Adjust: `app.log_tail` is `VecDeque<LogLine>` which works with `&VecDeque<LogLine>` typed argument.)

- [ ] **Step 4: Verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/quelch/src/tui/widgets/log_view.rs crates/quelch/src/tui/widgets/test.rs crates/quelch/src/tui/layout.rs
git commit -m "log_view: column layout with LEVEL TIME TARGET MESSAGE headings

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 10: status module + layout rewrite + footer dedup

**Files:**
- Create: `crates/quelch/src/tui/status.rs`
- Rewrite: `crates/quelch/src/tui/layout.rs`
- Modify: `crates/quelch/src/tui/mod.rs` (register `status`)

- [ ] **Step 1: Failing test**

Create `crates/quelch/src/tui/status.rs` with tests:

```rust
//! Centralised header-string builder. All TUI states map here.

use chrono::{DateTime, Utc};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::app::{App, EngineStatus};

/// Render the header as one `Line`, with colour coded by state.
pub fn header_line(app: &App, now: DateTime<Utc>, uptime: std::time::Duration) -> Line<'static> {
    let version = env!("CARGO_PKG_VERSION");
    let up = format_uptime(uptime);
    let state = state_span(app, now);
    Line::from(vec![
        Span::styled(format!(" quelch {version} "), Style::default().fg(Color::DarkGray)),
        Span::raw(" · "),
        state,
        Span::raw("   "),
        Span::styled(format!("uptime {up}"), Style::default().fg(Color::DarkGray)),
    ])
}

fn state_span(app: &App, _now: DateTime<Utc>) -> Span<'static> {
    if app.backoff_reason.is_some() {
        let remaining = app
            .backoff_until
            .map(|u| u.signed_duration_since(Utc::now()).num_seconds().max(0))
            .unwrap_or(0);
        return Span::styled(
            format!("◉ Azure client backing off · {remaining}s remaining"),
            Style::default().fg(Color::Yellow),
        );
    }
    match &app.status {
        EngineStatus::Idle => Span::styled(
            "○ Ready · press r to sync now".to_string(),
            Style::default().fg(Color::DarkGray),
        ),
        EngineStatus::Syncing { cycle, .. } => Span::styled(
            format!("{spin} Syncing · cycle {cycle}", spin = app.spinner_glyph()),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        EngineStatus::Paused => Span::styled(
            "⏸ Paused · press p to resume".to_string(),
            Style::default().fg(Color::Yellow),
        ),
        EngineStatus::Shutdown => Span::styled(
            "⏹ Shutting down".to_string(),
            Style::default().fg(Color::DarkGray),
        ),
    }
}

fn format_uptime(d: std::time::Duration) -> String {
    let s = d.as_secs();
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let ss = s % 60;
    format!("{h}:{m:02}:{ss:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        AuthConfig, AzureConfig, Config, JiraSourceConfig, SourceConfig, SyncConfig,
    };
    use crate::tui::app::App;
    use crate::tui::prefs::Prefs;

    fn app() -> App {
        let cfg = Config {
            azure: AzureConfig { endpoint: "x".into(), api_key: "k".into() },
            sources: vec![SourceConfig::Jira(JiraSourceConfig {
                name: "j".into(),
                url: "x".into(),
                auth: AuthConfig::DataCenter { pat: "p".into() },
                projects: vec!["DO".into()],
                index: "i".into(),
            })],
            sync: SyncConfig::default(),
        };
        App::new(&cfg, Prefs::default())
    }

    #[test]
    fn idle_header_mentions_ready() {
        let a = app();
        let line = header_line(&a, Utc::now(), std::time::Duration::from_secs(5));
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("Ready"));
        assert!(text.contains("uptime 0:00:05"));
    }

    #[test]
    fn paused_header_shows_pause_glyph() {
        let mut a = app();
        a.status = EngineStatus::Paused;
        let line = header_line(&a, Utc::now(), std::time::Duration::from_secs(0));
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("Paused"));
    }

    #[test]
    fn backoff_header_takes_precedence() {
        let mut a = app();
        a.backoff_reason = Some("HTTP 429".into());
        a.backoff_until = Some(Utc::now() + chrono::Duration::seconds(30));
        let line = header_line(&a, Utc::now(), std::time::Duration::from_secs(0));
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("backing off"));
    }
}
```

Register in `tui/mod.rs`:

```rust
//! Terminal user interface for `quelch watch` / `quelch sync`.

pub mod app;
pub mod events;
pub mod input;
pub mod layout;
pub mod metrics;
pub mod prefs;
pub mod spinner;
pub mod status;
pub mod tracing_layer;
pub mod widgets;
```

- [ ] **Step 2: Fail**

Run: `cargo test -p quelch tui::status`
Expected: fail — module not registered yet.

- [ ] **Step 3: Rewrite layout.rs**

Replace `crates/quelch/src/tui/layout.rs` entirely with:

```rust
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use super::app::{App, Focus};
use super::status::header_line;
use super::widgets::{
    azure_panel::AzurePanelWidget, drilldown::Drilldown, help_overlay::HelpOverlay,
    log_view::LogView, source_table::SourceTable,
};

/// Current uptime — the app owns a `start: Instant` field (Task 12 adds one).
pub fn draw(f: &mut Frame, app: &App, uptime: std::time::Duration, help_open: bool) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),   // header
            Constraint::Min(12),     // sources or log
            Constraint::Length(8),   // azure
            Constraint::Length(1),   // footer
        ])
        .split(f.area());

    f.render_widget(Clear, f.area());
    draw_header(f, areas[0], app, uptime);

    if app.prefs.log_view_on {
        f.render_widget(
            LogView { lines: &app.log_tail, focused: matches!(app.focus, Focus::Sources) },
            areas[1],
        );
    } else {
        draw_sources_area(f, areas[1], app);
    }

    f.render_widget(
        AzurePanelWidget {
            panel: &app.azure,
            drops: app.drops,
            focused: matches!(app.focus, Focus::Azure),
            backoff_reason: app.backoff_reason.as_deref(),
        },
        areas[2],
    );
    draw_footer(f, areas[3], app);

    // Help overlay renders on top of everything else.
    if help_open {
        f.render_widget(HelpOverlay, f.area());
    }
}

fn draw_header(f: &mut Frame, area: Rect, app: &App, uptime: std::time::Duration) {
    f.render_widget(
        Paragraph::new(header_line(app, chrono::Utc::now(), uptime)),
        area,
    );
}

fn draw_sources_area(f: &mut Frame, area: Rect, app: &App) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(if matches!(app.focus, Focus::Sources) {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        })
        .title("Sources");
    let inner = outer.inner(area);
    outer.render(area, f.buffer_mut());

    if app.sources.is_empty() {
        f.render_widget(Paragraph::new("No sources configured"), inner);
        return;
    }

    if app.drilldown_open && app.selected_subsource.is_some() {
        let split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(inner);
        f.render_widget(SourceTable { app }, split[0]);
        f.render_widget(Drilldown { app }, split[1]);
    } else {
        f.render_widget(SourceTable { app }, inner);
    }
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let msg = if app.footer.is_empty() {
        " ↑↓ select  ·  ←/→ collapse  ·  enter details  ·  r sync now  ·  p pause  ·  s logs  ·  ? help  ·  q quit".to_string()
    } else {
        format!(" {}", app.footer)
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(msg, Style::default().fg(Color::Gray)))),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        AuthConfig, AzureConfig, Config, JiraSourceConfig, SourceConfig, SyncConfig,
    };
    use crate::tui::app::App;
    use crate::tui::prefs::Prefs;
    use ratatui::backend::TestBackend;

    fn cfg() -> Config {
        Config {
            azure: AzureConfig { endpoint: "x".into(), api_key: "k".into() },
            sources: vec![SourceConfig::Jira(JiraSourceConfig {
                name: "j".into(),
                url: "x".into(),
                auth: AuthConfig::DataCenter { pat: "p".into() },
                projects: vec!["DO".into(), "HR".into()],
                index: "i".into(),
            })],
            sync: SyncConfig::default(),
        }
    }

    #[test]
    fn layout_renders_without_panicking() {
        let app = App::new(&cfg(), Prefs::default());
        let mut term = ratatui::Terminal::new(TestBackend::new(100, 26)).unwrap();
        term.draw(|f| {
            draw(f, &app, std::time::Duration::from_secs(1), false);
        })
        .unwrap();
    }

    #[test]
    fn footer_shows_only_one_keybinding_line() {
        let app = App::new(&cfg(), Prefs::default());
        let mut term = ratatui::Terminal::new(TestBackend::new(100, 26)).unwrap();
        term.draw(|f| {
            draw(f, &app, std::time::Duration::from_secs(1), false);
        })
        .unwrap();
        let buf = term.backend().buffer();
        let text: String = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        // The old buggy footer rendered two identical lines with key lists.
        // New footer should have the key prompt only once.
        let occurrences = text.matches("sync now").count();
        assert_eq!(occurrences, 1, "expected 1 footer line, found {occurrences}: {text}");
    }
}
```

- [ ] **Step 4: Verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/quelch/src/tui/status.rs crates/quelch/src/tui/layout.rs crates/quelch/src/tui/mod.rs
git commit -m "Add status module; layout rewrite with dedup footer + drilldown split

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 11: input — Enter opens drilldown, Esc closes, `?` toggles help

**Files:**
- Modify: `crates/quelch/src/tui/input.rs`

- [ ] **Step 1: Failing tests**

Append to `input.rs`'s `mod tests`:

```rust
    #[test]
    fn enter_on_focused_subsource_opens_drilldown() {
        let mut state = InputState::default();
        let mut app = make_app();
        // Navigate to focus a subsource
        state.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &mut app);
        assert_eq!(app.focused_subsource_name(), Some("DO"));
        // Enter opens drilldown
        state.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut app);
        assert!(app.drilldown_open);
        // Esc closes it
        state.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &mut app);
        assert!(!app.drilldown_open);
    }

    #[test]
    fn question_mark_toggles_help_overlay() {
        let mut state = InputState::default();
        let mut app = make_app();
        assert!(!state.help_open());
        state.on_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE), &mut app);
        assert!(state.help_open());
        state.on_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE), &mut app);
        assert!(!state.help_open());
    }

    #[test]
    fn esc_closes_help_overlay() {
        let mut state = InputState::default();
        let mut app = make_app();
        state.on_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE), &mut app);
        assert!(state.help_open());
        state.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &mut app);
        assert!(!state.help_open());
    }
```

- [ ] **Step 2: Fail**

Run: `cargo test -p quelch tui::input`
Expected: FAIL (no `help_open`; Enter currently toggles collapse).

- [ ] **Step 3: Implement**

Update `crates/quelch/src/tui/input.rs`:

Replace the `InputState` struct with:

```rust
#[derive(Default)]
pub struct InputState {
    pub pending_confirm: Option<(char, Instant)>,
    help_open: bool,
}

impl InputState {
    pub fn help_open(&self) -> bool {
        self.help_open
    }
}
```

In `on_key`, replace the existing `KeyCode::Char(' ') | KeyCode::Enter` arm with two distinct arms:

```rust
            KeyCode::Char(' ') => {
                if matches!(app.focus, Focus::Sources) {
                    app.toggle_selected_collapsed();
                }
            }
            KeyCode::Enter => {
                if matches!(app.focus, Focus::Sources)
                    && app.focused_subsource_name().is_some()
                {
                    app.toggle_drilldown();
                } else if matches!(app.focus, Focus::Sources) {
                    app.toggle_selected_collapsed();
                }
            }
            KeyCode::Esc => {
                if self.help_open {
                    self.help_open = false;
                } else if app.drilldown_open {
                    app.toggle_drilldown();
                }
            }
            KeyCode::Char('?') => {
                self.help_open = !self.help_open;
            }
```

- [ ] **Step 4: Verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/quelch/src/tui/input.rs
git commit -m "input: Enter opens drilldown, Esc closes overlays, ? toggles help

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 12: tui::run — EventStream async input + uptime tracking

**Files:**
- Modify: `crates/quelch/src/tui/mod.rs`

- [ ] **Step 1: Failing test**

Since this task is mostly wiring for an interactive component, it doesn't have a unit test that's worth more than the smoke test we already have. Instead, add a smoke test that verifies the helper for uptime-based header rendering works when called from `run` via a mocked backend.

Append to `crates/quelch/src/tui/mod.rs`'s existing `mod smoke_tests`:

```rust
    #[test]
    fn draw_accepts_uptime_and_help_open_flag() {
        use crate::config::{
            AuthConfig, AzureConfig, Config, JiraSourceConfig, SourceConfig, SyncConfig,
        };
        use crate::tui::app::App;
        use crate::tui::layout::draw;
        use crate::tui::prefs::Prefs;

        let cfg = Config {
            azure: AzureConfig { endpoint: "x".into(), api_key: "k".into() },
            sources: vec![SourceConfig::Jira(JiraSourceConfig {
                name: "j".into(),
                url: "x".into(),
                auth: AuthConfig::DataCenter { pat: "p".into() },
                projects: vec!["DO".into()],
                index: "i".into(),
            })],
            sync: SyncConfig::default(),
        };
        let app = App::new(&cfg, Prefs::default());
        let mut term = ratatui::Terminal::new(TestBackend::new(100, 26)).unwrap();
        term.draw(|f| draw(f, &app, std::time::Duration::from_secs(7), true))
            .unwrap();
    }
```

- [ ] **Step 2: Fail**

Run: `cargo test -p quelch tui::smoke_tests`
Expected: this new test may already pass given Task 10 updated `draw` signature.

- [ ] **Step 3: Rewrite run() for EventStream**

Replace the `run` function in `crates/quelch/src/tui/mod.rs` (keep `TerminalGuard` unchanged) with:

```rust
/// Entry point: runs the TUI until Shutdown or Ctrl-C.
pub async fn run(
    config: Config,
    prefs_path: PathBuf,
    mut events_rx: mpsc::Receiver<QuelchEvent>,
    cmd_tx: mpsc::Sender<UiCommand>,
    drops_counter: Arc<AtomicU64>,
) -> Result<()> {
    use crossterm::event::EventStream;
    use futures::StreamExt;

    let prefs = Prefs::load(&prefs_path)?;
    let mut app = App::new(&config, prefs);

    let _guard = TerminalGuard::new()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut input_state = InputState::default();
    let start = std::time::Instant::now();

    let mut interval = tokio::time::interval(Duration::from_millis(200));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut key_events = EventStream::new();

    loop {
        tokio::select! {
            _ = interval.tick() => {
                while let Ok(ev) = events_rx.try_recv() {
                    app.apply(ev);
                }
                app.tick_spinner();
                app.drops = drops_counter.load(Ordering::Relaxed);
                terminal.draw(|f| {
                    crate::tui::layout::draw(f, &app, start.elapsed(), input_state.help_open());
                })?;
            }
            Some(Ok(ev)) = key_events.next() => {
                if let Event::Key(key) = ev {
                    if key.kind == KeyEventKind::Press {
                        match input_state.on_key(key, &mut app) {
                            InputOutcome::Quit => {
                                let _ = cmd_tx.send(UiCommand::Shutdown).await;
                                app.prefs.save(&prefs_path).ok();
                                return Ok(());
                            }
                            InputOutcome::Command(cmd) => {
                                let _ = cmd_tx.send(cmd).await;
                            }
                            InputOutcome::None => {}
                        }
                    }
                }
            }
        }
    }
}
```

Update the imports at the top of the file:

```rust
use anyhow::Result;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crossterm::event::{Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::sync::UiCommand;

use self::app::App;
use self::events::QuelchEvent;
use self::input::{InputOutcome, InputState};
use self::prefs::Prefs;
```

Also enable the `event-stream` feature for crossterm in `crates/quelch/Cargo.toml`:

```toml
[dependencies]
# ... existing lines ...
crossterm = { workspace = true, features = ["event-stream"] }
```

Remove the plain `crossterm.workspace = true` line in favor of the above. Add `futures = "0.3"` to `[dependencies]` (required by `EventStream::next`).

- [ ] **Step 4: Verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/quelch/src/tui/mod.rs crates/quelch/Cargo.toml crates/quelch/Cargo.lock Cargo.lock
git commit -m "tui::run: async EventStream + uptime + help-overlay gating

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

(If `Cargo.lock` is in workspace root only, adjust the add accordingly.)

---

## Task 13: Workspace deps — humantime, rand, tokio-util, assert_cmd

**Files:**
- Modify: `Cargo.toml` (workspace)
- Modify: `crates/quelch/Cargo.toml`

- [ ] **Step 1: Update workspace deps**

In the root `Cargo.toml`, add to `[workspace.dependencies]`:

```toml
humantime = "2"
rand = "0.8"
tokio-util = { version = "0.7", features = ["rt"] }
assert_cmd = "2"
```

In `crates/quelch/Cargo.toml` `[dependencies]`:

```toml
humantime.workspace = true
rand.workspace = true
tokio-util.workspace = true
```

And in `[dev-dependencies]`:

```toml
assert_cmd.workspace = true
```

- [ ] **Step 2: Verify**

```bash
cargo build --workspace
cargo test --workspace
```

Both must succeed with the new deps available.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock crates/quelch/Cargo.toml
git commit -m "Add humantime, rand, tokio-util, assert_cmd deps

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 14: Mock /_sim/* mutation endpoints

**Files:**
- Modify: `crates/quelch/src/mock/mod.rs`

- [ ] **Step 1: Failing test**

Append to `mock/mod.rs`'s `mod tests`:

```rust
    #[tokio::test]
    async fn sim_upsert_issue_adds_to_jira_store() {
        let base = spawn_test_server().await;
        let client = reqwest::Client::new();

        let resp = client
            .post(format!("{base}/_sim/jira/upsert_issue"))
            .json(&serde_json::json!({
                "project": "QUELCH",
                "key": "QUELCH-999",
                "summary": "sim-created",
                "description": "injected by sim",
            }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());

        // Verify subsequent /jira/rest/api/2/search includes the injected issue.
        let search = client
            .get(format!("{base}/jira/rest/api/2/search"))
            .header("authorization", format!("Bearer {}", MOCK_TOKEN))
            .query(&[("jql", "project = QUELCH"), ("maxResults", "500")])
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = search.json().await.unwrap();
        let issues = body.get("issues").unwrap().as_array().unwrap();
        assert!(
            issues.iter().any(|i| i.get("key").and_then(|k| k.as_str()) == Some("QUELCH-999")),
            "injected issue not in search result"
        );
    }

    #[tokio::test]
    async fn sim_upsert_page_adds_to_confluence_store() {
        let base = spawn_test_server().await;
        let client = reqwest::Client::new();

        let resp = client
            .post(format!("{base}/_sim/confluence/upsert_page"))
            .json(&serde_json::json!({
                "space": "INFRA",
                "id": "200500",
                "title": "sim-created",
                "body": "<p>injected</p>",
            }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());

        let search = client
            .get(format!("{base}/confluence/rest/api/content/search"))
            .header("authorization", format!("Bearer {}", MOCK_TOKEN))
            .query(&[("cql", "space = INFRA")])
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = search.json().await.unwrap();
        let results = body.get("results").unwrap().as_array().unwrap();
        assert!(results.iter().any(|r| r.get("id").and_then(|i| i.as_str()) == Some("200500")));
    }
```

- [ ] **Step 2: Fail**

Run: `cargo test -p quelch mock::tests::sim_upsert_issue_adds_to_jira_store mock::tests::sim_upsert_page_adds_to_confluence_store`
Expected: FAIL with 404.

- [ ] **Step 3: Implement**

The existing `mock/mod.rs` currently stores Jira issues and Confluence pages as static `Vec<Value>` from `data::jira_issues()` / `data::confluence_pages()`. We need to make them mutable shared state.

Change the axum `State` type to include these plus the Azure state. Introduce a `MockState` type that owns all three.

At the top of `crates/quelch/src/mock/mod.rs`, replace the `SharedState`/`AzureMockState` section with:

```rust
/// Per-index storage for Azure mock.
#[derive(Default)]
struct IndexStore {
    docs: HashMap<String, Value>,
}

#[derive(Default)]
struct AzureStore {
    indexes: HashMap<String, IndexStore>,
    pending_faults: Vec<u16>,
}

/// Top-level mock state — owns Azure + mutable Jira/Confluence stores.
#[derive(Default)]
struct MockState {
    azure: AzureStore,
    jira_issues: Vec<Value>,
    confluence_pages: Vec<Value>,
}

type SharedState = Arc<Mutex<MockState>>;

fn consume_fault(state: &SharedState) -> Option<u16> {
    let mut s = state.lock().unwrap();
    if s.azure.pending_faults.is_empty() {
        None
    } else {
        Some(s.azure.pending_faults.remove(0))
    }
}
```

Update `build_router` to seed the shared state from `data::jira_issues()` + `data::confluence_pages()`:

```rust
pub fn build_router() -> Router {
    let state = Arc::new(Mutex::new(MockState {
        azure: AzureStore::default(),
        jira_issues: data::jira_issues(),
        confluence_pages: data::confluence_pages(),
    }));

    Router::new()
        .route("/jira/rest/api/2/search", get(jira_search))
        .route("/confluence/rest/api/content/search", get(confluence_search))
        .route("/azure/indexes/{name}", get(azure_index_get))
        .route("/azure/indexes/{name}", put(azure_index_put))
        .route("/azure/indexes/{name}", delete(azure_index_delete))
        .route("/azure/indexes/{name}/docs/index", post(azure_index_docs_post))
        .route("/azure/indexes/{name}/docs/search", post(azure_index_search_post))
        .route("/azure/indexes/{name}/docs", get(azure_index_docs_list))
        .route("/azure/indexes", post(azure_indexes_collection_post))
        .route("/azure/_fault", post(azure_fault_post))
        // NEW sim mutation endpoints:
        .route("/_sim/jira/upsert_issue", post(sim_upsert_issue))
        .route("/_sim/confluence/upsert_page", post(sim_upsert_page))
        .route("/_sim/jira/comment", post(sim_add_comment))
        .with_state(state)
}
```

Update the existing `jira_search` and `confluence_search` handlers to pull from `state.jira_issues` / `state.confluence_pages` instead of calling `data::jira_issues()` / `data::confluence_pages()` each time.

Replace `async fn jira_search(headers: HeaderMap, Query(params): Query<JiraSearchParams>) -> ...` with:

```rust
async fn jira_search(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(params): Query<JiraSearchParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    check_auth(&headers)?;

    let all_issues = state.lock().unwrap().jira_issues.clone();
    let jql = params.jql.unwrap_or_default();

    // ... rest of function body unchanged, but operating on `all_issues` (Vec<Value>)
    //     instead of the `Vec<&Value>` from before. The filtering is identical.
```

The body should be:

```rust
    let filtered: Vec<Value> = all_issues
        .into_iter()
        .filter(|issue| {
            if jql.is_empty() {
                return true;
            }
            if let Some(project) = extract_jql_project(&jql) {
                let issue_project = issue["fields"]["project"]["key"].as_str().unwrap_or("");
                if !project.eq_ignore_ascii_case(issue_project) {
                    return false;
                }
            }
            if let Some(updated_since) = extract_jql_updated(&jql) {
                let issue_updated = issue["fields"]["updated"].as_str().unwrap_or("");
                if issue_updated < updated_since.as_str() {
                    return false;
                }
            }
            true
        })
        .collect();

    let start_at = params.start_at.unwrap_or(0);
    let max_results = params.max_results.unwrap_or(50);
    let total = filtered.len() as u64;

    let page: Vec<Value> = filtered
        .into_iter()
        .skip(start_at as usize)
        .take(max_results as usize)
        .collect();

    Ok(Json(json!({
        "expand": "schema,names",
        "startAt": start_at,
        "maxResults": max_results,
        "total": total,
        "issues": page
    })))
}
```

Same pattern for `confluence_search` — add `State(state): State<SharedState>` as the first param, replace `let all_pages = data::confluence_pages();` with `let all_pages = state.lock().unwrap().confluence_pages.clone();`, and change the filter to operate on owned `Vec<Value>`.

Implement the new mutation handlers:

```rust
#[derive(Debug, Deserialize)]
struct SimUpsertIssue {
    project: String,
    key: String,
    summary: String,
    #[serde(default)]
    description: String,
}

async fn sim_upsert_issue(
    State(state): State<SharedState>,
    Json(body): Json<SimUpsertIssue>,
) -> impl IntoResponse {
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f+0000")
        .to_string();
    let mut s = state.lock().unwrap();
    if let Some(existing) = s
        .jira_issues
        .iter_mut()
        .find(|i| i["key"].as_str() == Some(body.key.as_str()))
    {
        existing["fields"]["summary"] = json!(body.summary);
        existing["fields"]["description"] = json!(body.description);
        existing["fields"]["updated"] = json!(now);
    } else {
        s.jira_issues.push(json!({
            "id": body.key.clone(),
            "key": body.key.clone(),
            "fields": {
                "summary": body.summary,
                "description": body.description,
                "status": {
                    "name": "Open",
                    "statusCategory": { "name": "New", "id": 2, "key": "new" }
                },
                "priority": { "name": "Medium" },
                "issuetype": { "name": "Story" },
                "project": {
                    "id": body.project.clone(),
                    "key": body.project.clone(),
                    "name": body.project.clone(),
                },
                "labels": [] as [String; 0],
                "created": now.clone(),
                "updated": now,
                "comment": { "comments": [], "maxResults": 0, "total": 0, "startAt": 0 }
            }
        }));
    }
    (StatusCode::OK, Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
struct SimUpsertPage {
    space: String,
    id: String,
    title: String,
    #[serde(default)]
    body: String,
}

async fn sim_upsert_page(
    State(state): State<SharedState>,
    Json(body): Json<SimUpsertPage>,
) -> impl IntoResponse {
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f+0000")
        .to_string();
    let mut s = state.lock().unwrap();
    if let Some(existing) = s
        .confluence_pages
        .iter_mut()
        .find(|p| p["id"].as_str() == Some(body.id.as_str()))
    {
        existing["title"] = json!(body.title);
        if let Some(storage) = existing
            .get_mut("body")
            .and_then(|b| b.get_mut("storage"))
            .and_then(|s| s.get_mut("value"))
        {
            *storage = json!(body.body);
        }
        existing["version"]["when"] = json!(now);
    } else {
        s.confluence_pages.push(json!({
            "id": body.id,
            "type": "page",
            "status": "current",
            "title": body.title,
            "space": { "key": body.space, "name": "sim" },
            "body": { "storage": { "value": body.body, "representation": "storage" } },
            "version": { "number": 1, "when": now.clone() },
            "history": { "createdDate": now, "latest": true },
            "ancestors": [],
            "metadata": { "labels": { "results": [], "start": 0, "limit": 200, "size": 0 } },
            "_links": {}
        }));
    }
    (StatusCode::OK, Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
struct SimAddComment {
    key: String,
    body: String,
    #[serde(default)]
    author: String,
}

async fn sim_add_comment(
    State(state): State<SharedState>,
    Json(body): Json<SimAddComment>,
) -> impl IntoResponse {
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f+0000")
        .to_string();
    let mut s = state.lock().unwrap();
    if let Some(issue) = s
        .jira_issues
        .iter_mut()
        .find(|i| i["key"].as_str() == Some(body.key.as_str()))
    {
        let comment = json!({
            "author": { "displayName": body.author },
            "body": body.body,
            "created": now,
            "updated": now,
        });
        if let Some(comments) = issue
            .get_mut("fields")
            .and_then(|f| f.get_mut("comment"))
            .and_then(|c| c.get_mut("comments"))
            .and_then(|v| v.as_array_mut())
        {
            comments.push(comment);
        }
        issue["fields"]["updated"] = json!(now);
        (StatusCode::OK, Json(json!({ "ok": true })))
    } else {
        (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" })))
    }
}
```

Update the existing Azure handlers to use the new `state.lock().unwrap().azure.xxx` path throughout. Concretely change every `s.indexes.` to `s.azure.indexes.` and every `s.pending_faults.` to `s.azure.pending_faults.`.

- [ ] **Step 4: Verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/quelch/src/mock/mod.rs
git commit -m "mock: /_sim/* mutation endpoints; shared mutable Jira/Confluence stores

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 15: sim::opts + CLI Sim subcommand (wire-only, not yet executable)

**Files:**
- Create: `crates/quelch/src/sim/mod.rs` (stub)
- Create: `crates/quelch/src/sim/opts.rs`
- Modify: `crates/quelch/src/cli.rs` (add `Sim { ... }` variant)
- Modify: `crates/quelch/src/lib.rs` (add `pub mod sim;`)
- Modify: `crates/quelch/src/main.rs` (add a no-op `Commands::Sim` arm that calls `sim::run(...)`)

- [ ] **Step 1: Failing test**

Create `crates/quelch/src/sim/opts.rs`:

```rust
//! SimOpts — parameters controlling the simulator.

use std::time::Duration;

#[derive(Debug, Clone)]
pub struct SimOpts {
    pub duration: Option<Duration>,
    pub seed: Option<u64>,
    pub rate_multiplier: f64,
    pub fault_rate: f64,
    pub assert_docs: Option<u64>,
    pub mock_port: Option<u16>,
}

impl Default for SimOpts {
    fn default() -> Self {
        Self {
            duration: None,
            seed: None,
            rate_multiplier: 1.0,
            fault_rate: 0.03,
            assert_docs: None,
            mock_port: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let o = SimOpts::default();
        assert_eq!(o.rate_multiplier, 1.0);
        assert!((o.fault_rate - 0.03).abs() < 1e-9);
        assert!(o.duration.is_none());
    }
}
```

Create `crates/quelch/src/sim/mod.rs` stub:

```rust
//! quelch simulator — runs the real engine against an in-process fake world.

pub mod opts;

use anyhow::Result;

pub use opts::SimOpts;

/// Entry point. Filled in by subsequent tasks.
pub async fn run(_opts: SimOpts) -> Result<()> {
    anyhow::bail!("sim::run not yet implemented");
}
```

Add `pub mod sim;` to the bottom of `crates/quelch/src/lib.rs`.

- [ ] **Step 2: Register `Sim` CLI variant**

In `crates/quelch/src/cli.rs`, add a new variant to `Commands`:

```rust
    /// Run quelch against a fully simulated environment for local testing and CI.
    Sim {
        /// Run for this long then exit. Default: run until Ctrl-C. Example: 30s, 2m, 1h.
        #[arg(long)]
        duration: Option<humantime::Duration>,
        /// Seed the activity generator for reproducible runs.
        #[arg(long)]
        seed: Option<u64>,
        /// Scale activity rate. 1.0 = default, 2.0 = twice as fast.
        #[arg(long, default_value = "1.0")]
        rate_multiplier: f64,
        /// Probability each Azure request gets a 429 or 503. 0.0 disables.
        #[arg(long, default_value = "0.03")]
        fault_rate: f64,
        /// CI-friendly: fail with exit code 1 if fewer than N docs are indexed.
        #[arg(long)]
        assert_docs: Option<u64>,
    },
```

Add a corresponding arm in `main.rs`'s `match cli.command { ... }` BEFORE the final `Commands::Mock { port }` arm or wherever most suits:

```rust
        Commands::Sim {
            duration,
            seed,
            rate_multiplier,
            fault_rate,
            assert_docs,
        } => {
            let opts = quelch::sim::SimOpts {
                duration: duration.map(|d| d.into()),
                seed,
                rate_multiplier,
                fault_rate,
                assert_docs,
                mock_port: None,
            };
            quelch::sim::run(opts).await
        }
```

- [ ] **Step 3: Verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/quelch/src/sim/ crates/quelch/src/cli.rs crates/quelch/src/lib.rs crates/quelch/src/main.rs
git commit -m "sim: scaffold SimOpts + Commands::Sim subcommand (not yet executable)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 16: sim::world starter corpus

**Files:**
- Create: `crates/quelch/src/sim/world.rs`

- [ ] **Step 1: Failing test**

Create `crates/quelch/src/sim/world.rs`:

```rust
//! Builds the sim's starter corpus by posting to the mock's /_sim/* endpoints.
//! Expects a mock server already running at `base_url`.

use anyhow::{Context, Result};
use rand::{SeedableRng, rngs::StdRng};

pub async fn seed(base_url: &str, seed: Option<u64>) -> Result<()> {
    let mut rng = match seed {
        Some(s) => StdRng::seed_from_u64(s),
        None => StdRng::from_entropy(),
    };
    let client = reqwest::Client::new();

    seed_jira_project(&client, base_url, "QUELCH", 40, &mut rng).await?;
    seed_jira_project(&client, base_url, "DEMO", 15, &mut rng).await?;
    seed_confluence_space(&client, base_url, "QUELCH", 20, &mut rng).await?;
    seed_confluence_space(&client, base_url, "INFRA", 8, &mut rng).await?;
    Ok(())
}

async fn seed_jira_project(
    client: &reqwest::Client,
    base: &str,
    project: &str,
    target_count: usize,
    _rng: &mut StdRng,
) -> Result<()> {
    // The mock already ships with built-in QUELCH and DEMO entries. We top
    // up to `target_count` with generated issues whose keys do not collide.
    for i in 0..target_count {
        let key = format!("{project}-SIM-{i}");
        let summary = format!("[{project}] generated issue {i}");
        let description = format!("Auto-generated entry for simulator starter corpus.");
        let body = serde_json::json!({
            "project": project,
            "key": key,
            "summary": summary,
            "description": description,
        });
        client
            .post(format!("{base}/_sim/jira/upsert_issue"))
            .json(&body)
            .send()
            .await
            .with_context(|| format!("seed jira {key}"))?
            .error_for_status()?;
    }
    Ok(())
}

async fn seed_confluence_space(
    client: &reqwest::Client,
    base: &str,
    space: &str,
    target_count: usize,
    _rng: &mut StdRng,
) -> Result<()> {
    for i in 0..target_count {
        let id = format!("{space}-SIM-{i}");
        let title = format!("[{space}] generated page {i}");
        let body = format!("<h1>{title}</h1><p>Auto-generated for simulator.</p>");
        client
            .post(format!("{base}/_sim/confluence/upsert_page"))
            .json(&serde_json::json!({
                "space": space,
                "id": id,
                "title": title,
                "body": body,
            }))
            .send()
            .await
            .with_context(|| format!("seed confluence {id}"))?
            .error_for_status()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    async fn spawn_mock() -> String {
        let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, crate::mock::build_router()).await.unwrap();
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn seeds_expected_counts() {
        let base = spawn_mock().await;
        seed(&base, Some(42)).await.unwrap();

        let client = reqwest::Client::new();
        let r = client
            .get(format!("{base}/jira/rest/api/2/search"))
            .header("authorization", "Bearer mock-pat-token")
            .query(&[("jql", "project = QUELCH"), ("maxResults", "500")])
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = r.json().await.unwrap();
        let issues = body.get("issues").unwrap().as_array().unwrap();
        // 17 built-in + 40 seeded = 57. Allow range to tolerate data.rs changes.
        assert!(issues.len() >= 57, "QUELCH issues: {}", issues.len());
    }
}
```

Register in `sim/mod.rs`:

```rust
pub mod opts;
pub mod world;

use anyhow::Result;

pub use opts::SimOpts;

pub async fn run(_opts: SimOpts) -> Result<()> {
    anyhow::bail!("sim::run not yet implemented");
}
```

- [ ] **Step 2: Verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/quelch/src/sim/
git commit -m "sim::world: starter-corpus seeder via /_sim/* endpoints

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 17: sim::scheduler + sim::{jira_gen,confluence_gen}

**Files:**
- Create: `crates/quelch/src/sim/jira_gen.rs`
- Create: `crates/quelch/src/sim/confluence_gen.rs`
- Create: `crates/quelch/src/sim/scheduler.rs`

- [ ] **Step 1: Create mutation generators**

Create `crates/quelch/src/sim/jira_gen.rs`:

```rust
//! Jira-specific mutation calls: create/update issues, add comments.

use anyhow::Result;
use rand::Rng;
use rand::rngs::StdRng;

pub async fn mutate(client: &reqwest::Client, base: &str, rng: &mut StdRng) -> Result<()> {
    let project = pick_project(rng);
    // 70% update / 30% create
    if rng.r#gen::<f32>() < 0.3 {
        create_issue(client, base, project, rng).await
    } else {
        update_random_issue(client, base, project, rng).await
    }
}

fn pick_project(rng: &mut StdRng) -> &'static str {
    if rng.r#gen::<f32>() < 0.7 { "QUELCH" } else { "DEMO" }
}

async fn create_issue(
    client: &reqwest::Client,
    base: &str,
    project: &str,
    rng: &mut StdRng,
) -> Result<()> {
    let n: u32 = rng.gen_range(1000..99999);
    let key = format!("{project}-{n}");
    let body = serde_json::json!({
        "project": project,
        "key": key,
        "summary": summary_text(rng),
        "description": "Created by sim.",
    });
    client
        .post(format!("{base}/_sim/jira/upsert_issue"))
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn update_random_issue(
    client: &reqwest::Client,
    base: &str,
    project: &str,
    rng: &mut StdRng,
) -> Result<()> {
    // Ask the mock for the first page; pick one to bump.
    let search = client
        .get(format!("{base}/jira/rest/api/2/search"))
        .header("authorization", "Bearer mock-pat-token")
        .query(&[
            ("jql", format!("project = {project}").as_str()),
            ("maxResults", "100"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    let issues = search
        .get("issues")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if issues.is_empty() {
        return create_issue(client, base, project, rng).await;
    }
    let idx = rng.gen_range(0..issues.len());
    let chosen = &issues[idx];
    let key = chosen
        .get("key")
        .and_then(|k| k.as_str())
        .unwrap_or("UNKNOWN")
        .to_string();
    let summary = chosen
        .get("fields")
        .and_then(|f| f.get("summary"))
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let body = serde_json::json!({
        "project": project,
        "key": key,
        "summary": format!("{summary} (updated)"),
        "description": "Updated by sim.",
    });
    client
        .post(format!("{base}/_sim/jira/upsert_issue"))
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

fn summary_text(rng: &mut StdRng) -> String {
    const WORDS: &[&str] = &[
        "fix", "refactor", "implement", "investigate", "document", "optimise",
        "cleanup", "polish", "design", "review", "audit",
    ];
    const NOUNS: &[&str] = &[
        "sync loop", "mock server", "TUI header", "Azure retry",
        "config parser", "embedding cache", "state file", "cursor",
    ];
    let verb = WORDS[rng.gen_range(0..WORDS.len())];
    let noun = NOUNS[rng.gen_range(0..NOUNS.len())];
    format!("{verb} {noun}")
}
```

Create `crates/quelch/src/sim/confluence_gen.rs`:

```rust
//! Confluence-specific mutation calls.

use anyhow::Result;
use rand::Rng;
use rand::rngs::StdRng;

pub async fn mutate(client: &reqwest::Client, base: &str, rng: &mut StdRng) -> Result<()> {
    let space = if rng.r#gen::<f32>() < 0.7 {
        "QUELCH"
    } else {
        "INFRA"
    };
    // 85% update / 15% create
    if rng.r#gen::<f32>() < 0.15 {
        create_page(client, base, space, rng).await
    } else {
        update_random_page(client, base, space, rng).await
    }
}

async fn create_page(
    client: &reqwest::Client,
    base: &str,
    space: &str,
    rng: &mut StdRng,
) -> Result<()> {
    let id: u32 = rng.gen_range(1_000_000..9_999_999);
    let title = format!("New page {id}");
    let body = format!("<h1>{title}</h1><p>Created by sim.</p>");
    client
        .post(format!("{base}/_sim/confluence/upsert_page"))
        .json(&serde_json::json!({
            "space": space,
            "id": id.to_string(),
            "title": title,
            "body": body,
        }))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn update_random_page(
    client: &reqwest::Client,
    base: &str,
    space: &str,
    rng: &mut StdRng,
) -> Result<()> {
    let search = client
        .get(format!("{base}/confluence/rest/api/content/search"))
        .header("authorization", "Bearer mock-pat-token")
        .query(&[("cql", format!("space = {space}").as_str()), ("limit", "100")])
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    let pages = search
        .get("results")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if pages.is_empty() {
        return create_page(client, base, space, rng).await;
    }
    let idx = rng.gen_range(0..pages.len());
    let chosen = &pages[idx];
    let id = chosen
        .get("id")
        .and_then(|k| k.as_str())
        .unwrap_or("")
        .to_string();
    let title = chosen
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("untitled")
        .to_string();
    client
        .post(format!("{base}/_sim/confluence/upsert_page"))
        .json(&serde_json::json!({
            "space": space,
            "id": id,
            "title": format!("{title} (v+1)"),
            "body": "<p>Updated by sim.</p>",
        }))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}
```

- [ ] **Step 2: Create scheduler**

Create `crates/quelch/src/sim/scheduler.rs`:

```rust
//! Burst-aware Poisson activity scheduler. Calls into jira_gen/confluence_gen
//! at varying intervals with occasional burst and lull modes.

use anyhow::Result;
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;
use std::time::{Duration, Instant};

use super::{confluence_gen, jira_gen};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Burst,
    Lull,
}

pub async fn run(
    base: String,
    seed: Option<u64>,
    rate_multiplier: f64,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<()> {
    let mut rng = match seed {
        Some(s) => StdRng::seed_from_u64(s),
        None => StdRng::from_entropy(),
    };
    let client = reqwest::Client::new();
    let mut mode = Mode::Normal;
    let mut mode_until = Instant::now();

    while !cancel.is_cancelled() {
        // Maybe transition mode.
        if Instant::now() >= mode_until {
            mode = next_mode(&mut rng);
            mode_until = Instant::now()
                + Duration::from_secs(match mode {
                    Mode::Burst => rng.gen_range(30..=60),
                    Mode::Lull => rng.gen_range(60..=90),
                    Mode::Normal => rng.gen_range(30..=60),
                });
        }

        let dwell_ms = match mode {
            Mode::Burst => rng.gen_range(100..=500),
            Mode::Lull => rng.gen_range(5_000..=15_000),
            Mode::Normal => rng.gen_range(2_000..=8_000),
        };
        let scaled = (dwell_ms as f64 / rate_multiplier).max(10.0) as u64;
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(scaled)) => {}
            _ = cancel.cancelled() => break,
        }

        // 65% Jira / 35% Confluence.
        let pick_jira = rng.r#gen::<f32>() < 0.65;
        let res = if pick_jira {
            jira_gen::mutate(&client, &base, &mut rng).await
        } else {
            confluence_gen::mutate(&client, &base, &mut rng).await
        };
        if let Err(e) = res {
            tracing::warn!(error = %e, "sim: mutation failed (continuing)");
        }
    }
    Ok(())
}

fn next_mode(rng: &mut StdRng) -> Mode {
    let roll: f32 = rng.r#gen();
    if roll < 0.30 {
        Mode::Burst
    } else if roll < 0.50 {
        Mode::Lull
    } else {
        Mode::Normal
    }
}
```

Register in `sim/mod.rs`:

```rust
pub mod confluence_gen;
pub mod jira_gen;
pub mod opts;
pub mod scheduler;
pub mod world;

use anyhow::Result;

pub use opts::SimOpts;

pub async fn run(_opts: SimOpts) -> Result<()> {
    anyhow::bail!("sim::run not yet implemented");
}
```

- [ ] **Step 3: Verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/quelch/src/sim/
git commit -m "sim: jira_gen + confluence_gen + burst-aware scheduler

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 18: sim::embedder (SimEmbedder)

**Files:**
- Create: `crates/quelch/src/sim/embedder.rs`

- [ ] **Step 1: Implementation**

Create `crates/quelch/src/sim/embedder.rs`:

```rust
//! SimEmbedder: wraps DeterministicEmbedder with jittery sleep to make
//! Azure p50/p95 charts meaningful.

use anyhow::Result;
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;
use std::sync::Mutex;
use std::time::Duration;

use crate::sync::embedder::{DeterministicEmbedder, EmbedFuture, Embedder};

pub struct SimEmbedder {
    inner: DeterministicEmbedder,
    rng: Mutex<StdRng>,
}

impl SimEmbedder {
    pub fn new(dims: usize, seed: Option<u64>) -> Self {
        let rng = match seed {
            Some(s) => StdRng::seed_from_u64(s.wrapping_add(1)),
            None => StdRng::from_entropy(),
        };
        Self {
            inner: DeterministicEmbedder::new(dims),
            rng: Mutex::new(rng),
        }
    }

    fn pick_jitter(&self) -> Duration {
        let mut r = self.rng.lock().unwrap();
        let ms = if r.r#gen::<f32>() < 0.01 {
            r.gen_range(300u64..=500)
        } else {
            r.gen_range(20u64..=150)
        };
        Duration::from_millis(ms)
    }
}

impl Embedder for SimEmbedder {
    fn embed_one<'a>(&'a self, text: &'a str) -> EmbedFuture<'a> {
        let jitter = self.pick_jitter();
        Box::pin(async move {
            tokio::time::sleep(jitter).await;
            self.inner.embed_one(text).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn same_text_same_vector() {
        let e = SimEmbedder::new(8, Some(42));
        let a = e.embed_one("hi").await.unwrap();
        let b = e.embed_one("hi").await.unwrap();
        assert_eq!(a, b);
    }

    #[tokio::test]
    async fn non_zero_latency() {
        let e = SimEmbedder::new(8, Some(42));
        let t = std::time::Instant::now();
        e.embed_one("x").await.unwrap();
        assert!(t.elapsed() >= Duration::from_millis(20));
    }
}
```

Also update `crates/quelch/src/sync/embedder.rs` — the `EmbedFuture<'a>` alias may currently be a private `type`; make it `pub` so `sim::embedder::SimEmbedder` can name it. Find the `type EmbedFuture<'a> = ...` line and change to `pub type EmbedFuture<'a> = ...`.

Register in `sim/mod.rs`:

```rust
pub mod confluence_gen;
pub mod embedder;
pub mod jira_gen;
pub mod opts;
pub mod scheduler;
pub mod world;
```

- [ ] **Step 2: Verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/quelch/src/sim/embedder.rs crates/quelch/src/sim/mod.rs crates/quelch/src/sync/embedder.rs
git commit -m "sim::embedder: SimEmbedder with jittery latency (1% long tail to 500ms)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 19: sim::azure_faults

**Files:**
- Create: `crates/quelch/src/sim/azure_faults.rs`

- [ ] **Step 1: Implementation**

Create `crates/quelch/src/sim/azure_faults.rs`:

```rust
//! Periodic fault injection into the mock Azure endpoint.

use anyhow::Result;
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;
use std::time::Duration;

pub async fn run(
    base: String,
    fault_rate: f64,
    seed: Option<u64>,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<()> {
    if fault_rate <= 0.0 {
        return Ok(());
    }
    let mut rng = match seed {
        Some(s) => StdRng::seed_from_u64(s.wrapping_add(2)),
        None => StdRng::from_entropy(),
    };
    let client = reqwest::Client::new();

    // Interpret fault_rate as faults per request. Azure request rate depends
    // on engine activity; we approximate by injecting at ~(fault_rate / avg_req_gap)
    // timed intervals. Simple approach: try injecting every 2-8 seconds with
    // probability `fault_rate * 10` (since each tick could hit several requests).
    loop {
        let dwell = Duration::from_millis(rng.gen_range(2_000..=8_000));
        tokio::select! {
            _ = tokio::time::sleep(dwell) => {}
            _ = cancel.cancelled() => return Ok(()),
        }
        let should_fault = rng.r#gen::<f64>() < (fault_rate * 10.0).min(1.0);
        if !should_fault {
            continue;
        }
        let status = if rng.r#gen::<f32>() < 0.6 { 429 } else { 503 };
        let _ = client
            .post(format!("{base}/azure/_fault"))
            .json(&serde_json::json!({"count": 1, "status": status}))
            .send()
            .await;
    }
}
```

Register in `sim/mod.rs`:

```rust
pub mod azure_faults;
pub mod confluence_gen;
pub mod embedder;
pub mod jira_gen;
pub mod opts;
pub mod scheduler;
pub mod world;
```

- [ ] **Step 2: Verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/quelch/src/sim/azure_faults.rs crates/quelch/src/sim/mod.rs
git commit -m "sim::azure_faults: periodic 429/503 injection into mock /azure/_fault

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 20: sim::run orchestration

**Files:**
- Rewrite: `crates/quelch/src/sim/mod.rs` (implement `run`)

- [ ] **Step 1: Implementation**

Replace the content of `crates/quelch/src/sim/mod.rs`:

```rust
//! quelch simulator — runs the real engine against an in-process fake world.
//!
//! Spins up: the existing axum mock server, starter corpus seeder, burst-aware
//! activity scheduler, Azure fault injector, simulated embedder; then the real
//! quelch engine (`sync::run_sync_with`) and optionally the TUI.

pub mod azure_faults;
pub mod confluence_gen;
pub mod embedder;
pub mod jira_gen;
pub mod opts;
pub mod scheduler;
pub mod world;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio_util::sync::CancellationToken;

pub use opts::SimOpts;

use crate::azure::schema::EmbeddingConfig;
use crate::config::{
    AuthConfig, AzureConfig, Config, ConfluenceSourceConfig, JiraSourceConfig, SourceConfig,
    SyncConfig,
};
use crate::sim::embedder::SimEmbedder;
use crate::sync::{IndexMode, UiCommand};

const MOCK_PAT: &str = "mock-pat-token";

/// Runs the simulator until `opts.duration` elapses or Ctrl-C is pressed.
/// Returns Ok iff the run ended successfully and `assert_docs` (if set) was met.
pub async fn run(opts: SimOpts) -> Result<()> {
    let cancel = CancellationToken::new();
    let ctrl_c_cancel = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            ctrl_c_cancel.cancel();
        }
    });

    // 1. Start mock server on random port.
    let listener = tokio::net::TcpListener::bind(SocketAddr::from((
        [127, 0, 0, 1],
        opts.mock_port.unwrap_or(0),
    )))
    .await
    .context("bind mock port")?;
    let mock_addr = listener.local_addr()?;
    let mock_cancel = cancel.clone();
    let mock_handle = tokio::spawn(async move {
        let router = crate::mock::build_router();
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async move { mock_cancel.cancelled().await })
            .await;
    });
    let base = format!("http://{mock_addr}");
    tracing::info!(mock = %base, "sim: mock server up");

    // 2. Seed starter corpus.
    world::seed(&base, opts.seed).await.context("seed corpus")?;
    tracing::info!("sim: starter corpus seeded");

    // 3. Spawn scheduler.
    let scheduler_cancel = cancel.clone();
    let scheduler_base = base.clone();
    let scheduler_rate = opts.rate_multiplier;
    let scheduler_seed = opts.seed;
    let scheduler_handle = tokio::spawn(async move {
        let _ = scheduler::run(scheduler_base, scheduler_seed, scheduler_rate, scheduler_cancel)
            .await;
    });

    // 4. Spawn fault injector.
    let fault_cancel = cancel.clone();
    let fault_base = base.clone();
    let fault_seed = opts.seed;
    let fault_rate = opts.fault_rate;
    let fault_handle = tokio::spawn(async move {
        let _ = azure_faults::run(fault_base, fault_rate, fault_seed, fault_cancel).await;
    });

    // 5. Build sim Config and Embedder.
    let config = sim_config(&base);
    let embedding = EmbeddingConfig {
        dimensions: 8,
        vectorizer_json: serde_json::json!({}),
    };
    let embedder = SimEmbedder::new(8, opts.seed);

    // 6. Run engine in background (watch-style loop).
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<UiCommand>(16);
    let state_path = std::env::temp_dir().join(format!("quelch-sim-{}.json", std::process::id()));

    let engine_cancel = cancel.clone();
    let engine_config = config.clone();
    let engine_state_path = state_path.clone();
    let engine_handle = tokio::spawn(async move {
        let _ = run_engine_loop(
            engine_config,
            engine_state_path,
            embedding,
            embedder,
            cmd_rx_ref(&mut cmd_rx),
            engine_cancel,
        )
        .await;
    });

    // 7. Wait for duration OR cancel.
    let started = Instant::now();
    if let Some(duration) = opts.duration {
        tokio::select! {
            _ = tokio::time::sleep(duration) => cancel.cancel(),
            _ = cancel.cancelled() => {}
        }
    } else {
        cancel.cancelled().await;
    }

    // 8. Graceful shutdown.
    let _ = cmd_tx.send(UiCommand::Shutdown).await;
    let _ = engine_handle.await;
    scheduler_handle.abort();
    fault_handle.abort();
    mock_handle.abort();

    // 9. Evaluate assert_docs.
    let docs = synced_doc_count(&state_path).unwrap_or(0);
    println!(
        "sim: {:.1}s, {} docs synced",
        started.elapsed().as_secs_f32(),
        docs
    );
    if let Some(threshold) = opts.assert_docs
        && docs < threshold
    {
        anyhow::bail!("assert_docs failed: only {docs} < {threshold}");
    }
    Ok(())
}

fn cmd_rx_ref<T>(r: &mut tokio::sync::mpsc::Receiver<T>) -> &mut tokio::sync::mpsc::Receiver<T> {
    r
}

async fn run_engine_loop(
    config: Config,
    state_path: PathBuf,
    embedding: EmbeddingConfig,
    embedder: SimEmbedder,
    _cmd_rx: &mut tokio::sync::mpsc::Receiver<UiCommand>,
    cancel: CancellationToken,
) -> Result<()> {
    let (_tx, mut rx) = tokio::sync::mpsc::channel::<UiCommand>(1);

    let mut cycle: u64 = 0;
    while !cancel.is_cancelled() {
        cycle += 1;
        let _ = crate::sync::run_sync_with(
            &config,
            &state_path,
            &embedding,
            IndexMode::AutoCreate,
            Some(&embedder as &dyn crate::sync::embedder::Embedder),
            None,
            &mut rx,
            cycle,
        )
        .await;
        // Wait between cycles with cancel-awareness.
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
            _ = cancel.cancelled() => break,
        }
    }
    Ok(())
}

fn sim_config(base: &str) -> Config {
    Config {
        azure: AzureConfig {
            endpoint: format!("{base}/azure"),
            api_key: "ignored".into(),
        },
        sources: vec![
            SourceConfig::Jira(JiraSourceConfig {
                name: "sim-jira".into(),
                url: format!("{base}/jira"),
                auth: AuthConfig::DataCenter { pat: MOCK_PAT.into() },
                projects: vec!["QUELCH".into(), "DEMO".into()],
                index: "sim-jira-issues".into(),
            }),
            SourceConfig::Confluence(ConfluenceSourceConfig {
                name: "sim-confluence".into(),
                url: format!("{base}/confluence"),
                auth: AuthConfig::DataCenter { pat: MOCK_PAT.into() },
                spaces: vec!["QUELCH".into(), "INFRA".into()],
                index: "sim-confluence-pages".into(),
            }),
        ],
        sync: SyncConfig::default(),
    }
}

fn synced_doc_count(state_path: &std::path::Path) -> Result<u64> {
    let raw = std::fs::read_to_string(state_path)?;
    let v: serde_json::Value = serde_json::from_str(&raw)?;
    let mut total = 0u64;
    if let Some(sources) = v.get("sources").and_then(|s| s.as_object()) {
        for (_, src) in sources {
            if let Some(subs) = src.get("subsources").and_then(|s| s.as_object()) {
                for (_, sub) in subs {
                    if let Some(n) = sub.get("documents_synced").and_then(|n| n.as_u64()) {
                        total += n;
                    }
                }
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn short_run_succeeds() {
        let opts = SimOpts {
            duration: Some(Duration::from_millis(500)),
            seed: Some(42),
            rate_multiplier: 5.0,
            fault_rate: 0.0,
            assert_docs: None,
            mock_port: None,
        };
        run(opts).await.unwrap();
    }
}
```

- [ ] **Step 2: Sim-mode log filter in main.rs**

In `crates/quelch/src/main.rs`, update `install_plain` to take the subcommand and choose a filter appropriately. Find the `install_plain` function and adjust:

```rust
fn install_plain(verbose: u8, quiet: bool, json: bool, is_sim: bool) {
    let filter = match (quiet, verbose, is_sim) {
        (true, _, _) => "error".to_string(),
        (_, 0, true) => "quelch=warn,sim=info".to_string(),
        (_, 0, false) => "quelch=info".to_string(),
        (_, 1, true) => "quelch=info,sim=debug".to_string(),
        (_, 1, false) => "quelch=debug".to_string(),
        (_, 2, _) => "quelch=debug,sim=trace,reqwest=debug".to_string(),
        _ => "trace".to_string(),
    };
    let builder = tracing_subscriber::fmt().with_env_filter(EnvFilter::new(&filter));
    if json {
        builder.json().init();
    } else {
        builder.init();
    }
}
```

Update the caller (there's one in `main()`):

```rust
    let mode = decide_mode(&cli, &cli.command);
    let is_sim = matches!(&cli.command, Commands::Sim { .. });
    let tui_inputs = match mode {
        LogMode::Plain => {
            install_plain(cli.verbose, cli.quiet, cli.json, is_sim);
            None
        }
        LogMode::Tui => {
            let (rx, drops) = install_tui();
            Some((rx, drops))
        }
    };
```

- [ ] **Step 3: Verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/quelch/src/sim/mod.rs crates/quelch/src/main.rs
git commit -m "sim::run: orchestration (mock + scheduler + faults + engine + embedder)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 21: Integration test — sim_headless

**Files:**
- Create: `crates/quelch/tests/sim_headless.rs`

- [ ] **Step 1: Implementation**

Create `crates/quelch/tests/sim_headless.rs`:

```rust
//! Spawns `quelch sim` as a child process and asserts the CI-contract.

use assert_cmd::Command;
use std::time::Duration;

#[test]
fn sim_runs_briefly_and_syncs_some_docs() {
    let mut cmd = Command::cargo_bin("quelch").unwrap();
    cmd.arg("sim")
        .arg("--duration")
        .arg("3s")
        .arg("--seed")
        .arg("42")
        .arg("--no-tui")
        .arg("--rate-multiplier")
        .arg("5.0")
        .arg("--assert-docs")
        .arg("5")
        .timeout(Duration::from_secs(30))
        .assert()
        .success();
}
```

- [ ] **Step 2: Verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --test sim_headless
cargo test --workspace
git add crates/quelch/tests/sim_headless.rs
git commit -m "Add sim_headless integration test

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 22: CI — sim-smoke-test job

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Find existing CI file**

Run: `cat .github/workflows/ci.yml | head -80`

Note the shape of existing jobs (`test`, `lint`, etc.) so the new job fits the convention.

- [ ] **Step 2: Append new job**

At the bottom of `.github/workflows/ci.yml`, append (adjust indentation if the file uses a different style):

```yaml
  sim-smoke-test:
    name: Simulator smoke test
    runs-on: ubuntu-latest
    needs: test
    steps:
      - uses: actions/checkout@v5
      - uses: dtolnay/rust-toolchain@stable
      - name: Build release binary
        run: cargo build --release -p quelch
      - name: Run simulator
        run: |
          ./target/release/quelch sim \
            --duration 30s \
            --seed 42 \
            --no-tui \
            --rate-multiplier 2.0 \
            --assert-docs 20
        timeout-minutes: 3
```

If the existing `test` job's `name:` value differs from `test`, substitute accordingly.

- [ ] **Step 3: Commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add .github/workflows/ci.yml
git commit -m "CI: sim-smoke-test job runs quelch sim on every push

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 23: Clean up stray scratch files

**Files:**
- Delete: `tmp.txt`, `build_output.json`, `build_output_clean.json`
- Modify: `.gitignore` if any stray pattern needs pinning

- [ ] **Step 1: Inspect**

Run: `ls /Users/kristofer/repos/quelch/{tmp.txt,build_output.json,build_output_clean.json}` — confirm they exist.

- [ ] **Step 2: Delete and gitignore**

```bash
git rm tmp.txt build_output.json build_output_clean.json
```

Append to `.gitignore`:

```
# Developer scratch
/tmp.txt
/build_output*.json
```

- [ ] **Step 3: Commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add -A
git commit -m "Clean up stray scratch files

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 24: Headless TUI snapshot capture (`--snapshot-to`)

Adds a flag to `quelch sim` that runs the sim with a ratatui `TestBackend` and writes a text dump of rendered frames to a file. This is the mechanism that lets an AI agent (or CI) verify the TUI actually renders correctly without a real terminal.

**Files:**
- Modify: `crates/quelch/src/cli.rs` (two new flags on `Sim`)
- Modify: `crates/quelch/src/sim/mod.rs` (new `run_tui_snapshot` helper)
- Modify: `crates/quelch/src/tui/layout.rs` (expose nothing new — reuse existing `draw`)

- [ ] **Step 1: Add flags to CLI**

In `crates/quelch/src/cli.rs` `Commands::Sim`, append two more fields:

```rust
        /// Render the TUI to a headless backend and write a multi-frame text
        /// dump to this file. Enables deterministic verification of the TUI
        /// from CI or an AI agent. Implies --no-tui for stdout.
        #[arg(long)]
        snapshot_to: Option<PathBuf>,
        /// Number of frames to capture when --snapshot-to is set.
        #[arg(long, default_value = "10")]
        snapshot_frames: u32,
```

Add `use std::path::PathBuf;` at the top of `cli.rs` if not already present.

In `main.rs` Commands::Sim dispatch arm, extend `SimOpts` construction to include the new fields and update `SimOpts` in `sim/opts.rs`:

```rust
// In sim/opts.rs:
pub struct SimOpts {
    pub duration: Option<Duration>,
    pub seed: Option<u64>,
    pub rate_multiplier: f64,
    pub fault_rate: f64,
    pub assert_docs: Option<u64>,
    pub mock_port: Option<u16>,
    pub snapshot_to: Option<std::path::PathBuf>,
    pub snapshot_frames: u32,
}

impl Default for SimOpts {
    fn default() -> Self {
        Self {
            duration: None,
            seed: None,
            rate_multiplier: 1.0,
            fault_rate: 0.03,
            assert_docs: None,
            mock_port: None,
            snapshot_to: None,
            snapshot_frames: 10,
        }
    }
}
```

In `main.rs` dispatch:

```rust
        Commands::Sim {
            duration,
            seed,
            rate_multiplier,
            fault_rate,
            assert_docs,
            snapshot_to,
            snapshot_frames,
        } => {
            let opts = quelch::sim::SimOpts {
                duration: duration.map(|d| d.into()),
                seed,
                rate_multiplier,
                fault_rate,
                assert_docs,
                mock_port: None,
                snapshot_to,
                snapshot_frames,
            };
            quelch::sim::run(opts).await
        }
```

- [ ] **Step 2: Implement snapshot mode**

In `crates/quelch/src/sim/mod.rs`, at the end of the file add:

```rust
/// Run the sim rendering into a headless ratatui TestBackend and write
/// N frames of the full rendered buffer to `path`. Does NOT touch stdout's
/// alternate screen — safe for CI and AI-agent verification.
async fn run_tui_snapshot(
    opts: &SimOpts,
    base: &str,
    events_rx: tokio::sync::mpsc::Receiver<crate::tui::events::QuelchEvent>,
    drops: std::sync::Arc<std::sync::atomic::AtomicU64>,
) -> Result<()> {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::io::Write;

    let path = opts
        .snapshot_to
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("snapshot_to not set"))?
        .clone();
    let frames = opts.snapshot_frames.max(1);
    let mut events_rx = events_rx;

    let prefs = crate::tui::prefs::Prefs::default();
    let config = sim_config(base);
    let mut app = crate::tui::app::App::new(&config, prefs);

    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend)?;
    let start = std::time::Instant::now();
    let mut file = std::fs::File::create(&path)?;

    for frame_idx in 0..frames {
        // Drain events that arrived since last frame
        while let Ok(ev) = events_rx.try_recv() {
            app.apply(ev);
        }
        app.tick_spinner();
        app.drops = drops.load(std::sync::atomic::Ordering::Relaxed);

        terminal.draw(|f| {
            crate::tui::layout::draw(f, &app, start.elapsed(), false);
        })?;

        let buf = terminal.backend().buffer();
        writeln!(
            file,
            "===== FRAME {frame_idx} (uptime {:.2}s) =====",
            start.elapsed().as_secs_f32()
        )?;
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>();
            writeln!(file, "{line}")?;
        }
        writeln!(file)?;

        // Pace frames at roughly 500ms each so events accumulate.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Ok(())
}
```

Update `run(opts)` to branch: when `opts.snapshot_to.is_some()`, bypass the stdout-TUI path entirely. After the engine + scheduler + fault injector spawn, instead of waiting on duration OR cancel, call `run_tui_snapshot` with the event receiver that the engine's `TuiLayer` emits into.

This requires `run` to install a `TuiLayer` for snapshot mode (normally done by `main.rs`). For simplicity, add a dedicated branch in `run`:

```rust
pub async fn run(opts: SimOpts) -> Result<()> {
    // ... existing setup up through engine_handle spawn ...

    if opts.snapshot_to.is_some() {
        // Install TuiLayer directly (main.rs hasn't done this for us in snapshot mode).
        use tracing_subscriber::prelude::*;
        let (layer, rx, drops) = crate::tui::tracing_layer::layer_and_receiver();
        // Best-effort: if a global subscriber is already set, ignore the error.
        let _ = tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new("quelch=info,sim=info"))
            .with(layer)
            .try_init();

        // Run snapshot for the requested number of frames.
        let res = run_tui_snapshot(&opts, &base, rx, drops).await;

        // Shut down everything cleanly.
        cancel.cancel();
        let _ = engine_handle.await;
        scheduler_handle.abort();
        fault_handle.abort();
        mock_handle.abort();

        // Evaluate assert_docs.
        let docs = synced_doc_count(&state_path).unwrap_or(0);
        println!(
            "sim-snapshot: {} frames written to {}, {} docs synced",
            opts.snapshot_frames,
            opts.snapshot_to.as_ref().unwrap().display(),
            docs
        );
        if let Some(threshold) = opts.assert_docs
            && docs < threshold
        {
            anyhow::bail!("assert_docs failed: only {docs} < {threshold}");
        }
        return res;
    }

    // ... the existing duration/Ctrl-C/shutdown path continues unchanged ...
}
```

Make sure this branch comes AFTER the engine/scheduler/fault spawns and BEFORE the existing "7. Wait for duration OR cancel" section. The simplest structure is to insert the branch just before step 7 and `return` from it.

- [ ] **Step 3: Verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/quelch/src/cli.rs crates/quelch/src/sim/mod.rs crates/quelch/src/sim/opts.rs crates/quelch/src/main.rs
git commit -m "sim: --snapshot-to FILE captures TUI frames to text for AI verification

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 25: AI-verifiable TUI snapshot test

The real Yes-I-tested-this gate. Produces a text file an agent can grep to confirm the TUI renders according to the spec.

**Files:**
- Create: `crates/quelch/tests/tui_snapshot.rs`

- [ ] **Step 1: Write the test**

Create `crates/quelch/tests/tui_snapshot.rs`:

```rust
//! TUI snapshot verification. Runs `quelch sim --snapshot-to FILE` and asserts
//! the dumped frames contain everything the redesigned TUI is supposed to show.
//! This is the test the agent runs to claim "I verified the TUI looks right".

use assert_cmd::Command;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn tui_snapshot_contains_spec_mandated_content() {
    let dir = tempdir().unwrap();
    let snap_path = dir.path().join("snap.txt");

    let mut cmd = Command::cargo_bin("quelch").unwrap();
    cmd.arg("sim")
        .arg("--snapshot-to")
        .arg(&snap_path)
        .arg("--snapshot-frames")
        .arg("8")
        .arg("--seed")
        .arg("42")
        .arg("--rate-multiplier")
        .arg("4.0")
        .arg("--fault-rate")
        .arg("0.2")
        .timeout(Duration::from_secs(60))
        .assert()
        .success();

    let snap = std::fs::read_to_string(&snap_path).expect("snapshot file");
    assert!(!snap.is_empty(), "snapshot file empty");

    // Header: identifies the binary and shows a clear status word.
    assert!(snap.contains("quelch"), "header: quelch banner missing");

    // Sources pane: column headings (the main v0.4.0 complaint).
    for heading in ["Source", "Status", "Items", "Rate", "Last item", "Updated"] {
        assert!(
            snap.contains(heading),
            "sources heading missing: {heading}\n{snap}"
        );
    }

    // Subsource rows for the sim's configured projects/spaces.
    for expected in ["sim-jira", "sim-confluence", "QUELCH", "INFRA"] {
        assert!(snap.contains(expected), "expected subsource row: {expected}");
    }

    // Azure panel: plain-English labels (the second major v0.4.0 complaint).
    for label in [
        "Total requests",
        "Failed (4xx)",
        "Failed (5xx)",
        "Throttled",
        "Latency",
        "median",
    ] {
        assert!(snap.contains(label), "azure label missing: {label}");
    }

    // Footer: single keybinding line, no duplication (v0.4.0 shipped two).
    let footer_key_hits = snap.matches("sync now").count();
    assert!(
        footer_key_hits >= 1,
        "expected sync-now keybinding in footer"
    );
    // Each frame renders the footer once; 8 frames means 8 occurrences max.
    // If someone re-introduced duplication (two lines per frame) it would be 16+.
    assert!(
        footer_key_hits <= 10,
        "footer appears duplicated — {} occurrences, expected ≤8 (one per frame)",
        footer_key_hits
    );

    // At least one frame should show engine activity. Either `Syncing` or
    // `Ready` must appear (seeded run exercises both).
    assert!(
        snap.contains("Syncing") || snap.contains("Ready"),
        "expected Syncing or Ready state to appear in snapshot"
    );
}

#[test]
fn tui_snapshot_azure_chart_renders_something() {
    let dir = tempdir().unwrap();
    let snap_path = dir.path().join("snap.txt");

    let mut cmd = Command::cargo_bin("quelch").unwrap();
    cmd.arg("sim")
        .arg("--snapshot-to")
        .arg(&snap_path)
        .arg("--snapshot-frames")
        .arg("6")
        .arg("--seed")
        .arg("7")
        .arg("--rate-multiplier")
        .arg("6.0")
        .timeout(Duration::from_secs(60))
        .assert()
        .success();

    let snap = std::fs::read_to_string(&snap_path).unwrap();
    // Chart axis labels should appear.
    assert!(snap.contains("-60s"), "chart x-axis label missing");
    assert!(snap.contains("now"), "chart x-axis label missing");
}
```

- [ ] **Step 2: Run + verify**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --test tui_snapshot
```

Both tests must pass. If they fail, investigate — the snapshot file in `dir.path()` is preserved until the TempDir is dropped; print it before asserting during development if needed.

- [ ] **Step 3: Commit**

```bash
git add crates/quelch/tests/tui_snapshot.rs
git commit -m "Add tui_snapshot integration test — AI-verifiable TUI rendering

Confirms the redesigned TUI actually renders: column headings,
plain-English labels, chart axis labels, deduplicated footer, and
engine state transitions. Failing this test means a TUI regression.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 26: Log-mode verification (agent-runnable)

Plain-log mode's equivalent. Runs `quelch sim --no-tui`, captures stdout, asserts the summary line is correct and key tracing phases appear.

**Files:**
- Modify: `crates/quelch/tests/sim_headless.rs` (extend with log-content assertions)

- [ ] **Step 1: Extend the existing test file**

Replace the entire content of `crates/quelch/tests/sim_headless.rs` with:

```rust
//! Spawns `quelch sim --no-tui` and asserts both the CI exit-code contract
//! and that stdout contains the expected structured-log content.

use assert_cmd::Command;
use std::time::Duration;

#[test]
fn sim_runs_briefly_and_syncs_some_docs() {
    let mut cmd = Command::cargo_bin("quelch").unwrap();
    cmd.arg("sim")
        .arg("--duration")
        .arg("3s")
        .arg("--seed")
        .arg("42")
        .arg("--no-tui")
        .arg("--rate-multiplier")
        .arg("5.0")
        .arg("--assert-docs")
        .arg("5")
        .timeout(Duration::from_secs(30))
        .assert()
        .success();
}

#[test]
fn log_mode_stdout_contains_summary_and_key_phases() {
    let output = Command::cargo_bin("quelch")
        .unwrap()
        .arg("sim")
        .arg("--duration")
        .arg("5s")
        .arg("--seed")
        .arg("42")
        .arg("--no-tui")
        .arg("--rate-multiplier")
        .arg("4.0")
        .arg("-v")   // bump to quelch=info,sim=debug so phases appear
        .timeout(Duration::from_secs(30))
        .output()
        .expect("run quelch sim");

    assert!(
        output.status.success(),
        "sim failed: status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");

    // The summary line is emitted to stdout at the end.
    assert!(
        stdout.contains("docs synced"),
        "expected summary line with 'docs synced' in stdout:\n{stdout}"
    );

    // Structured tracing events should appear in stderr (default fmt output
    // goes to stderr in tracing-subscriber 0.3).
    for phase in [
        "cycle_started",
        "source_started",
        "subsource_started",
    ] {
        assert!(
            combined.contains(phase),
            "expected phase '{phase}' in log output:\n{combined}"
        );
    }
}
```

- [ ] **Step 2: Run + verify + commit**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --test sim_headless
git add crates/quelch/tests/sim_headless.rs
git commit -m "Log-mode AI verification: assert summary line and key phases

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 27: Final AI self-verification run

The controller (you, the executing agent) personally runs both modes end-to-end and reports the snapshot contents back to confirm the redesign is real.

**Files:**
- None — verification only; no commits.

- [ ] **Step 1: TUI verification**

```bash
mkdir -p /tmp/quelch-verify
cargo run --release -p quelch -- sim \
  --snapshot-to /tmp/quelch-verify/tui.txt \
  --snapshot-frames 10 \
  --seed 42 \
  --rate-multiplier 4.0 \
  --fault-rate 0.2 \
  --assert-docs 10
```

Expected exit code 0. Then `Read` (via the Read tool) `/tmp/quelch-verify/tui.txt` and manually confirm:
- Header line on every frame.
- `Source`, `Status`, `Items`, `Rate`, `Last item`, `Updated` column headings present.
- Both `sim-jira` and `sim-confluence` appear.
- At least one frame shows `Syncing` status and ANY frame shows something non-zero in the chart row.
- Footer has ONE keybinding line (not duplicated).
- Azure panel has `Total requests`, `Latency`, `median`, `Failed (4xx)`, `Failed (5xx)`, `Throttled`.

- [ ] **Step 2: Log-mode verification**

```bash
cargo run --release -p quelch -- sim \
  --duration 5s \
  --seed 42 \
  --no-tui \
  --rate-multiplier 4.0 \
  -v 2>/tmp/quelch-verify/log.stderr \
  >/tmp/quelch-verify/log.stdout
echo "exit=$?"
```

Expected exit 0. Then `Read` `/tmp/quelch-verify/log.stdout` and `/tmp/quelch-verify/log.stderr`. Confirm:
- stdout ends with `sim: N.Ns, N docs synced` where N ≥ 10.
- stderr contains `phase="cycle_started"`, `phase="source_started"`, `phase="subsource_started"` (or similar phase lines depending on `fmt` formatter style).

- [ ] **Step 3: Report back**

The agent executing Task 27 should respond with a structured report: exit codes, summary counts, and whether each expected content element was found. Only after both sections pass may the agent tell the user "Yes, I tested both modes myself."

---

## Self-Review

**Spec coverage:**
- §2.1 module layout → Tasks 2, 5, 7, 8, 10, 15 cover every new module/widget.
- §3 simulator (SimOpts, starter corpus, scheduler, mutations, SimEmbedder, faults, CLI, log filter, shutdown) → Tasks 13–21.
- §4 TUI redesign (frame, header, source table, Azure chart, spinner, drilldown, help, log view, input, latency fix) → Tasks 1–12.
- §5 data plumbing (engine emissions + TuiLayer mapping + Prefs additions + recent_docs + chart_points) → Tasks 1–4.
- §6 CI → Task 22.
- §7 testability → test cases embedded in each task + Task 21 integration + Task 24 manual.
- §8 files touched — each file appears in at least one task's Files list.
- §9 rollout — release notes mention in Task 24; no breaking changes.

**Placeholder scan:** no "TBD/TODO/similar to Task N/etc." — every step shows actual code or commands. A few inline notes ("adjust indentation if different") are legitimate engineer-judgment calls and not placeholders.

**Type consistency:**
- `SimOpts` fields consistent between Task 15 (opts.rs) and Task 20 (run).
- `Spinner::tick`/`glyph` match between Task 2 (spinner.rs) and Task 3 (App usage).
- `RecentDoc { ts, id }` consistent across Tasks 3, 7, 11.
- `FieldVisitor` field additions in Task 1 match the phase constants emitted in Task 1.
- `AzurePanelWidget.backoff_reason: Option<&str>` matches layout.rs caller.
- `draw(f, &app, uptime, help_open)` signature is consistent across Tasks 10, 12, and the smoke test.

**Known gap:** `cycle` parameter on `run_sync_with` was added in the previous feature's Task 7 fix (commit 4fb25db). Task 20's `run_engine_loop` passes `cycle` — correct. No action needed.

Plan looks solid.
