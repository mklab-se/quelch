//! Enum dispatch wrapper so the generic engine (`cycle::run<C>`) can work
//! over multiple concrete connector types in a single worker.
//!
//! Because [`SourceConnector`] uses `trait_variant::make(Send)`, it is not
//! dyn-compatible.  `AnyConnector` wraps each supported connector variant and
//! implements `SourceConnector` by dispatching to the inner type.
//!
//! Adding a new connector type means:
//! 1. Add a variant here.
//! 2. Add a match arm in each method body.

use chrono::{DateTime, Utc};

use crate::sources::{
    BackfillCheckpoint, Companions, FetchPage, SourceConnector, confluence::ConfluenceConnector,
    jira::JiraConnector,
};

/// Dispatch enum for supported source connectors.
///
/// The worker holds `Vec<(CursorKey, AnyConnector)>`; `cycle::run<AnyConnector>`
/// handles either source type without boxing.
#[derive(Clone)]
pub enum AnyConnector {
    /// Jira source connector.
    Jira(JiraConnector),
    /// Confluence source connector.
    Confluence(ConfluenceConnector),
    /// In-test mock variant — only compiled in test builds.
    #[cfg(test)]
    Mock(crate::ingest::test_helpers::MockConnector),
}

impl SourceConnector for AnyConnector {
    fn source_type(&self) -> &str {
        match self {
            Self::Jira(c) => c.source_type(),
            Self::Confluence(c) => c.source_type(),
            #[cfg(test)]
            Self::Mock(c) => c.source_type(),
        }
    }

    fn source_name(&self) -> &str {
        match self {
            Self::Jira(c) => c.source_name(),
            Self::Confluence(c) => c.source_name(),
            #[cfg(test)]
            Self::Mock(c) => c.source_name(),
        }
    }

    fn subsources(&self) -> &[String] {
        match self {
            Self::Jira(c) => c.subsources(),
            Self::Confluence(c) => c.subsources(),
            #[cfg(test)]
            Self::Mock(c) => c.subsources(),
        }
    }

    fn primary_container(&self) -> &str {
        match self {
            Self::Jira(c) => c.primary_container(),
            Self::Confluence(c) => c.primary_container(),
            #[cfg(test)]
            Self::Mock(c) => c.primary_container(),
        }
    }

    async fn fetch_window(
        &self,
        subsource: &str,
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
        batch_size: usize,
        page_token: Option<&str>,
    ) -> anyhow::Result<FetchPage> {
        match self {
            Self::Jira(c) => {
                c.fetch_window(subsource, window_start, window_end, batch_size, page_token)
                    .await
            }
            Self::Confluence(c) => {
                c.fetch_window(subsource, window_start, window_end, batch_size, page_token)
                    .await
            }
            #[cfg(test)]
            Self::Mock(c) => {
                c.fetch_window(subsource, window_start, window_end, batch_size, page_token)
                    .await
            }
        }
    }

    async fn fetch_backfill_page(
        &self,
        subsource: &str,
        backfill_target: DateTime<Utc>,
        last_seen: Option<&BackfillCheckpoint>,
        batch_size: usize,
    ) -> anyhow::Result<FetchPage> {
        match self {
            Self::Jira(c) => {
                c.fetch_backfill_page(subsource, backfill_target, last_seen, batch_size)
                    .await
            }
            Self::Confluence(c) => {
                c.fetch_backfill_page(subsource, backfill_target, last_seen, batch_size)
                    .await
            }
            #[cfg(test)]
            Self::Mock(c) => {
                c.fetch_backfill_page(subsource, backfill_target, last_seen, batch_size)
                    .await
            }
        }
    }

    async fn list_all_ids(&self, subsource: &str) -> anyhow::Result<Vec<String>> {
        match self {
            Self::Jira(c) => c.list_all_ids(subsource).await,
            Self::Confluence(c) => c.list_all_ids(subsource).await,
            #[cfg(test)]
            Self::Mock(c) => c.list_all_ids(subsource).await,
        }
    }

    async fn fetch_companions(&self, subsource: &str) -> anyhow::Result<Companions> {
        match self {
            Self::Jira(c) => c.fetch_companions(subsource).await,
            Self::Confluence(c) => c.fetch_companions(subsource).await,
            #[cfg(test)]
            Self::Mock(c) => c.fetch_companions(subsource).await,
        }
    }
}
