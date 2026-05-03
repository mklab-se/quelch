//! Fleet-dashboard TUI for `quelch status --tui`.
//!
//! Polls `cosmos::meta::list_all` on a timer and renders the results as a
//! read-only table. No tracing-event pipeline — state comes from Cosmos.

pub mod app;
pub mod input;
pub mod layout;
pub mod widgets;

use std::io;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use crate::cosmos::CosmosBackend;
use crate::cosmos::meta;

use self::app::App;
use self::input::{InputOutcome, on_key};

// ---------------------------------------------------------------------------
// Terminal guard
// ---------------------------------------------------------------------------

/// Restores the terminal to its original state when dropped — even on panic.
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

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Run the fleet dashboard until the user quits.
///
/// 1. Sets up the terminal (alternate screen, raw mode).
/// 2. Spawns a background task that polls `cosmos::meta::list_all` every
///    `refresh_interval` and pushes the result via a channel.
/// 3. Main loop: redraws on each timer tick; processes key events.
/// 4. Restores the terminal on exit (even on panic, via [`TerminalGuard`]).
pub async fn run_status_dashboard(
    cosmos: Arc<dyn CosmosBackend>,
    meta_container: String,
    refresh_interval: Duration,
) -> Result<()> {
    let mut app = App::new();

    // Channel for poll results: `Ok(rows)` or `Err(message)`.
    let (poll_tx, mut poll_rx) = mpsc::channel::<Result<Vec<_>, String>>(4);

    // Spawn the background poller.
    let cosmos_clone = cosmos.clone();
    let container = meta_container.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(refresh_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let result = meta::list_all(cosmos_clone.as_ref(), &container)
                .await
                .map_err(|e| e.to_string());
            if poll_tx.send(result).await.is_err() {
                break; // receiver dropped — TUI exited
            }
        }
    });

    // Set up the terminal.
    let _guard = TerminalGuard::new()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    // Redraw timer at ~10 Hz (poll updates are on a slower interval).
    let mut redraw_interval = tokio::time::interval(Duration::from_millis(100));
    redraw_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut key_events = EventStream::new();

    loop {
        tokio::select! {
            // Redraw tick.
            _ = redraw_interval.tick() => {
                // Drain any pending poll results (take the last one if multiple arrived).
                while let Ok(result) = poll_rx.try_recv() {
                    app.handle_poll_result(result);
                }
                terminal.draw(|f| {
                    layout::draw(f, &app);
                })?;
            }

            // Keyboard input.
            Some(Ok(ev)) = key_events.next() => {
                if let Event::Key(key) = ev
                    && key.kind == KeyEventKind::Press
                {
                    if on_key(key, &mut app) == InputOutcome::Quit {
                        return Ok(());
                    }
                    // Immediate redraw after key event for responsiveness.
                    terminal.draw(|f| layout::draw(f, &app))?;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_guard_constructs_and_drops_safely() {
        // We cannot enter raw mode in a test harness, but the struct's
        // Drop should run without panicking even in the un-initialised case.
        let g = TerminalGuard { restored: false };
        drop(g);
    }
}
