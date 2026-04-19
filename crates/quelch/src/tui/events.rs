//! QuelchEvent — the TUI's view-side representation of engine tracing events.

use chrono::{DateTime, Utc};
use std::time::{Duration, Instant};
use tracing::Level;

pub type SourceId = String;
pub type SubsourceId = String;

#[derive(Debug, Clone)]
pub enum QuelchEvent {
    CycleStarted {
        cycle: u64,
        at: DateTime<Utc>,
    },
    CycleFinished {
        cycle: u64,
        duration: Duration,
    },

    SourceStarted {
        source: SourceId,
    },
    SourceFinished {
        source: SourceId,
        docs_synced: u64,
        duration: Duration,
    },
    SourceFailed {
        source: SourceId,
        error: String,
    },

    SubsourceStarted {
        source: SourceId,
        subsource: SubsourceId,
    },
    SubsourceFinished {
        source: SourceId,
        subsource: SubsourceId,
        cursor: DateTime<Utc>,
    },
    SubsourceFailed {
        source: SourceId,
        subsource: SubsourceId,
        error: String,
    },
    SubsourceBatch {
        source: SourceId,
        subsource: SubsourceId,
        fetched: u64,
        cursor: DateTime<Utc>,
        sample_id: String,
    },

    DocSynced {
        source: SourceId,
        subsource: SubsourceId,
        id: String,
        updated: DateTime<Utc>,
    },
    DocFailed {
        source: SourceId,
        subsource: SubsourceId,
        id: String,
        error: String,
    },

    AzureRequest {
        at: Instant,
        method: String,
        path: String,
    },
    AzureResponse {
        at: Instant,
        status: u16,
        latency: Duration,
        throttled: bool,
    },

    BackoffStarted {
        source: SourceId,
        until: DateTime<Utc>,
        reason: String,
    },
    BackoffFinished {
        source: SourceId,
    },

    Log {
        level: Level,
        target: String,
        message: String,
        ts: DateTime<Utc>,
    },
}

impl QuelchEvent {
    /// Lifecycle events must never be dropped under backpressure.
    pub fn is_lifecycle(&self) -> bool {
        matches!(
            self,
            QuelchEvent::CycleStarted { .. }
                | QuelchEvent::CycleFinished { .. }
                | QuelchEvent::SourceStarted { .. }
                | QuelchEvent::SourceFinished { .. }
                | QuelchEvent::SourceFailed { .. }
                | QuelchEvent::SubsourceStarted { .. }
                | QuelchEvent::SubsourceFinished { .. }
                | QuelchEvent::SubsourceFailed { .. }
                | QuelchEvent::BackoffStarted { .. }
                | QuelchEvent::BackoffFinished { .. }
                | QuelchEvent::AzureResponse { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_classification() {
        assert!(
            QuelchEvent::CycleStarted {
                cycle: 1,
                at: Utc::now()
            }
            .is_lifecycle()
        );
        assert!(
            QuelchEvent::AzureResponse {
                at: Instant::now(),
                status: 200,
                latency: Duration::from_millis(10),
                throttled: false
            }
            .is_lifecycle()
        );
        assert!(
            !QuelchEvent::Log {
                level: Level::INFO,
                target: "x".into(),
                message: "y".into(),
                ts: Utc::now()
            }
            .is_lifecycle()
        );
        assert!(
            !QuelchEvent::DocSynced {
                source: "s".into(),
                subsource: "ss".into(),
                id: "i".into(),
                updated: Utc::now()
            }
            .is_lifecycle()
        );
    }
}
