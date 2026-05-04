//! App state for the fleet-dashboard TUI.
//!
//! State is derived from polling `cosmos::meta::list_all`; there is no
//! tracing-event pipeline. The TUI is read-only.

use chrono::{DateTime, Utc};

use crate::cosmos::meta::{Cursor, CursorKey};

/// The complete mutable state the TUI needs across frames.
pub struct App {
    /// Latest cursor rows from the most recent poll.
    pub rows: Vec<(CursorKey, Cursor)>,
    /// Wall-clock time of the last successful (or attempted) poll.
    pub last_poll_at: Option<DateTime<Utc>>,
    /// Error message from the most recent poll failure, if any.
    pub last_poll_error: Option<String>,
    /// Row index currently highlighted (0-based, clamped to `rows.len()`).
    pub selected_index: usize,
    /// Whether the help overlay is currently visible.
    pub help_visible: bool,
}

impl App {
    /// Create an empty app waiting for the first poll.
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            last_poll_at: None,
            last_poll_error: None,
            selected_index: 0,
            help_visible: false,
        }
    }

    /// Apply a poll result to the app state.
    ///
    /// On success the row list is replaced and the error is cleared.
    /// On failure the stale row list is kept so the screen stays populated.
    pub fn handle_poll_result(&mut self, result: Result<Vec<(CursorKey, Cursor)>, String>) {
        self.last_poll_at = Some(Utc::now());
        match result {
            Ok(rows) => {
                self.rows = rows;
                self.last_poll_error = None;
                self.ensure_valid_selection();
            }
            Err(e) => {
                self.last_poll_error = Some(e);
            }
        }
    }

    /// Move the selected row by `delta` (positive = down, negative = up).
    pub fn move_selection(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let len = self.rows.len() as i32;
        let next = (self.selected_index as i32 + delta).rem_euclid(len);
        self.selected_index = next as usize;
    }

    /// Toggle help overlay visibility.
    pub fn toggle_help(&mut self) {
        self.help_visible = !self.help_visible;
    }

    /// Return the currently selected `(CursorKey, Cursor)` row, if any.
    pub fn selected_row(&self) -> Option<&(CursorKey, Cursor)> {
        self.rows.get(self.selected_index)
    }

    fn ensure_valid_selection(&mut self) {
        if self.rows.is_empty() {
            self.selected_index = 0;
        } else {
            self.selected_index = self.selected_index.min(self.rows.len() - 1);
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosmos::meta::CursorKey;

    fn key(deployment: &str, source: &str, subsource: &str) -> CursorKey {
        CursorKey {
            deployment_name: deployment.to_string(),
            source_name: source.to_string(),
            subsource: subsource.to_string(),
        }
    }

    fn two_rows() -> Vec<(CursorKey, Cursor)> {
        vec![
            (key("prod", "jira-cloud", "DO"), Cursor::default()),
            (key("prod", "jira-cloud", "INT"), Cursor::default()),
        ]
    }

    #[test]
    fn initial_state_is_empty() {
        let app = App::new();
        assert!(app.rows.is_empty());
        assert!(app.last_poll_at.is_none());
        assert!(app.last_poll_error.is_none());
        assert_eq!(app.selected_index, 0);
        assert!(!app.help_visible);
    }

    #[test]
    fn handle_poll_result_success_replaces_rows() {
        let mut app = App::new();
        app.handle_poll_result(Ok(two_rows()));
        assert_eq!(app.rows.len(), 2);
        assert!(app.last_poll_at.is_some());
        assert!(app.last_poll_error.is_none());
    }

    #[test]
    fn handle_poll_result_error_preserves_stale_rows() {
        let mut app = App::new();
        app.handle_poll_result(Ok(two_rows()));
        app.handle_poll_result(Err("network timeout".to_string()));
        // Rows should still be there from the previous successful poll.
        assert_eq!(app.rows.len(), 2);
        assert!(app.last_poll_error.is_some());
    }

    #[test]
    fn move_selection_wraps_around() {
        let mut app = App::new();
        app.handle_poll_result(Ok(two_rows()));
        assert_eq!(app.selected_index, 0);
        app.move_selection(1);
        assert_eq!(app.selected_index, 1);
        // Wrap down → back to 0
        app.move_selection(1);
        assert_eq!(app.selected_index, 0);
        // Wrap up → back to 1
        app.move_selection(-1);
        assert_eq!(app.selected_index, 1);
    }

    #[test]
    fn move_selection_noop_on_empty() {
        let mut app = App::new();
        app.move_selection(1);
        assert_eq!(app.selected_index, 0);
    }

    #[test]
    fn toggle_help_flips() {
        let mut app = App::new();
        assert!(!app.help_visible);
        app.toggle_help();
        assert!(app.help_visible);
        app.toggle_help();
        assert!(!app.help_visible);
    }

    #[test]
    fn selected_row_returns_correct_entry() {
        let mut app = App::new();
        app.handle_poll_result(Ok(two_rows()));
        app.move_selection(1);
        let (k, _) = app.selected_row().unwrap();
        assert_eq!(k.subsource, "INT");
    }

    #[test]
    fn selected_row_none_when_empty() {
        let app = App::new();
        assert!(app.selected_row().is_none());
    }

    #[test]
    fn selection_clamped_after_poll_shrinks_list() {
        let mut app = App::new();
        app.handle_poll_result(Ok(two_rows()));
        app.selected_index = 1;
        // Next poll returns only one row.
        app.handle_poll_result(Ok(vec![(key("prod", "jira", "DO"), Cursor::default())]));
        assert_eq!(app.selected_index, 0);
    }
}
