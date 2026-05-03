// TODO(quelch v2 phase 2+): re-enable when ingest connectors land.
//
// The v1 Jira connector is stubbed for the v2 config layer work (Phase 1).
// It will be replaced by the new connector in Phase 2.

use anyhow::Result;

use super::{FetchResult, SourceConnector};
use crate::config::JiraSourceConfig;

pub struct JiraConnector {
    pub config: JiraSourceConfig,
}

impl JiraConnector {
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

    fn index_name(&self) -> &str {
        // TODO(quelch v2): index_name is a v1 concept; in v2 use container name
        "stub"
    }

    fn subsources(&self) -> &[String] {
        &self.config.projects
    }

    async fn fetch_changes(
        &self,
        _subsource: &str,
        _cursor: Option<&super::SyncCursor>,
        _batch_size: usize,
    ) -> Result<FetchResult> {
        anyhow::bail!("JiraConnector is stubbed in v2 Phase 1")
    }

    async fn fetch_all_ids(&self, _subsource: &str) -> Result<Vec<String>> {
        anyhow::bail!("JiraConnector is stubbed in v2 Phase 1")
    }
}
