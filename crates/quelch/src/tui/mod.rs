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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
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
use self::layout::draw;
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

struct InputReader {
    rx: mpsc::UnboundedReceiver<KeyEvent>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl InputReader {
    fn spawn() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();

        let handle = thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                match event::poll(Duration::from_millis(100)) {
                    Ok(true) => match event::read() {
                        Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                            if tx.send(key).is_err() {
                                break;
                            }
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    },
                    Ok(false) => {}
                    Err(_) => break,
                }
            }
        });

        Self {
            rx,
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for InputReader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
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
    terminal.clear()?;
    let mut input_state = InputState::default();
    let mut input_reader = InputReader::spawn();

    let mut frame_clock = tokio::time::interval(Duration::from_millis(125));
    frame_clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    app.drops = drops_counter.load(Ordering::Relaxed);
    terminal.draw(|f| draw(f, &app))?;

    loop {
        tokio::select! {
            _ = frame_clock.tick() => {
                app.drops = drops_counter.load(Ordering::Relaxed);
                terminal.draw(|f| draw(f, &app))?;
            }
            Some(ev) = events_rx.recv() => {
                app.apply(ev);
                while let Ok(next) = events_rx.try_recv() {
                    app.apply(next);
                }
                app.drops = drops_counter.load(Ordering::Relaxed);
                terminal.draw(|f| draw(f, &app))?;
            }
            Some(key) = input_reader.rx.recv() => {
                match input_state.on_key(key, &mut app) {
                    InputOutcome::Quit => {
                        app.status = app::EngineStatus::Shutdown;
                        app.footer = "Shutting down after the current batch boundary".into();
                        let _ = cmd_tx.send(UiCommand::Shutdown).await;
                        app.prefs.save(&prefs_path).ok();
                        return Ok(());
                    }
                    InputOutcome::Command(cmd) => {
                        let _ = cmd_tx.send(cmd).await;
                    }
                    InputOutcome::None => {}
                }
                app.drops = drops_counter.load(Ordering::Relaxed);
                terminal.draw(|f| draw(f, &app))?;
            }
            else => {
                app.prefs.save(&prefs_path).ok();
                return Ok(());
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
            draw(f, &app);
        })
        .unwrap();
    }
}
