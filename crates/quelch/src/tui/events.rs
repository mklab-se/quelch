//! QuelchEvent — the TUI's view-side representation of engine tracing events.

use chrono::{DateTime, Utc};
use std::time::{Duration, Instant};
use tracing::Level;

pub type SourceId = String;
pub type SubsourceId = String;

/// Pipeline stage a subsource is currently in, independent of the overall
/// `SubsourceState`. Used by the TUI to show "what's happening right now"
/// so the operator can tell fetching-from-Jira apart from embedding and
/// from pushing-to-Azure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    Idle,
    Fetching,
    /// `done` of `total` documents embedded so far in the current batch.
    Embedding {
        done: u64,
        total: u64,
    },
    /// About to push `total` documents to Azure AI Search.
    Pushing {
        total: u64,
    },
}

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
    /// A document has landed in Azure AI Search. Fired by the engine after
    /// `push_documents` returns success. This is the event the "live feed",
    /// "latest ID", and drilldown "last pushed" readouts listen to.
    DocPushed {
        source: SourceId,
        subsource: SubsourceId,
        id: String,
        updated: DateTime<Utc>,
    },
    /// A whole batch has landed in Azure. This is the coarser-grained
    /// companion to `DocPushed` — one event per successful push, carrying
    /// the batch size plus a sample of the first few IDs. The TUI live feed
    /// renders one row per batch (readable) rather than one row per doc
    /// (92 rows all with the same timestamp = noise).
    BatchPushed {
        source: SourceId,
        subsource: SubsourceId,
        count: u64,
        sample_ids: Vec<String>,
        latest_id: String,
    },
    /// Authoritative total doc count in the source's Azure index.
    IndexCount {
        source: SourceId,
        count: u64,
    },
    /// Authoritative doc count for a specific subsource within its source's
    /// index (filtered by project/space).
    SubsourceCount {
        source: SourceId,
        subsource: SubsourceId,
        count: u64,
    },
    /// What a subsource is doing RIGHT NOW inside its current batch.
    Stage {
        source: SourceId,
        subsource: SubsourceId,
        stage: Stage,
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
