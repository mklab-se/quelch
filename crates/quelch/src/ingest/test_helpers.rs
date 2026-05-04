//! Test helpers for the ingest engine — `MockConnector` and friends.
//!
//! Only compiled in `#[cfg(test)]` contexts.

#![cfg(test)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeZone, Utc};

use crate::sources::{BackfillCheckpoint, Companions, FetchPage, SourceConnector, SourceDocument};

// ---------------------------------------------------------------------------
// Test document builder
// ---------------------------------------------------------------------------

/// Build a minimal [`SourceDocument`] with the given id and partition key.
pub fn make_source_doc(id: &str, partition_key: &str) -> SourceDocument {
    let mut fields = HashMap::new();
    fields.insert("title".into(), format!("Doc {id}").into());
    SourceDocument {
        id: id.to_string(),
        partition_key: partition_key.to_string(),
        fields,
        updated_at: Utc.with_ymd_and_hms(2024, 6, 1, 10, 0, 0).single().unwrap(),
        source_link: format!("https://example.com/{id}"),
    }
}

// ---------------------------------------------------------------------------
// MockConnector
// ---------------------------------------------------------------------------

/// Canned response for one `fetch_window` call.
pub struct WindowPage {
    pub documents: Vec<SourceDocument>,
    pub next_page_token: Option<String>,
}

/// Canned response for one `fetch_backfill_page` call.
pub struct BackfillPage {
    pub documents: Vec<SourceDocument>,
    /// The `last_seen` checkpoint (normally the last doc's updated+key).
    pub last_seen: Option<BackfillCheckpoint>,
}

/// Control knob for `list_all_ids`.
pub enum ListIdsResponse {
    Ok(Vec<String>),
    Err(String),
}

/// Simple mock source connector for ingest engine tests.
///
/// Callers pre-load canned pages via the builder methods; the mock connector
/// returns them in order.  Methods that run out of pre-loaded pages return an
/// empty page (end-of-stream).
///
/// For error injection: set `fetch_window_error_on_page` to return an error
/// on the Nth `fetch_window` call (0-indexed).
#[derive(Clone)]
pub struct MockConnector {
    pub source_name: String,
    pub primary_container: String,

    /// Pre-loaded window pages, returned in order.
    window_pages: Arc<Mutex<Vec<Result<WindowPage, String>>>>,
    /// Pre-loaded backfill pages, returned in order.
    backfill_pages: Arc<Mutex<Vec<Result<BackfillPage, String>>>>,
    /// IDs returned by `list_all_ids`.
    list_ids: Arc<Mutex<Option<ListIdsResponse>>>,
    /// Companions returned by `fetch_companions`.
    companions: Arc<Mutex<Option<Companions>>>,
}

impl MockConnector {
    /// Create a new mock connector.
    pub fn new(source_name: &str, primary_container: &str) -> Self {
        Self {
            source_name: source_name.to_string(),
            primary_container: primary_container.to_string(),
            window_pages: Arc::new(Mutex::new(Vec::new())),
            backfill_pages: Arc::new(Mutex::new(Vec::new())),
            list_ids: Arc::new(Mutex::new(None)),
            companions: Arc::new(Mutex::new(None)),
        }
    }

    /// Push one window page (success).
    pub fn push_window_page(&self, docs: Vec<SourceDocument>, next_page_token: Option<String>) {
        self.window_pages.lock().unwrap().push(Ok(WindowPage {
            documents: docs,
            next_page_token,
        }));
    }

    /// Push a window page that returns an error.
    pub fn push_window_error(&self, msg: impl Into<String>) {
        self.window_pages.lock().unwrap().push(Err(msg.into()));
    }

    /// Push one backfill page (success).
    pub fn push_backfill_page(
        &self,
        docs: Vec<SourceDocument>,
        last_seen: Option<BackfillCheckpoint>,
    ) {
        self.backfill_pages.lock().unwrap().push(Ok(BackfillPage {
            documents: docs,
            last_seen,
        }));
    }

    /// Push a backfill page that returns an error.
    pub fn push_backfill_error(&self, msg: impl Into<String>) {
        self.backfill_pages.lock().unwrap().push(Err(msg.into()));
    }

    /// Set the response for `list_all_ids`.
    pub fn set_list_ids(&self, ids: Vec<String>) {
        *self.list_ids.lock().unwrap() = Some(ListIdsResponse::Ok(ids));
    }

    /// Set `list_all_ids` to return an error.
    pub fn set_list_ids_error(&self, msg: impl Into<String>) {
        *self.list_ids.lock().unwrap() = Some(ListIdsResponse::Err(msg.into()));
    }

    /// Set the companions returned by `fetch_companions`.
    pub fn set_companions(&self, companions: Companions) {
        *self.companions.lock().unwrap() = Some(companions);
    }
}

impl SourceConnector for MockConnector {
    fn source_type(&self) -> &str {
        "mock"
    }

    fn source_name(&self) -> &str {
        &self.source_name
    }

    fn subsources(&self) -> &[String] {
        // Return a static empty slice — tests set up keys via CursorKey directly.
        &[]
    }

    fn primary_container(&self) -> &str {
        &self.primary_container
    }

    async fn fetch_window(
        &self,
        _subsource: &str,
        _window_start: DateTime<Utc>,
        _window_end: DateTime<Utc>,
        _batch_size: usize,
        _page_token: Option<&str>,
    ) -> anyhow::Result<FetchPage> {
        let mut pages = self.window_pages.lock().unwrap();
        match pages.first() {
            None => {
                // No more pages — return empty (end of stream).
                Ok(FetchPage {
                    documents: vec![],
                    next_page_token: None,
                    last_seen: None,
                })
            }
            Some(Ok(_)) => {
                let page = match pages.remove(0) {
                    Ok(p) => p,
                    Err(_) => unreachable!(),
                };
                Ok(FetchPage {
                    documents: page.documents,
                    next_page_token: page.next_page_token,
                    last_seen: None,
                })
            }
            Some(Err(_)) => {
                let err = match pages.remove(0) {
                    Err(e) => e,
                    Ok(_) => unreachable!(),
                };
                Err(anyhow::anyhow!("{err}"))
            }
        }
    }

    async fn fetch_backfill_page(
        &self,
        _subsource: &str,
        _backfill_target: DateTime<Utc>,
        _last_seen: Option<&BackfillCheckpoint>,
        _batch_size: usize,
    ) -> anyhow::Result<FetchPage> {
        let mut pages = self.backfill_pages.lock().unwrap();
        match pages.first() {
            None => Ok(FetchPage {
                documents: vec![],
                next_page_token: None,
                last_seen: None,
            }),
            Some(Ok(_)) => {
                let page = match pages.remove(0) {
                    Ok(p) => p,
                    Err(_) => unreachable!(),
                };
                Ok(FetchPage {
                    documents: page.documents,
                    next_page_token: None,
                    last_seen: page.last_seen,
                })
            }
            Some(Err(_)) => {
                let err = match pages.remove(0) {
                    Err(e) => e,
                    Ok(_) => unreachable!(),
                };
                Err(anyhow::anyhow!("{err}"))
            }
        }
    }

    async fn list_all_ids(&self, _subsource: &str) -> anyhow::Result<Vec<String>> {
        match self.list_ids.lock().unwrap().as_ref() {
            Some(ListIdsResponse::Ok(ids)) => Ok(ids.clone()),
            Some(ListIdsResponse::Err(e)) => Err(anyhow::anyhow!("{e}")),
            None => Ok(vec![]),
        }
    }

    async fn fetch_companions(&self, _subsource: &str) -> anyhow::Result<Companions> {
        Ok(self.companions.lock().unwrap().take().unwrap_or_default())
    }
}
