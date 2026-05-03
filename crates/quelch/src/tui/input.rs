use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};

use crate::sync::UiCommand;

use super::app::{App, EngineStatus};

#[derive(Default)]
pub struct InputState {
    /// Tracks the last timestamp an ALL-CAPS action (R, P) was pressed, to
    /// implement "press again within 2s to confirm".
    pub pending_confirm: Option<(char, Instant)>,
    help_open: bool,
}

#[derive(Debug)]
pub enum InputOutcome {
    None,
    Command(UiCommand),
    Quit,
}

impl InputState {
    pub fn help_open(&self) -> bool {
        self.help_open
    }

    pub fn on_key(&mut self, key: KeyEvent, app: &mut App) -> InputOutcome {
        let is_shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Char('q') => return InputOutcome::Quit,
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                return InputOutcome::Quit;
            }
            KeyCode::Down => {
                app.move_selection_down();
            }
            KeyCode::Up => {
                app.move_selection_up();
            }
            KeyCode::Left => {
                app.move_selection_left();
            }
            KeyCode::Right => {
                app.move_selection_right();
            }
            KeyCode::Char(' ') => {
                app.toggle_selected_collapsed();
            }
            KeyCode::Enter => {
                if app.focused_subsource_name().is_some() {
                    app.toggle_drilldown();
                } else {
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
            KeyCode::Char('s') => {
                app.prefs.log_view_on = !app.prefs.log_view_on;
            }
            KeyCode::Char('p') => match app.status {
                EngineStatus::Paused => {
                    app.status = EngineStatus::Idle;
                    return InputOutcome::Command(UiCommand::Resume);
                }
                _ => {
                    app.status = EngineStatus::Paused;
                    return InputOutcome::Command(UiCommand::Pause);
                }
            },
            KeyCode::Char('r') if !is_shift => {
                return InputOutcome::Command(UiCommand::SyncNow);
            }
            KeyCode::Char('R') => {
                if self.armed('R') {
                    if let Some(src) = app.focused_source_name() {
                        return InputOutcome::Command(UiCommand::ResetCursor {
                            source: src.to_string(),
                            subsource: app.focused_subsource_name().map(str::to_string),
                        });
                    }
                } else {
                    self.arm('R');
                    app.footer = "press R again within 2s to reset".into();
                }
            }
            KeyCode::Char('P') => {
                if self.armed('P') {
                    if let Some(src) = app.focused_source_name() {
                        return InputOutcome::Command(UiCommand::PurgeNow {
                            source: src.to_string(),
                        });
                    }
                } else {
                    self.arm('P');
                    app.footer = "press P again within 2s to purge".into();
                }
            }
            KeyCode::Char('c') => {
                app.footer.clear();
            }
            _ => {}
        }
        self.prune_expired();
        InputOutcome::None
    }

    fn arm(&mut self, key: char) {
        self.pending_confirm = Some((key, Instant::now()));
    }

    fn armed(&mut self, key: char) -> bool {
        let now = Instant::now();
        match self.pending_confirm {
            Some((k, t)) if k == key && now.duration_since(t) <= Duration::from_secs(2) => {
                self.pending_confirm = None;
                true
            }
            _ => false,
        }
    }

    fn prune_expired(&mut self) {
        if let Some((_, t)) = self.pending_confirm
            && Instant::now().duration_since(t) > Duration::from_secs(2)
        {
            self.pending_confirm = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        AuthConfig, AzureConfig, Config, CosmosConfig, JiraSourceConfig, OpenAiConfig, SourceConfig,
    };
    use crate::tui::app::App;
    use crate::tui::prefs::Prefs;

    fn make_app() -> App {
        // TODO(quelch v2 phase 3+): move to a shared test fixture builder
        let cfg = Config {
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
                name: "j".into(),
                url: "x".into(),
                auth: AuthConfig::DataCenter { pat: "p".into() },
                projects: vec!["DO".into()],
                container: None,
                companion_containers: Default::default(),
                fields: Default::default(),
            })],
            ingest: Default::default(),
            deployments: vec![],
            mcp: Default::default(),
            rigg: Default::default(),
            state: Default::default(),
        };
        App::new(&cfg, Prefs::default())
    }

    #[test]
    fn space_toggles_source_collapsed() {
        let mut state = InputState::default();
        let mut app = make_app();
        let key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        state.on_key(key, &mut app);
        assert!(app.prefs.is_source_collapsed("j"));
        state.on_key(key, &mut app);
        assert!(!app.prefs.is_source_collapsed("j"));
    }

    #[test]
    fn s_toggles_log_view() {
        let mut state = InputState::default();
        let mut app = make_app();
        let key = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE);
        state.on_key(key, &mut app);
        assert!(app.prefs.log_view_on);
    }

    #[test]
    fn shift_r_requires_second_press() {
        let mut state = InputState::default();
        let mut app = make_app();
        let shift_r = KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT);
        match state.on_key(shift_r, &mut app) {
            InputOutcome::None => {}
            other => panic!("expected arm first, got {other:?}"),
        }
        match state.on_key(shift_r, &mut app) {
            InputOutcome::Command(UiCommand::ResetCursor { source, .. }) => {
                assert_eq!(source, "j");
            }
            other => panic!("expected ResetCursor, got {other:?}"),
        }
    }

    #[test]
    fn arrows_move_selection() {
        let mut state = InputState::default();
        let mut app = make_app();

        state.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &mut app);
        assert_eq!(app.focused_subsource_name(), Some("DO"));

        state.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), &mut app);
        assert_eq!(app.focused_subsource_name(), None);
    }

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
        state.on_key(
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
            &mut app,
        );
        assert!(state.help_open());
        state.on_key(
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
            &mut app,
        );
        assert!(!state.help_open());
    }

    #[test]
    fn esc_closes_help_overlay() {
        let mut state = InputState::default();
        let mut app = make_app();
        state.on_key(
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
            &mut app,
        );
        assert!(state.help_open());
        state.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &mut app);
        assert!(!state.help_open());
    }
}
