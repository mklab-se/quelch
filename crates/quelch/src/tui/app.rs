//! Live app state for the TUI.

use std::collections::VecDeque;
use std::time::Instant;

use chrono::{DateTime, Utc};
use tracing::Level;

use super::events::QuelchEvent;
use super::metrics::{AzurePanel, Throughput};
use super::prefs::Prefs;
use crate::config::{Config, SourceConfig};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineStatus {
    Idle,
    Syncing { cycle: u64, since: DateTime<Utc> },
    Paused,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceState {
    Idle,
    Syncing,
    Error(String),
    Backoff { until: DateTime<Utc> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubsourceState {
    Idle,
    Syncing,
    Error(String),
}

pub struct SourceView {
    pub name: String,
    pub kind: String,
    pub state: SourceState,
    pub subsources: Vec<SubsourceView>,
}

pub struct SubsourceView {
    pub key: String,
    pub state: SubsourceState,
    pub last_cursor: Option<DateTime<Utc>>,
    pub last_sample_id: Option<String>,
    pub docs_synced_total: u64,
    pub last_errors: VecDeque<String>,
    pub throughput: Throughput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Sources,
    Azure,
}

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
}

pub struct LogLine {
    pub ts: DateTime<Utc>,
    pub level: Level,
    pub target: String,
    pub message: String,
}

const LOG_CAP: usize = 500;
const LAST_ERRORS_CAP: usize = 3;

impl App {
    pub fn new(config: &Config, prefs: Prefs) -> Self {
        let sources = config
            .sources
            .iter()
            .map(|sc| {
                let (kind, subs) = match sc {
                    SourceConfig::Jira(j) => ("jira".to_string(), j.projects.clone()),
                    SourceConfig::Confluence(c) => ("confluence".to_string(), c.spaces.clone()),
                };
                SourceView {
                    name: sc.name().to_string(),
                    kind,
                    state: SourceState::Idle,
                    subsources: subs
                        .into_iter()
                        .map(|k| SubsourceView {
                            key: k,
                            state: SubsourceState::Idle,
                            last_cursor: None,
                            last_sample_id: None,
                            docs_synced_total: 0,
                            last_errors: VecDeque::new(),
                            throughput: Throughput::default(),
                        })
                        .collect(),
                }
            })
            .collect();
        Self {
            sources,
            azure: AzurePanel::default(),
            focus: if prefs.focus.eq_ignore_ascii_case("azure") {
                Focus::Azure
            } else {
                Focus::Sources
            },
            prefs,
            status: EngineStatus::Idle,
            footer: "Waiting for sync activity. Use arrows to inspect sources, s to toggle logs, q to quit.".into(),
            log_tail: VecDeque::with_capacity(LOG_CAP),
            drops: 0,
            selected_source: 0,
            selected_subsource: None,
        }
    }

    pub fn apply(&mut self, ev: QuelchEvent) {
        match ev {
            QuelchEvent::CycleStarted { cycle, at } => {
                self.status = EngineStatus::Syncing { cycle, since: at };
                self.footer = format!("Cycle {cycle} started");
            }
            QuelchEvent::CycleFinished { cycle, duration } => {
                if !matches!(self.status, EngineStatus::Paused | EngineStatus::Shutdown) {
                    self.status = EngineStatus::Idle;
                }
                self.footer = format!(
                    "Cycle {cycle} finished in {:.1}s",
                    duration.as_secs_f32()
                );
            }
            QuelchEvent::SourceStarted { source } => {
                if let Some(src) = self.find_source_mut(&source) {
                    src.state = SourceState::Syncing;
                }
                self.footer = format!("Syncing source {source}");
            }
            QuelchEvent::SourceFinished {
                source,
                docs_synced,
                duration,
            } => {
                self.recompute_source_state(&source);
                self.footer = format!(
                    "Finished {source}, synced {docs_synced} docs in {:.1}s",
                    duration.as_secs_f32()
                );
            }
            QuelchEvent::SubsourceStarted { source, subsource } => {
                if let Some(sub) = self.find_subsource_mut(&source, &subsource) {
                    sub.state = SubsourceState::Syncing;
                }
                if let Some(src) = self.find_source_mut(&source) {
                    src.state = SourceState::Syncing;
                }
                self.footer = format!("Syncing {source}/{subsource}");
            }
            QuelchEvent::SubsourceFinished {
                source,
                subsource,
                cursor,
            } => {
                if let Some(sub) = self.find_subsource_mut(&source, &subsource) {
                    sub.state = SubsourceState::Idle;
                    sub.last_cursor = Some(cursor);
                }
                self.recompute_source_state(&source);
                self.footer = format!("Finished {source}/{subsource}");
            }
            QuelchEvent::SubsourceBatch {
                source,
                subsource,
                fetched,
                sample_id,
                ..
            } => {
                if let Some(sub) = self.find_subsource_mut(&source, &subsource) {
                    sub.docs_synced_total += fetched;
                    sub.last_sample_id = Some(sample_id);
                    sub.throughput.add(Instant::now(), fetched);
                }
                self.footer = format!("{source}/{subsource}: pushed {fetched} docs");
            }
            QuelchEvent::SubsourceFailed {
                source,
                subsource,
                error,
            } => {
                if let Some(sub) = self.find_subsource_mut(&source, &subsource) {
                    sub.state = SubsourceState::Error(error.clone());
                    if sub.last_errors.len() >= LAST_ERRORS_CAP {
                        sub.last_errors.pop_front();
                    }
                    sub.last_errors.push_back(error);
                }
                self.recompute_source_state(&source);
                self.footer = format!("error: {source}/{subsource} failed");
            }
            QuelchEvent::SourceFailed { source, error } => {
                if let Some(src) = self.find_source_mut(&source) {
                    src.state = SourceState::Error(error.clone());
                }
                self.footer = format!("error: {}: {}", source, error);
            }
            QuelchEvent::AzureResponse {
                at,
                status,
                latency,
                throttled,
            } => {
                self.azure.on_response(at, status, latency, throttled);
            }
            QuelchEvent::BackoffStarted {
                source,
                until,
                reason,
            } => {
                if let Some(src) = self.find_source_mut(&source) {
                    src.state = SourceState::Backoff { until };
                }
                self.footer = format!("{source} backing off: {reason}");
            }
            QuelchEvent::BackoffFinished { source } => {
                self.recompute_source_state(&source);
                self.footer = format!("{source} resumed after backoff");
            }
            QuelchEvent::Log {
                level,
                target,
                message,
                ts,
            } => {
                if self.log_tail.len() >= LOG_CAP {
                    self.log_tail.pop_front();
                }
                self.log_tail.push_back(LogLine {
                    ts,
                    level,
                    target,
                    message: if message.is_empty() {
                        "event".into()
                    } else {
                        message
                    },
                });
            }
            QuelchEvent::AzureRequest { .. } | QuelchEvent::DocSynced { .. } | QuelchEvent::DocFailed { .. } => {}
        }

        self.ensure_valid_selection();
    }

    pub fn focused_source_name(&self) -> Option<&str> {
        self.sources.get(self.selected_source).map(|source| source.name.as_str())
    }

    pub fn focused_subsource_name(&self) -> Option<&str> {
        let source = self.sources.get(self.selected_source)?;
        let sub_idx = self.selected_subsource?;
        source.subsources.get(sub_idx).map(|sub| sub.key.as_str())
    }

    pub fn move_selection_down(&mut self) {
        self.ensure_valid_selection();

        let Some(source) = self.sources.get(self.selected_source) else {
            return;
        };

        let source_is_collapsed = self.prefs.is_source_collapsed(&source.name);
        match self.selected_subsource {
            Some(sub_idx) if sub_idx + 1 < source.subsources.len() => {
                self.selected_subsource = Some(sub_idx + 1);
            }
            Some(_) | None if self.selected_source + 1 < self.sources.len() => {
                if self.selected_subsource.is_none()
                    && !source_is_collapsed
                    && !source.subsources.is_empty()
                {
                    self.selected_subsource = Some(0);
                } else {
                    self.selected_source += 1;
                    self.selected_subsource = None;
                }
            }
            None if !source_is_collapsed && !source.subsources.is_empty() => {
                self.selected_subsource = Some(0);
            }
            _ => {}
        }
    }

    pub fn move_selection_up(&mut self) {
        self.ensure_valid_selection();

        match self.selected_subsource {
            Some(sub_idx) if sub_idx > 0 => {
                self.selected_subsource = Some(sub_idx - 1);
            }
            Some(_) => {
                self.selected_subsource = None;
            }
            None if self.selected_source > 0 => {
                self.selected_source -= 1;
                let prev = &self.sources[self.selected_source];
                if !self.prefs.is_source_collapsed(&prev.name) && !prev.subsources.is_empty() {
                    self.selected_subsource = Some(prev.subsources.len() - 1);
                }
            }
            None => {}
        }
    }

    pub fn move_selection_left(&mut self) {
        self.ensure_valid_selection();

        if self.selected_subsource.is_some() {
            self.selected_subsource = None;
            return;
        }

        if let Some(source) = self.focused_source_name().map(str::to_string)
            && !self.prefs.is_source_collapsed(&source)
        {
            self.prefs.toggle_source_collapsed(&source);
        }
    }

    pub fn move_selection_right(&mut self) {
        self.ensure_valid_selection();

        if self.selected_subsource.is_some() {
            return;
        }

        let Some(source) = self.sources.get(self.selected_source) else {
            return;
        };

        if self.prefs.is_source_collapsed(&source.name) {
            self.prefs.toggle_source_collapsed(&source.name);
        } else if !source.subsources.is_empty() {
            self.selected_subsource = Some(0);
        }
    }

    pub fn toggle_selected_collapsed(&mut self) {
        self.ensure_valid_selection();

        let Some(source_name) = self.focused_source_name().map(str::to_string) else {
            return;
        };

        match self.focused_subsource_name().map(str::to_string) {
            Some(subsource_name) => self
                .prefs
                .toggle_subsource_collapsed(&source_name, &subsource_name),
            None => self.prefs.toggle_source_collapsed(&source_name),
        }
    }

    pub fn selected_source_total_docs(&self, source_name: &str) -> u64 {
        self.sources
            .iter()
            .find(|source| source.name == source_name)
            .map(|source| source.subsources.iter().map(|sub| sub.docs_synced_total).sum())
            .unwrap_or(0)
    }

    fn ensure_valid_selection(&mut self) {
        if self.sources.is_empty() {
            self.selected_source = 0;
            self.selected_subsource = None;
            return;
        }

        self.selected_source = self.selected_source.min(self.sources.len() - 1);
        let sub_len = self.sources[self.selected_source].subsources.len();
        self.selected_subsource = self.selected_subsource.filter(|idx| *idx < sub_len);
    }

    fn recompute_source_state(&mut self, source_name: &str) {
        let Some(source) = self.find_source_mut(source_name) else {
            return;
        };

        if matches!(source.state, SourceState::Backoff { .. }) {
            return;
        }

        source.state = if source
            .subsources
            .iter()
            .any(|sub| matches!(sub.state, SubsourceState::Syncing))
        {
            SourceState::Syncing
        } else if let Some(error) = source.subsources.iter().find_map(|sub| match &sub.state {
            SubsourceState::Error(error) => Some(error.clone()),
            _ => None,
        }) {
            SourceState::Error(error)
        } else {
            SourceState::Idle
        };
    }

    fn find_source_mut(&mut self, name: &str) -> Option<&mut SourceView> {
        self.sources.iter_mut().find(|s| s.name == name)
    }

    fn find_subsource_mut(&mut self, src: &str, sub: &str) -> Option<&mut SubsourceView> {
        self.find_source_mut(src)
            .and_then(|s| s.subsources.iter_mut().find(|ss| ss.key == sub))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        AuthConfig, AzureConfig, Config, JiraSourceConfig, SourceConfig, SyncConfig,
    };

    fn cfg() -> Config {
        Config {
            azure: AzureConfig {
                endpoint: "http://x".into(),
                api_key: "k".into(),
            },
            sources: vec![SourceConfig::Jira(JiraSourceConfig {
                name: "my-jira".into(),
                url: "http://x".into(),
                auth: AuthConfig::DataCenter { pat: "p".into() },
                projects: vec!["DO".into(), "HR".into()],
                index: "idx".into(),
            })],
            sync: SyncConfig::default(),
        }
    }

    #[test]
    fn initialises_sources_and_subsources() {
        let a = App::new(&cfg(), Prefs::default());
        assert_eq!(a.sources.len(), 1);
        assert_eq!(a.sources[0].subsources.len(), 2);
        assert_eq!(a.sources[0].subsources[0].key, "DO");
    }

    #[test]
    fn applies_batch_event() {
        let mut a = App::new(&cfg(), Prefs::default());
        a.apply(QuelchEvent::SubsourceBatch {
            source: "my-jira".into(),
            subsource: "DO".into(),
            fetched: 5,
            cursor: Utc::now(),
            sample_id: "DO-1".into(),
        });
        let s = &a.sources[0].subsources[0];
        assert_eq!(s.docs_synced_total, 5);
        assert_eq!(s.last_sample_id.as_deref(), Some("DO-1"));
    }

    #[test]
    fn arrow_navigation_walks_visible_tree() {
        let mut a = App::new(&cfg(), Prefs::default());

        assert_eq!(a.focused_source_name(), Some("my-jira"));
        assert_eq!(a.focused_subsource_name(), None);

        a.move_selection_down();
        assert_eq!(a.focused_subsource_name(), Some("DO"));

        a.move_selection_down();
        assert_eq!(a.focused_subsource_name(), Some("HR"));

        a.move_selection_up();
        assert_eq!(a.focused_subsource_name(), Some("DO"));

        a.move_selection_left();
        assert_eq!(a.focused_subsource_name(), None);
    }
}
