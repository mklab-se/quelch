//! Terminal user interface for `quelch watch` / `quelch sync`.

pub mod app;
pub mod events;
pub mod input;
pub mod layout;
pub mod metrics;
pub mod prefs;
pub mod tracing_layer;
pub mod widgets;

use anyhow::Result;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
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
use self::layout::{LayoutOptions, draw};
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
    let prefs = Prefs::load(&prefs_path)?;
    let mut app = App::new(&config, prefs);

    let _guard = TerminalGuard::new()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut input_state = InputState::default();

    let mut interval = tokio::time::interval(Duration::from_millis(200));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                while let Ok(ev) = events_rx.try_recv() {
                    app.apply(ev);
                }
                app.drops = drops_counter.load(Ordering::Relaxed);
                let focused_source = app.sources.first().map(|s| s.name.clone());
                let focused_sub = app
                    .sources
                    .first()
                    .and_then(|s| s.subsources.first().map(|x| x.key.clone()));
                terminal.draw(|f| {
                    draw(
                        f,
                        &app,
                        LayoutOptions {
                            focused_source: focused_source.as_deref(),
                            focused_subsource: focused_sub.as_deref(),
                        },
                    );
                })?;
                if event::poll(Duration::from_millis(0))?
                    && let Event::Key(key) = event::read()?
                    && key.kind == KeyEventKind::Press
                {
                    match input_state.on_key(
                        key,
                        &mut app,
                        focused_source.as_deref(),
                        focused_sub.as_deref(),
                    ) {
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

    #[test]
    fn layout_draw_on_test_backend_does_not_panic() {
        use crate::config::{
            AuthConfig, AzureConfig, Config, JiraSourceConfig, SourceConfig, SyncConfig,
        };
        use crate::tui::app::App;
        use crate::tui::prefs::Prefs;

        let cfg = Config {
            azure: AzureConfig {
                endpoint: "x".into(),
                api_key: "k".into(),
            },
            sources: vec![SourceConfig::Jira(JiraSourceConfig {
                name: "j".into(),
                url: "x".into(),
                auth: AuthConfig::DataCenter { pat: "p".into() },
                projects: vec!["DO".into(), "HR".into()],
                index: "i".into(),
            })],
            sync: SyncConfig::default(),
        };
        let app = App::new(&cfg, Prefs::default());
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| {
            draw(
                f,
                &app,
                LayoutOptions {
                    focused_source: Some("j"),
                    focused_subsource: Some("DO"),
                },
            );
        })
        .unwrap();
    }
}
