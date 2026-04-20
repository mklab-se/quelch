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

pub const MOCK_PAT: &str = "mock-pat-token";

/// Runs the simulator until `opts.duration` elapses or Ctrl-C is pressed.
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
        let _ = scheduler::run(
            scheduler_base,
            scheduler_seed,
            scheduler_rate,
            scheduler_cancel,
        )
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
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<UiCommand>(16);
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
            cmd_rx,
            engine_cancel,
        )
        .await;
    });

    // Snapshot mode: render the TUI to a TestBackend and dump frames to disk.
    if let Some(snapshot_path) = opts.snapshot_to.clone() {
        use tracing_subscriber::prelude::*;
        let (layer, rx, drops) = crate::tui::tracing_layer::layer_and_receiver();
        let _ = tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new("quelch=info,sim=info"))
            .with(layer)
            .try_init();

        let res = run_tui_snapshot(&opts, &base, rx, drops).await;

        let _ = cmd_tx.send(UiCommand::Shutdown).await;
        cancel.cancel();
        let _ = engine_handle.await;
        scheduler_handle.abort();
        fault_handle.abort();
        mock_handle.abort();

        let docs = synced_doc_count(&state_path).unwrap_or(0);
        println!(
            "sim-snapshot: {} frames written to {}, {} docs synced",
            opts.snapshot_frames,
            snapshot_path.display(),
            docs
        );
        if let Some(threshold) = opts.assert_docs
            && docs < threshold
        {
            anyhow::bail!("assert_docs failed: only {docs} < {threshold}");
        }
        return res;
    }

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

async fn run_engine_loop(
    config: Config,
    state_path: PathBuf,
    embedding: EmbeddingConfig,
    embedder: SimEmbedder,
    mut cmd_rx: tokio::sync::mpsc::Receiver<UiCommand>,
    cancel: CancellationToken,
) -> Result<()> {
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
            &mut cmd_rx,
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
                auth: AuthConfig::DataCenter {
                    pat: MOCK_PAT.into(),
                },
                projects: vec!["QUELCH".into(), "DEMO".into()],
                index: "sim-jira-issues".into(),
            }),
            SourceConfig::Confluence(ConfluenceSourceConfig {
                name: "sim-confluence".into(),
                url: format!("{base}/confluence"),
                auth: AuthConfig::DataCenter {
                    pat: MOCK_PAT.into(),
                },
                spaces: vec!["QUELCH".into(), "INFRA".into()],
                index: "sim-confluence-pages".into(),
            }),
        ],
        sync: SyncConfig::default(),
    }
}

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

    let backend = TestBackend::new(opts.snapshot_width, opts.snapshot_height);
    let mut terminal = Terminal::new(backend)?;
    let start = std::time::Instant::now();
    let mut file = std::fs::File::create(&path)?;

    for frame_idx in 0..frames {
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

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Ok(())
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
            snapshot_to: None,
            snapshot_frames: 10,
            snapshot_width: 120,
            snapshot_height: 40,
        };
        run(opts).await.unwrap();
    }
}
