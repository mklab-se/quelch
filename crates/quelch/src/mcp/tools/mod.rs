//! MCP tool request/response types and handler implementations.

pub mod aggregate;
pub mod get;
pub mod list_sources;
pub mod query;
pub mod search;
pub mod search_api;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Ordering direction.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SortDir {
    Asc,
    Desc,
}

/// A single `ORDER BY` clause field + direction.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct OrderBy {
    pub field: String,
    pub dir: SortDir,
}

// ---------------------------------------------------------------------------
// Shared test helpers (available in #[cfg(test)])
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod test_helpers {
    use std::collections::HashMap;

    use serde_json::{Value, json};

    use crate::config::BackedBy;
    use crate::config::data_sources::ResolvedDataSource;
    use crate::cosmos::{CosmosBackend, InMemoryCosmos};
    use crate::mcp::expose::ExposeResolver;

    /// Build an `ExposeResolver` that exposes `jira_issues` → `jira-issues` container.
    pub fn build_expose_jira_issues() -> ExposeResolver {
        let mut map = HashMap::new();
        map.insert(
            "jira_issues".to_string(),
            ResolvedDataSource {
                kind: "jira_issue".to_string(),
                backed_by: vec![BackedBy {
                    container: "jira-issues".to_string(),
                }],
            },
        );
        ExposeResolver::from_map(map)
    }

    /// Build an `ExposeResolver` that exposes multiple sources.
    pub fn build_expose(sources: &[(&str, &str, &str)]) -> ExposeResolver {
        let mut map = HashMap::new();
        for (name, kind, container) in sources {
            map.insert(
                name.to_string(),
                ResolvedDataSource {
                    kind: kind.to_string(),
                    backed_by: vec![BackedBy {
                        container: container.to_string(),
                    }],
                },
            );
        }
        ExposeResolver::from_map(map)
    }

    /// Build an `InMemoryCosmos` pre-populated with Jira issue documents.
    ///
    /// Documents returned:
    /// - 3 Open stories (IDs: i1, i2, i3)
    /// - 2 Done stories (IDs: i4, i5)
    /// - 1 soft-deleted Open story (ID: i6, `_deleted: true`)
    pub async fn build_cosmos_with_jira_issues() -> InMemoryCosmos {
        let cosmos = InMemoryCosmos::new();
        let container = "jira-issues";

        for (id, status, deleted) in &[
            ("i1", "Open", false),
            ("i2", "Open", false),
            ("i3", "Open", false),
            ("i4", "Done", false),
            ("i5", "Done", false),
        ] {
            let mut doc = json!({
                "id": id,
                "_partition_key": "DO",
                "status": status,
                "type": "Story",
                "labels": ["backend"],
            });
            if *deleted {
                doc["_deleted"] = json!(false);
            }
            cosmos.upsert(container, doc).await.unwrap();
        }

        // One soft-deleted document
        cosmos
            .upsert(
                container,
                json!({
                    "id": "i6",
                    "_partition_key": "DO",
                    "status": "Open",
                    "type": "Story",
                    "_deleted": true,
                    "labels": ["backend", "frontend"],
                }),
            )
            .await
            .unwrap();

        cosmos
    }

    /// A single document for point-read tests.
    pub async fn build_cosmos_with_single_doc(container: &str, doc: Value) -> InMemoryCosmos {
        let cosmos = InMemoryCosmos::new();
        cosmos.upsert(container, doc).await.unwrap();
        cosmos
    }
}
