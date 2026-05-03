//! Keyboard input handling for the v2 read-only fleet dashboard.
//!
//! The only actions are navigation (↑/↓), quit (q/Esc/Ctrl-C), and help (?).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::App;

/// Outcome of processing a single key event.
#[derive(Debug, PartialEq, Eq)]
pub enum InputOutcome {
    /// Nothing happened.
    None,
    /// User requested quit.
    Quit,
}

/// Process a key event and mutate `app` as needed.
pub fn on_key(key: KeyEvent, app: &mut App) -> InputOutcome {
    match key.code {
        KeyCode::Char('q') => return InputOutcome::Quit,
        KeyCode::Esc => {
            if app.help_visible {
                app.help_visible = false;
            } else {
                return InputOutcome::Quit;
            }
        }
        KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
            return InputOutcome::Quit;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.move_selection(1);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.move_selection(-1);
        }
        KeyCode::Char('?') => {
            app.toggle_help();
        }
        _ => {}
    }
    InputOutcome::None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosmos::meta::{Cursor, CursorKey};

    fn key_evt(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_c() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
    }

    fn two_row_app() -> App {
        let mut app = App::new();
        app.handle_poll_result(Ok(vec![
            (
                CursorKey {
                    deployment_name: "p".into(),
                    source_name: "j".into(),
                    subsource: "DO".into(),
                },
                Cursor::default(),
            ),
            (
                CursorKey {
                    deployment_name: "p".into(),
                    source_name: "j".into(),
                    subsource: "INT".into(),
                },
                Cursor::default(),
            ),
        ]));
        app
    }

    #[test]
    fn q_quits() {
        let mut app = App::new();
        assert_eq!(
            on_key(key_evt(KeyCode::Char('q')), &mut app),
            InputOutcome::Quit
        );
    }

    #[test]
    fn esc_quits_when_help_closed() {
        let mut app = App::new();
        assert_eq!(on_key(key_evt(KeyCode::Esc), &mut app), InputOutcome::Quit);
    }

    #[test]
    fn esc_closes_help_when_open() {
        let mut app = App::new();
        app.help_visible = true;
        assert_eq!(on_key(key_evt(KeyCode::Esc), &mut app), InputOutcome::None);
        assert!(!app.help_visible);
    }

    #[test]
    fn ctrl_c_quits() {
        let mut app = App::new();
        assert_eq!(on_key(ctrl_c(), &mut app), InputOutcome::Quit);
    }

    #[test]
    fn down_moves_selection() {
        let mut app = two_row_app();
        assert_eq!(app.selected_index, 0);
        on_key(key_evt(KeyCode::Down), &mut app);
        assert_eq!(app.selected_index, 1);
    }

    #[test]
    fn up_moves_selection() {
        let mut app = two_row_app();
        app.selected_index = 1;
        on_key(key_evt(KeyCode::Up), &mut app);
        assert_eq!(app.selected_index, 0);
    }

    #[test]
    fn j_and_k_navigate() {
        let mut app = two_row_app();
        on_key(key_evt(KeyCode::Char('j')), &mut app);
        assert_eq!(app.selected_index, 1);
        on_key(key_evt(KeyCode::Char('k')), &mut app);
        assert_eq!(app.selected_index, 0);
    }

    #[test]
    fn question_mark_toggles_help() {
        let mut app = App::new();
        assert!(!app.help_visible);
        on_key(key_evt(KeyCode::Char('?')), &mut app);
        assert!(app.help_visible);
        on_key(key_evt(KeyCode::Char('?')), &mut app);
        assert!(!app.help_visible);
    }
}
