// TODO(quelch v2 task 3.3): Reimplement ConfluenceConnector using the v2 SourceConnector trait.
//
// The v1 connector logic (auth, CQL paging, JSON parsing) has been removed because
// the v1 trait shape (fetch_changes / fetch_all_ids) is incompatible with v2.
// Task 3.3 will provide the full implementation.
//
// Key v1 patterns to replicate in 3.3:
//   - Basic auth via base64-encoded "user:token"
//   - CQL filter: `space = "{key}" AND lastModified >= "{timestamp}" ORDER BY lastModified ASC`
//   - Pagination via `_links.next` or `start` / `limit` / `size` fields
//   - Fields to extract: title, body (storage format), space, version, ancestors, labels
//   - source_link: `{base_url}/wiki{page._links.webui}`
//   - partition_key: space key (subsource)
//   - Companion containers: spaces (populated from page.space fields)

use chrono::{DateTime, Utc};

use super::{BackfillCheckpoint, Companions, FetchPage, SourceConnector};
use crate::config::ConfluenceSourceConfig;

/// Confluence source connector — stub pending Task 3.3.
pub struct ConfluenceConnector {
    pub config: ConfluenceSourceConfig,
}

impl ConfluenceConnector {
    /// Create a new `ConfluenceConnector` from config.
    pub fn new(config: ConfluenceSourceConfig) -> Self {
        Self { config }
    }
}

impl SourceConnector for ConfluenceConnector {
    fn source_type(&self) -> &str {
        "confluence"
    }

    fn source_name(&self) -> &str {
        &self.config.name
    }

    fn subsources(&self) -> &[String] {
        &self.config.spaces
    }

    fn primary_container(&self) -> &str {
        // TODO(quelch v2 task 3.3): derive from config or use "confluence-pages"
        "confluence-pages"
    }

    async fn fetch_window(
        &self,
        _subsource: &str,
        _window_start: DateTime<Utc>,
        _window_end: DateTime<Utc>,
        _batch_size: usize,
        _page_token: Option<&str>,
    ) -> anyhow::Result<FetchPage> {
        unimplemented!("TODO(quelch v2 task 3.3): implement ConfluenceConnector::fetch_window")
    }

    async fn fetch_backfill_page(
        &self,
        _subsource: &str,
        _backfill_target: DateTime<Utc>,
        _last_seen: Option<&BackfillCheckpoint>,
        _batch_size: usize,
    ) -> anyhow::Result<FetchPage> {
        unimplemented!(
            "TODO(quelch v2 task 3.3): implement ConfluenceConnector::fetch_backfill_page"
        )
    }

    async fn list_all_ids(&self, _subsource: &str) -> anyhow::Result<Vec<String>> {
        unimplemented!("TODO(quelch v2 task 3.3): implement ConfluenceConnector::list_all_ids")
    }

    async fn fetch_companions(&self, _subsource: &str) -> anyhow::Result<Companions> {
        unimplemented!("TODO(quelch v2 task 3.3): implement ConfluenceConnector::fetch_companions")
    }
}
