//! MCP `query` tool — exhaustive structured query against a Cosmos-backed data source.
//!
//! # Soft-delete
//!
//! By default (`include_deleted: false`) the tool appends:
//! ```sql
//! AND (NOT IS_DEFINED(c._deleted) OR c._deleted = false)
//! ```
//! Pass `include_deleted: true` to include tombstoned documents.
//!
//! # Pagination
//!
//! Use `top` (default 50) to limit page size and `cursor` (the `next_cursor`
//! from a prior response) to iterate.  The real Azure backend returns a
//! continuation token; the in-memory backend always returns a single page.
//!
//! # Multi-container fan-out
//!
//! TODO(quelch v2 follow-up): implement proper multi-container fan-out with
//! merged cursors. Currently only the first backing container is queried.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cosmos::CosmosBackend;
use crate::mcp::error::McpError;
use crate::mcp::expose::ExposeResolver;
use crate::mcp::filter::{cosmos_sql::SqlBuilder, parse};

use super::{OrderBy, SortDir};

/// Request parameters for the `query` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct QueryRequest {
    /// Logical data-source name (as exposed by the deployment).
    pub data_source: String,
    /// Optional filter expression (JSON filter grammar).
    #[serde(rename = "where")]
    pub r#where: Option<Value>,
    /// Sort order.
    pub order_by: Option<Vec<OrderBy>>,
    /// Maximum number of documents to return in this page (default: 50).
    #[serde(default = "default_top")]
    pub top: usize,
    /// Pagination cursor from a previous response.
    pub cursor: Option<String>,
    /// When `true`, return only `total` (items will be empty).
    #[serde(default)]
    pub count_only: bool,
    /// When `true`, include soft-deleted documents.
    #[serde(default)]
    pub include_deleted: bool,
}

fn default_top() -> usize {
    50
}

/// Response from the `query` tool.
#[derive(Debug, Serialize)]
pub struct QueryResponse {
    /// Matching documents (empty when `count_only: true`).
    pub items: Vec<Value>,
    /// Pagination cursor for the next page, or `None` when exhausted.
    pub next_cursor: Option<String>,
    /// Total number of matching documents.
    pub total: u64,
}

/// Execute the `query` tool.
pub async fn run(
    cosmos: &dyn CosmosBackend,
    expose: &ExposeResolver,
    req: QueryRequest,
) -> Result<QueryResponse, McpError> {
    let resolved = expose.resolve(&req.data_source)?;

    // Parse and translate the `where` filter.
    let where_ast = match req.r#where {
        Some(ref v) => Some(parse(v)?),
        None => None,
    };

    let builder = SqlBuilder::new(req.include_deleted);
    let user_filter = match &where_ast {
        Some(w) => Some(builder.build(w)?),
        None => None,
    };

    // Build SQL: SELECT * FROM c [WHERE ...] [ORDER BY ...]
    let mut sql = String::from("SELECT * FROM c");
    let mut params: Vec<(String, Value)> = Vec::new();

    if let Some(uf) = user_filter {
        sql.push_str(" WHERE ");
        sql.push_str(&uf.sql_fragment);
        params.extend(uf.params);
    } else if !req.include_deleted {
        // No user filter but still need soft-delete predicate.
        sql.push_str(" WHERE (NOT IS_DEFINED(c._deleted) OR c._deleted = false)");
    }

    if let Some(orderings) = &req.order_by {
        sql.push_str(" ORDER BY ");
        let parts: Vec<String> = orderings
            .iter()
            .map(|o| {
                format!(
                    "c.{} {}",
                    o.field,
                    match o.dir {
                        SortDir::Asc => "ASC",
                        SortDir::Desc => "DESC",
                    }
                )
            })
            .collect();
        sql.push_str(&parts.join(", "));
    }

    // TODO(quelch v2 follow-up): multi-container fan-out with merged cursors.
    // For now, query only the first backing container.
    let container = &resolved.backed_by[0].container;

    if req.count_only {
        // Rewrite as COUNT query, stripping ORDER BY.
        let count_sql = if let Some(idx) = sql.find(" WHERE ") {
            format!("SELECT VALUE COUNT(1) FROM c{}", &sql[idx..])
        } else {
            "SELECT VALUE COUNT(1) FROM c".to_string()
        };
        // Strip ORDER BY from count query.
        let count_sql = if let Some(o_idx) = count_sql.find(" ORDER BY") {
            count_sql[..o_idx].to_string()
        } else {
            count_sql
        };

        let mut stream = cosmos.query(container, &count_sql, params).await?;
        let total = if let Some(page) = stream.next_page().await? {
            page.first().and_then(Value::as_u64).unwrap_or(0)
        } else {
            0
        };
        return Ok(QueryResponse {
            items: vec![],
            next_cursor: None,
            total,
        });
    }

    let mut stream = cosmos.query(container, &sql, params).await?;

    let mut items = Vec::new();
    let mut total: u64 = 0;
    if let Some(page) = stream.next_page().await? {
        total = page.len() as u64;
        items.extend(page.into_iter().take(req.top));
    }

    let next_cursor = stream.continuation_token().map(String::from);

    Ok(QueryResponse {
        items,
        next_cursor,
        total,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::tools::test_helpers::{
        build_cosmos_with_jira_issues, build_expose, build_expose_jira_issues,
    };
    use serde_json::json;

    #[tokio::test]
    async fn query_returns_matching_docs() {
        let cosmos = build_cosmos_with_jira_issues().await;
        let expose = build_expose_jira_issues();
        let req = QueryRequest {
            data_source: "jira_issues".into(),
            r#where: Some(json!({"status": "Open"})),
            order_by: None,
            top: 50,
            cursor: None,
            count_only: false,
            include_deleted: false,
        };
        let result = run(&cosmos, &expose, req).await.unwrap();
        // 3 Open docs (i1, i2, i3) — i6 is Open but soft-deleted
        assert_eq!(result.total, 3);
        assert_eq!(result.items.len(), 3);
    }

    #[tokio::test]
    async fn query_excludes_soft_deleted_by_default() {
        let cosmos = build_cosmos_with_jira_issues().await;
        let expose = build_expose_jira_issues();
        // No filter — should get all non-deleted docs (5 out of 6)
        let req = QueryRequest {
            data_source: "jira_issues".into(),
            r#where: None,
            order_by: None,
            top: 50,
            cursor: None,
            count_only: false,
            include_deleted: false,
        };
        let result = run(&cosmos, &expose, req).await.unwrap();
        assert_eq!(result.total, 5);
        let ids: Vec<&str> = result
            .items
            .iter()
            .map(|d| d["id"].as_str().unwrap())
            .collect();
        assert!(!ids.contains(&"i6"), "soft-deleted i6 should be excluded");
    }

    #[tokio::test]
    async fn query_with_include_deleted_returns_tombstoned() {
        let cosmos = build_cosmos_with_jira_issues().await;
        let expose = build_expose_jira_issues();
        let req = QueryRequest {
            data_source: "jira_issues".into(),
            r#where: None,
            order_by: None,
            top: 50,
            cursor: None,
            count_only: false,
            include_deleted: true,
        };
        let result = run(&cosmos, &expose, req).await.unwrap();
        assert_eq!(result.total, 6);
        let ids: Vec<&str> = result
            .items
            .iter()
            .map(|d| d["id"].as_str().unwrap())
            .collect();
        assert!(
            ids.contains(&"i6"),
            "i6 should be included when include_deleted=true"
        );
    }

    #[tokio::test]
    async fn query_returns_forbidden_for_unexposed_data_source() {
        let cosmos = build_cosmos_with_jira_issues().await;
        let expose = build_expose_jira_issues(); // only exposes jira_issues
        let req = QueryRequest {
            data_source: "jira_sprints".into(),
            r#where: None,
            order_by: None,
            top: 50,
            cursor: None,
            count_only: false,
            include_deleted: false,
        };
        let err = run(&cosmos, &expose, req).await.unwrap_err();
        assert!(
            matches!(err, McpError::Forbidden(_)),
            "expected Forbidden, got {err:?}"
        );
    }

    #[tokio::test]
    async fn query_count_only_returns_only_total() {
        let cosmos = build_cosmos_with_jira_issues().await;
        let expose = build_expose_jira_issues();
        let req = QueryRequest {
            data_source: "jira_issues".into(),
            r#where: None,
            order_by: None,
            top: 50,
            cursor: None,
            count_only: true,
            include_deleted: false,
        };
        let result = run(&cosmos, &expose, req).await.unwrap();
        assert_eq!(result.total, 5); // 5 non-deleted docs
        assert!(
            result.items.is_empty(),
            "items should be empty for count_only"
        );
    }

    #[tokio::test]
    async fn query_top_limits_page_size() {
        let cosmos = build_cosmos_with_jira_issues().await;
        let expose = build_expose_jira_issues();
        let req = QueryRequest {
            data_source: "jira_issues".into(),
            r#where: None,
            order_by: None,
            top: 2,
            cursor: None,
            count_only: false,
            include_deleted: false,
        };
        let result = run(&cosmos, &expose, req).await.unwrap();
        // total reflects all matches, items capped at top
        assert_eq!(result.items.len(), 2);
        assert_eq!(result.total, 5);
    }

    #[tokio::test]
    async fn query_multiple_exposed_sources() {
        let cosmos = build_cosmos_with_jira_issues().await;
        let expose = build_expose(&[
            ("jira_issues", "jira_issue", "jira-issues"),
            ("jira_sprints", "jira_sprint", "jira-sprints"),
        ]);
        // Query jira_sprints (container empty) → 0 results
        let req = QueryRequest {
            data_source: "jira_sprints".into(),
            r#where: None,
            order_by: None,
            top: 50,
            cursor: None,
            count_only: false,
            include_deleted: false,
        };
        let result = run(&cosmos, &expose, req).await.unwrap();
        assert_eq!(result.total, 0);
    }
}
