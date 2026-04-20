//! Custom tracing Layer that maps engine events to `QuelchEvent`.
//!
//! Attaches a bounded `mpsc::Sender`-style channel. When full, the oldest
//! **non-lifecycle** event in the layer's internal overflow buffer is
//! dropped and the `drops` counter is bumped. Lifecycle events (see
//! `QuelchEvent::is_lifecycle`) are never dropped.

use chrono::Utc;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

use super::events::QuelchEvent;
use crate::sync::phases;

const EVENT_CHANNEL_CAP: usize = 1024;
const OVERFLOW_CAP: usize = 1024;

#[derive(Clone)]
pub struct TuiLayer {
    inner: Arc<Inner>,
}

struct Inner {
    tx: mpsc::Sender<QuelchEvent>,
    overflow: Mutex<VecDeque<QuelchEvent>>,
    drops: Arc<AtomicU64>,
}

/// Returns the layer + the receiver the TUI will consume + an external
/// handle to the drops counter (so the TUI footer can display it).
pub fn layer_and_receiver() -> (TuiLayer, mpsc::Receiver<QuelchEvent>, Arc<AtomicU64>) {
    let (tx, rx) = mpsc::channel(EVENT_CHANNEL_CAP);
    let drops = Arc::new(AtomicU64::new(0));
    let layer = TuiLayer {
        inner: Arc::new(Inner {
            tx,
            overflow: Mutex::new(VecDeque::with_capacity(OVERFLOW_CAP)),
            drops: drops.clone(),
        }),
    };
    (layer, rx, drops)
}

impl TuiLayer {
    fn emit(&self, ev: QuelchEvent) {
        match self.inner.tx.try_send(ev) {
            Ok(_) => {}
            Err(mpsc::error::TrySendError::Full(ev)) => self.enqueue_overflow(ev),
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
        self.drain_overflow();
    }

    fn enqueue_overflow(&self, ev: QuelchEvent) {
        let mut q = self.inner.overflow.lock().unwrap();
        if q.len() >= OVERFLOW_CAP {
            let victim_idx = q.iter().position(|e| !e.is_lifecycle()).unwrap_or(0);
            q.remove(victim_idx);
            self.inner.drops.fetch_add(1, Ordering::Relaxed);
        }
        q.push_back(ev);
    }

    fn drain_overflow(&self) {
        let mut q = self.inner.overflow.lock().unwrap();
        while let Some(ev) = q.pop_front() {
            match self.inner.tx.try_send(ev) {
                Ok(_) => {}
                Err(mpsc::error::TrySendError::Full(ev)) => {
                    q.push_front(ev);
                    break;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => break,
            }
        }
    }

    pub fn drops_counter(&self) -> u64 {
        self.inner.drops.load(Ordering::Relaxed)
    }
}

/// Visitor that picks out the fields the TuiLayer cares about.
#[derive(Default)]
struct FieldVisitor {
    phase: Option<String>,
    source: Option<String>,
    subsource: Option<String>,
    doc_id: Option<String>,
    updated: Option<String>,
    cursor: Option<String>,
    fetched: Option<u64>,
    sample_id: Option<String>,
    status: Option<u64>,
    latency_ms: Option<u64>,
    throttled: Option<u64>,
    cycle: Option<u64>,
    docs_synced: Option<u64>,
    duration_ms: Option<u64>,
    message: Option<String>,
    error: Option<String>,
    reason: Option<String>,
    delay_ms: Option<u64>,
    stage: Option<String>,
    done: Option<u64>,
    total: Option<u64>,
    count: Option<u64>,
    sample_ids: Option<String>,
    latest_id: Option<String>,
}

impl tracing::field::Visit for FieldVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        let v = value.to_string();
        match field.name() {
            "phase" => self.phase = Some(v),
            "source" => self.source = Some(v),
            "subsource" => self.subsource = Some(v),
            "doc_id" => self.doc_id = Some(v),
            "updated" => self.updated = Some(v),
            "cursor" => self.cursor = Some(v),
            "sample_id" => self.sample_id = Some(v),
            "message" => self.message = Some(v),
            "error" => self.error = Some(v),
            "reason" => self.reason = Some(v),
            "stage" => self.stage = Some(v),
            "sample_ids" => self.sample_ids = Some(v),
            "latest_id" => self.latest_id = Some(v),
            _ => {}
        }
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "fetched" => self.fetched = Some(value),
            "status" => self.status = Some(value),
            "latency_ms" => self.latency_ms = Some(value),
            "throttled" => self.throttled = Some(value),
            "cycle" => self.cycle = Some(value),
            "docs_synced" => self.docs_synced = Some(value),
            "duration_ms" => self.duration_ms = Some(value),
            "delay_ms" => self.delay_ms = Some(value),
            "done" => self.done = Some(value),
            "total" => self.total = Some(value),
            "count" => self.count = Some(value),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let v = format!("{value:?}");
        match field.name() {
            "cursor" => self.cursor = Some(v.trim_matches('"').to_string()),
            "message" => self.message = Some(v),
            "error" => self.error = Some(v.trim_matches('"').to_string()),
            _ => {}
        }
    }
}

impl<S> Layer<S> for TuiLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut v = FieldVisitor::default();
        event.record(&mut v);

        let qe = match v.phase.as_deref() {
            Some(p) if p == phases::CYCLE_STARTED => Some(QuelchEvent::CycleStarted {
                cycle: v.cycle.unwrap_or(0),
                at: Utc::now(),
            }),
            Some(p) if p == phases::CYCLE_FINISHED => Some(QuelchEvent::CycleFinished {
                cycle: v.cycle.unwrap_or(0),
                duration: Duration::from_millis(v.duration_ms.unwrap_or(0)),
            }),
            Some(p) if p == phases::SOURCE_STARTED => v
                .source
                .clone()
                .map(|source| QuelchEvent::SourceStarted { source }),
            Some(p) if p == phases::SOURCE_FINISHED => {
                v.source.clone().map(|source| QuelchEvent::SourceFinished {
                    source,
                    docs_synced: v.docs_synced.unwrap_or(0),
                    duration: Duration::from_millis(v.duration_ms.unwrap_or(0)),
                })
            }
            Some(p) if p == phases::SUBSOURCE_STARTED => {
                v.source.clone().zip(v.subsource.clone()).map(|(s, ss)| {
                    QuelchEvent::SubsourceStarted {
                        source: s,
                        subsource: ss,
                    }
                })
            }
            Some(p) if p == phases::SUBSOURCE_FINISHED => {
                v.source.clone().zip(v.subsource.clone()).map(|(s, ss)| {
                    QuelchEvent::SubsourceFinished {
                        source: s,
                        subsource: ss,
                        cursor: Utc::now(),
                    }
                })
            }
            Some(p) if p == phases::SUBSOURCE_BATCH => {
                v.source.clone().zip(v.subsource.clone()).map(|(s, ss)| {
                    QuelchEvent::SubsourceBatch {
                        source: s,
                        subsource: ss,
                        fetched: v.fetched.unwrap_or(0),
                        cursor: Utc::now(),
                        sample_id: v.sample_id.clone().unwrap_or_default(),
                    }
                })
            }
            Some(p) if p == phases::SUBSOURCE_FAILED => {
                v.source.clone().zip(v.subsource.clone()).map(|(s, ss)| {
                    QuelchEvent::SubsourceFailed {
                        source: s,
                        subsource: ss,
                        error: v
                            .error
                            .clone()
                            .or_else(|| v.message.clone())
                            .unwrap_or_default(),
                    }
                })
            }
            Some(p) if p == phases::SOURCE_FAILED => {
                v.source.clone().map(|s| QuelchEvent::SourceFailed {
                    source: s,
                    error: v
                        .error
                        .clone()
                        .or_else(|| v.message.clone())
                        .unwrap_or_default(),
                })
            }
            Some(p) if p == phases::DOC_SYNCED => v
                .source
                .clone()
                .zip(v.subsource.clone())
                .zip(v.doc_id.clone())
                .map(|((s, ss), id)| QuelchEvent::DocSynced {
                    source: s,
                    subsource: ss,
                    id,
                    updated: Utc::now(),
                }),
            Some(p) if p == phases::DOC_PUSHED => v
                .source
                .clone()
                .zip(v.subsource.clone())
                .zip(v.doc_id.clone())
                .map(|((s, ss), id)| QuelchEvent::DocPushed {
                    source: s,
                    subsource: ss,
                    id,
                    updated: Utc::now(),
                }),
            Some(p) if p == phases::BATCH_PUSHED => {
                v.source.clone().zip(v.subsource.clone()).map(|(s, ss)| {
                    let sample_ids = v
                        .sample_ids
                        .as_deref()
                        .map(|csv| {
                            csv.split(',')
                                .map(|part| part.trim().to_string())
                                .filter(|p| !p.is_empty())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let latest_id = v
                        .latest_id
                        .clone()
                        .unwrap_or_else(|| sample_ids.last().cloned().unwrap_or_default());
                    QuelchEvent::BatchPushed {
                        source: s,
                        subsource: ss,
                        count: v.count.unwrap_or(0),
                        sample_ids,
                        latest_id,
                    }
                })
            }
            Some(p) if p == phases::INDEX_COUNT => {
                v.source.clone().map(|s| QuelchEvent::IndexCount {
                    source: s,
                    count: v.count.unwrap_or(0),
                })
            }
            Some(p) if p == phases::SUBSOURCE_COUNT => {
                v.source.clone().zip(v.subsource.clone()).map(|(s, ss)| {
                    QuelchEvent::SubsourceCount {
                        source: s,
                        subsource: ss,
                        count: v.count.unwrap_or(0),
                    }
                })
            }
            Some(p) if p == phases::STAGE => v.source.clone().zip(v.subsource.clone()).and_then(
                |(s, ss)| -> Option<QuelchEvent> {
                    let stage = match v.stage.as_deref()? {
                        "fetching" => crate::tui::events::Stage::Fetching,
                        "embedding" => crate::tui::events::Stage::Embedding {
                            done: v.done.unwrap_or(0),
                            total: v.total.unwrap_or(0),
                        },
                        "pushing" => crate::tui::events::Stage::Pushing {
                            total: v.total.unwrap_or(0),
                        },
                        "idle" => crate::tui::events::Stage::Idle,
                        _ => return None,
                    };
                    Some(QuelchEvent::Stage {
                        source: s,
                        subsource: ss,
                        stage,
                    })
                },
            ),
            Some(p) if p == phases::AZURE_RESPONSE => Some(QuelchEvent::AzureResponse {
                at: Instant::now(),
                status: v.status.unwrap_or(0) as u16,
                latency: Duration::from_millis(v.latency_ms.unwrap_or(0)),
                throttled: v.throttled.unwrap_or(0) != 0,
            }),
            Some(p) if p == phases::BACKOFF_STARTED => {
                v.source.clone().map(|s| QuelchEvent::BackoffStarted {
                    source: s,
                    until: chrono::Utc::now()
                        + chrono::Duration::milliseconds(v.delay_ms.unwrap_or(0) as i64),
                    reason: v.reason.clone().unwrap_or_default(),
                })
            }
            Some(p) if p == phases::BACKOFF_FINISHED => v
                .source
                .clone()
                .map(|s| QuelchEvent::BackoffFinished { source: s }),
            _ => None,
        };

        let final_event = qe.unwrap_or_else(|| QuelchEvent::Log {
            level: *event.metadata().level(),
            target: event.metadata().target().to_string(),
            message: v.message.unwrap_or_default(),
            ts: Utc::now(),
        });
        self.emit(final_event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::info;
    use tracing_subscriber::prelude::*;

    #[tokio::test]
    async fn emits_subsource_started_event() {
        let (layer, mut rx, _drops) = layer_and_receiver();
        let subscriber = tracing_subscriber::registry().with(layer);
        let _g = tracing::subscriber::set_default(subscriber);

        info!(
            phase = "subsource_started",
            source = "s",
            subsource = "ss",
            "x"
        );

        let ev = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("timed out")
            .unwrap();
        match ev {
            QuelchEvent::SubsourceStarted { source, subsource } => {
                assert_eq!(source, "s");
                assert_eq!(subsource, "ss");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn maps_unknown_events_to_log() {
        let (layer, mut rx, _drops) = layer_and_receiver();
        let subscriber = tracing_subscriber::registry().with(layer);
        let _g = tracing::subscriber::set_default(subscriber);

        info!("bare message");

        let ev = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(ev, QuelchEvent::Log { .. }));
    }

    #[tokio::test]
    async fn subsource_batch_event_roundtrips_through_tracing() {
        use crate::sync::phases;

        let (layer, mut rx, _drops) = layer_and_receiver();
        let subscriber = tracing_subscriber::registry().with(layer);
        let _g = tracing::subscriber::set_default(subscriber);

        tracing::info!(
            phase = phases::SUBSOURCE_BATCH,
            source = "s",
            subsource = "ss",
            fetched = 5u64,
            sample_id = "id-1",
            "batch"
        );

        let ev = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match ev {
            QuelchEvent::SubsourceBatch {
                source,
                subsource,
                fetched,
                sample_id,
                ..
            } => {
                assert_eq!(source, "s");
                assert_eq!(subsource, "ss");
                assert_eq!(fetched, 5);
                assert_eq!(sample_id, "id-1");
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn source_finished_event_roundtrips_through_tracing() {
        use crate::sync::phases;

        let (layer, mut rx, _drops) = layer_and_receiver();
        let subscriber = tracing_subscriber::registry().with(layer);
        let _g = tracing::subscriber::set_default(subscriber);

        tracing::info!(
            phase = phases::SOURCE_FINISHED,
            source = "s",
            docs_synced = 9u64,
            duration_ms = 25u64,
            "done"
        );

        let ev = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match ev {
            QuelchEvent::SourceFinished {
                source,
                docs_synced,
                duration,
            } => {
                assert_eq!(source, "s");
                assert_eq!(docs_synced, 9);
                assert_eq!(duration, Duration::from_millis(25));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn emits_source_started_and_finished() {
        use crate::sync::phases;
        use tracing_subscriber::prelude::*;

        let (layer, mut rx, _drops) = layer_and_receiver();
        let subscriber = tracing_subscriber::registry().with(layer);
        let _g = tracing::subscriber::set_default(subscriber);

        tracing::info!(phase = phases::SOURCE_STARTED, source = "my-jira", "start");
        tracing::info!(
            phase = phases::SOURCE_FINISHED,
            source = "my-jira",
            docs_synced = 42u64,
            duration_ms = 1234u64,
            "done"
        );

        let first = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match first {
            QuelchEvent::SourceStarted { source } => assert_eq!(source, "my-jira"),
            other => panic!("expected SourceStarted, got {other:?}"),
        }

        let second = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match second {
            QuelchEvent::SourceFinished {
                source,
                docs_synced,
                duration,
            } => {
                assert_eq!(source, "my-jira");
                assert_eq!(docs_synced, 42);
                assert_eq!(duration.as_millis(), 1234);
            }
            other => panic!("expected SourceFinished, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn emits_backoff_events() {
        use crate::sync::phases;
        use tracing_subscriber::prelude::*;

        let (layer, mut rx, _drops) = layer_and_receiver();
        let subscriber = tracing_subscriber::registry().with(layer);
        let _g = tracing::subscriber::set_default(subscriber);

        tracing::warn!(
            phase = phases::BACKOFF_STARTED,
            source = "azure",
            reason = "HTTP 429",
            delay_ms = 1000u64,
            "backoff"
        );
        tracing::info!(
            phase = phases::BACKOFF_FINISHED,
            source = "azure",
            "resumed"
        );

        let first = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(first, QuelchEvent::BackoffStarted { .. }));

        let second = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match second {
            QuelchEvent::BackoffFinished { source } => assert_eq!(source, "azure"),
            other => panic!("expected BackoffFinished, got {other:?}"),
        }
    }
}
