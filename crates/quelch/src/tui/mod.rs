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

/// Restores the terminal on drop — even if a panic unwinds through run().
pub struct TerminalGuard {
    restored: bool,
}

impl TerminalGuard {
    pub fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        Ok(Self { restored: false })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if !self.restored {
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            let _ = disable_raw_mode();
            self.restored = true;
        }
    }
}

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
                app.tick_throughputs(std::time::Instant::now());
                app.drops = drops_counter.load(Ordering::Relaxed);
                terminal.draw(|f| {
                    crate::tui::layout::draw(f, &app, start.elapsed(), input_state.help_open());
                })?;
            }
            Some(Ok(ev)) = key_events.next() => {
                if let Event::Key(key) = ev
                    && key.kind == KeyEventKind::Press
                {
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

#[cfg(test)]
mod smoke_tests {
    use super::*;
    use ratatui::backend::TestBackend;

    #[test]
    fn terminal_guard_constructs_and_drops() {
        // We can't enter raw mode in a unit test harness, but the struct
        // can be constructed directly and Drop should run safely.
        let g = TerminalGuard { restored: false };
        drop(g);
    }

    fn minimal_v2_config(source_name: &str, projects: Vec<&str>) -> crate::config::Config {
        // TODO(quelch v2 phase 3+): move to a shared test fixture builder
        use crate::config::{
            AuthConfig, AzureConfig, Config, CosmosConfig, JiraSourceConfig, OpenAiConfig,
            SourceConfig,
        };
        Config {
            azure: AzureConfig {
                subscription_id: "sub".into(),
                resource_group: "rg".into(),
                region: "swedencentral".into(),
                naming: Default::default(),
                skip_role_assignments: false,
            },
            cosmos: CosmosConfig::default(),
            search: Default::default(),
            openai: OpenAiConfig {
                endpoint: "https://x.openai.azure.com".into(),
                embedding_deployment: "te".into(),
                embedding_dimensions: 1536,
            },
            sources: vec![SourceConfig::Jira(JiraSourceConfig {
                name: source_name.into(),
                url: "http://x".into(),
                auth: AuthConfig::DataCenter { pat: "p".into() },
                projects: projects.into_iter().map(String::from).collect(),
                container: None,
                companion_containers: Default::default(),
                fields: Default::default(),
            })],
            ingest: Default::default(),
            deployments: vec![],
            mcp: Default::default(),
            rigg: Default::default(),
            state: Default::default(),
        }
    }

    #[test]
    fn layout_draw_on_test_backend_does_not_panic() {
        use crate::tui::app::App;
        use crate::tui::layout::draw;
        use crate::tui::prefs::Prefs;

        let cfg = minimal_v2_config("j", vec!["DO", "HR"]);
        let app = App::new(&cfg, Prefs::default());
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| {
            draw(f, &app, std::time::Duration::from_secs(0), false);
        })
        .unwrap();
    }

    #[test]
    fn draw_accepts_uptime_and_help_open_flag() {
        use crate::tui::app::App;
        use crate::tui::layout::draw;
        use crate::tui::prefs::Prefs;

        let cfg = minimal_v2_config("j", vec!["DO"]);
        let app = App::new(&cfg, Prefs::default());
        let mut term = ratatui::Terminal::new(TestBackend::new(100, 26)).unwrap();
        term.draw(|f| draw(f, &app, std::time::Duration::from_secs(7), true))
            .unwrap();
    }
}
