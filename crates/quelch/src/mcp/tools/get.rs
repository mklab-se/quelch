//! MCP `get` tool — fetch a single document by ID.
//!
//! Tries each backing container in order and returns the first match.
//! When `include_deleted: false` (the default), soft-deleted documents are
//! silently skipped as if they did not exist.
//!
//! # Partition-key strategy
//!
//! The real Cosmos SDK requires a partition key for a point-read. Since the MCP
//! caller only provides the document ID, we use a cross-partition SQL query
//! (`WHERE c.id = @id`) which is slower but always correct. Production
//! deployments with high throughput may want to encode the partition key into
//! the ID and parse it back, but that optimisation is out of scope for now.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cosmos::CosmosBackend;
use crate::mcp::error::McpError;
use crate::mcp::expose::ExposeResolver;

/// Request parameters for the `get` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetRequest {
    /// Logical data-source name.
    pub data_source: String,
    /// Document ID to look up.
    pub id: String,
    /// When `true`, return soft-deleted documents too.
    #[serde(default)]
    pub include_deleted: bool,
}

/// Response from the `get` tool.
#[derive(Debug, Serialize)]
pub struct GetResponse {
    /// The matching document, or `null` if not found (or soft-deleted).
    pub document: Option<Value>,
}

/// Execute the `get` tool.
pub async fn run(
    cosmos: &dyn CosmosBackend,
    expose: &ExposeResolver,
    req: GetRequest,
) -> Result<GetResponse, McpError> {
    let resolved = expose.resolve(&req.data_source)?;

    // Try each backing container until the document is found (or exhausted).
    for backing in &resolved.backed_by {
        let sql = "SELECT * FROM c WHERE c.id = @id";
        let params = vec![("@id".to_string(), Value::String(req.id.clone()))];

        let mut stream = cosmos.query(&backing.container, sql, params).await?;
        if let Some(page) = stream.next_page().await? {
            for doc in page {
                let is_deleted = doc
                    .get("_deleted")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if is_deleted && !req.include_deleted {
                    continue;
                }
                return Ok(GetResponse {
                    document: Some(doc),
                });
            }
        }
    }

    Ok(GetResponse { document: None })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosmos::InMemoryCosmos;
    use crate::mcp::tools::test_helpers::build_expose_jira_issues;
    use serde_json::json;

    async fn make_cosmos_with_doc(id: &str, deleted: bool) -> InMemoryCosmos {
        let cosmos = InMemoryCosmos::new();
        let mut doc = json!({
            "id": id,
            "_partition_key": "DO",
            "status": "Open",
        });
        if deleted {
            doc["_deleted"] = json!(true);
        }
        cosmos.upsert("jira-issues", doc).await.unwrap();
        cosmos
    }

    #[tokio::test]
    async fn get_returns_document_when_found() {
        let cosmos = make_cosmos_with_doc("DO-1", false).await;
        let expose = build_expose_jira_issues();
        let req = GetRequest {
            data_source: "jira_issues".into(),
            id: "DO-1".into(),
            include_deleted: false,
        };
        let resp = run(&cosmos, &expose, req).await.unwrap();
        assert!(resp.document.is_some());
        assert_eq!(resp.document.unwrap()["id"], "DO-1");
    }

    #[tokio::test]
    async fn get_returns_none_for_missing_document() {
        let cosmos = make_cosmos_with_doc("DO-1", false).await;
        let expose = build_expose_jira_issues();
        let req = GetRequest {
            data_source: "jira_issues".into(),
            id: "DO-9999".into(),
            include_deleted: false,
        };
        let resp = run(&cosmos, &expose, req).await.unwrap();
        assert!(resp.document.is_none());
    }

    #[tokio::test]
    async fn get_returns_null_for_soft_deleted_by_default() {
        let cosmos = make_cosmos_with_doc("DO-2", true).await;
        let expose = build_expose_jira_issues();
        let req = GetRequest {
            data_source: "jira_issues".into(),
            id: "DO-2".into(),
            include_deleted: false,
        };
        let resp = run(&cosmos, &expose, req).await.unwrap();
        assert!(
            resp.document.is_none(),
            "soft-deleted doc should not be returned"
        );
    }

    #[tokio::test]
    async fn get_returns_soft_deleted_when_include_deleted_set() {
        let cosmos = make_cosmos_with_doc("DO-2", true).await;
        let expose = build_expose_jira_issues();
        let req = GetRequest {
            data_source: "jira_issues".into(),
            id: "DO-2".into(),
            include_deleted: true,
        };
        let resp = run(&cosmos, &expose, req).await.unwrap();
        assert!(
            resp.document.is_some(),
            "soft-deleted doc should be returned with include_deleted=true"
        );
    }

    #[tokio::test]
    async fn get_forbidden_for_unexposed_data_source() {
        let cosmos = make_cosmos_with_doc("DO-1", false).await;
        let expose = build_expose_jira_issues(); // only exposes jira_issues
        let req = GetRequest {
            data_source: "confluence_pages".into(),
            id: "some-id".into(),
            include_deleted: false,
        };
        let err = run(&cosmos, &expose, req).await.unwrap_err();
        assert!(matches!(err, McpError::Forbidden(_)));
    }

    #[tokio::test]
    async fn get_searches_multiple_containers() {
        // Doc lives in a second backing container
        let cosmos = InMemoryCosmos::new();
        cosmos
            .upsert(
                "jira-issues-2",
                json!({"id": "DO-5", "_partition_key": "DO", "status": "Done"}),
            )
            .await
            .unwrap();
        use crate::config::{BackedBy, data_sources::ResolvedDataSource};
        use crate::mcp::expose::ExposeResolver;
        use std::collections::HashMap;
        let mut map = HashMap::new();
        map.insert(
            "jira_issues".to_string(),
            ResolvedDataSource {
                kind: "jira_issue".to_string(),
                backed_by: vec![
                    BackedBy {
                        container: "jira-issues-1".to_string(),
                    },
                    BackedBy {
                        container: "jira-issues-2".to_string(),
                    },
                ],
            },
        );
        let expose = ExposeResolver::from_map(map);
        let req = GetRequest {
            data_source: "jira_issues".into(),
            id: "DO-5".into(),
            include_deleted: false,
        };
        let resp = run(&cosmos, &expose, req).await.unwrap();
        assert!(resp.document.is_some());
        assert_eq!(resp.document.unwrap()["id"], "DO-5");
    }
}
