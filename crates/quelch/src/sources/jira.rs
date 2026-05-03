// TODO(quelch v2 task 3.2): Reimplement JiraConnector using the v2 SourceConnector trait.
//
// The v1 connector logic (auth, JQL paging, JSON parsing) has been removed because
// the v1 trait shape (fetch_changes / fetch_all_ids) is incompatible with v2.
// Task 3.2 will provide the full implementation.
//
// Key v1 patterns to replicate in 3.2:
//   - Basic auth via base64-encoded "user:token"
//   - JQL filter: `project = {key} AND updated >= "{timestamp}" ORDER BY updated ASC`
//   - Pagination via `startAt` / `maxResults` / `total` fields in the response
//   - Fields to extract: summary, description, status, assignee, reporter, priority,
//     issuetype, labels, fixVersions, sprint, created, updated
//   - source_link: `{base_url}/browse/{issue_key}`
//   - partition_key: project key (subsource)
//   - Companion containers: sprints, fix_versions (populated from issue fields)

use chrono::{DateTime, Utc};

use super::{BackfillCheckpoint, Companions, FetchPage, SourceConnector};
use crate::config::JiraSourceConfig;

/// Jira source connector — stub pending Task 3.2.
pub struct JiraConnector {
    pub config: JiraSourceConfig,
}

impl JiraConnector {
    /// Create a new `JiraConnector` from config.
    pub fn new(config: JiraSourceConfig) -> Self {
        Self { config }
    }
}

impl SourceConnector for JiraConnector {
    fn source_type(&self) -> &str {
        "jira"
    }

    fn source_name(&self) -> &str {
        &self.config.name
    }

    fn subsources(&self) -> &[String] {
        &self.config.projects
    }

    fn primary_container(&self) -> &str {
        // TODO(quelch v2 task 3.2): derive from config or use "jira-issues"
        "jira-issues"
    }

    async fn fetch_window(
        &self,
        _subsource: &str,
        _window_start: DateTime<Utc>,
        _window_end: DateTime<Utc>,
        _batch_size: usize,
        _page_token: Option<&str>,
    ) -> anyhow::Result<FetchPage> {
        unimplemented!("TODO(quelch v2 task 3.2): implement JiraConnector::fetch_window")
    }

    async fn fetch_backfill_page(
        &self,
        _subsource: &str,
        _backfill_target: DateTime<Utc>,
        _last_seen: Option<&BackfillCheckpoint>,
        _batch_size: usize,
    ) -> anyhow::Result<FetchPage> {
        unimplemented!("TODO(quelch v2 task 3.2): implement JiraConnector::fetch_backfill_page")
    }

    async fn list_all_ids(&self, _subsource: &str) -> anyhow::Result<Vec<String>> {
        unimplemented!("TODO(quelch v2 task 3.2): implement JiraConnector::list_all_ids")
    }

    async fn fetch_companions(&self, _subsource: &str) -> anyhow::Result<Companions> {
        unimplemented!("TODO(quelch v2 task 3.2): implement JiraConnector::fetch_companions")
    }
}
