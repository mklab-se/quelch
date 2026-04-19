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
            prefs,
            status: EngineStatus::Idle,
            focus: Focus::Sources,
            footer: String::new(),
            log_tail: VecDeque::with_capacity(LOG_CAP),
            drops: 0,
        }
    }

    pub fn apply(&mut self, ev: QuelchEvent) {
        match ev {
            QuelchEvent::CycleStarted { cycle, at } => {
                self.status = EngineStatus::Syncing { cycle, since: at };
            }
            QuelchEvent::CycleFinished { .. } => {
                self.status = EngineStatus::Idle;
            }
            QuelchEvent::SubsourceStarted { source, subsource } => {
                if let Some(sub) = self.find_subsource_mut(&source, &subsource) {
                    sub.state = SubsourceState::Syncing;
                }
                if let Some(src) = self.find_source_mut(&source) {
                    src.state = SourceState::Syncing;
                }
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
                    message,
                });
            }
            _ => {}
        }
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
}
