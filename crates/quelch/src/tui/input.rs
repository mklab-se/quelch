use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};

use crate::sync::UiCommand;

use super::app::{App, EngineStatus, Focus};

#[derive(Default)]
pub struct InputState {
    /// Tracks the last timestamp an ALL-CAPS action (R, P) was pressed, to
    /// implement "press again within 2s to confirm".
    pub pending_confirm: Option<(char, Instant)>,
}

#[derive(Debug)]
pub enum InputOutcome {
    None,
    Command(UiCommand),
    Quit,
}

impl InputState {
    pub fn on_key(
        &mut self,
        key: KeyEvent,
        app: &mut App,
        focused_source: Option<&str>,
        focused_sub: Option<&str>,
    ) -> InputOutcome {
        let is_shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Char('q') => return InputOutcome::Quit,
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                return InputOutcome::Quit;
            }
            KeyCode::Tab => {
                app.focus = match app.focus {
                    Focus::Sources => Focus::Azure,
                    Focus::Azure => Focus::Sources,
                };
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                if let Some(src) = focused_source {
                    match focused_sub {
                        Some(sub) => app.prefs.toggle_subsource_collapsed(src, sub),
                        None => app.prefs.toggle_source_collapsed(src),
                    }
                }
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
                    if let Some(src) = focused_source {
                        return InputOutcome::Command(UiCommand::ResetCursor {
                            source: src.to_string(),
                            subsource: focused_sub.map(str::to_string),
                        });
                    }
                } else {
                    self.arm('R');
                    app.footer = "press R again within 2s to reset".into();
                }
            }
            KeyCode::Char('P') => {
                if self.armed('P') {
                    if let Some(src) = focused_source {
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
        AuthConfig, AzureConfig, Config, JiraSourceConfig, SourceConfig, SyncConfig,
    };
    use crate::tui::app::App;
    use crate::tui::prefs::Prefs;

    fn make_app() -> App {
        let cfg = Config {
            azure: AzureConfig {
                endpoint: "x".into(),
                api_key: "k".into(),
            },
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
    fn space_toggles_source_collapsed() {
        let mut state = InputState::default();
        let mut app = make_app();
        let key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        state.on_key(key, &mut app, Some("j"), None);
        assert!(app.prefs.is_source_collapsed("j"));
        state.on_key(key, &mut app, Some("j"), None);
        assert!(!app.prefs.is_source_collapsed("j"));
    }

    #[test]
    fn s_toggles_log_view() {
        let mut state = InputState::default();
        let mut app = make_app();
        let key = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE);
        state.on_key(key, &mut app, Some("j"), None);
        assert!(app.prefs.log_view_on);
    }

    #[test]
    fn shift_r_requires_second_press() {
        let mut state = InputState::default();
        let mut app = make_app();
        let shift_r = KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT);
        match state.on_key(shift_r, &mut app, Some("j"), None) {
            InputOutcome::None => {}
            other => panic!("expected arm first, got {other:?}"),
        }
        match state.on_key(shift_r, &mut app, Some("j"), None) {
            InputOutcome::Command(UiCommand::ResetCursor { source, .. }) => {
                assert_eq!(source, "j");
            }
            other => panic!("expected ResetCursor, got {other:?}"),
        }
    }
}
