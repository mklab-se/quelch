//! MCP `aggregate` tool — count and sum grouped query results.
//!
//! # Array field fan-out
//!
//! When `group_by` names an array field (e.g. `labels` on `jira_issue`), the
//! tool would ideally emit a Cosmos SQL `JOIN v IN c.labels … GROUP BY v` to
//! fan out each array element into its own group.  However, `InMemoryCosmos`
//! does not support JOIN or GROUP BY syntax.
//!
//! **Current behaviour**: the aggregate tool applies grouping in-process after
//! a full-table scan.  This is semantically correct and works against both
//! `InMemoryCosmos` and the real Cosmos SDK (which would need the JOIN form for
//! large datasets — see the TODO below).
//!
//! Array field detection uses [`crate::mcp::schema::SchemaCatalog`].  If the
//! catalog lists `group_by` as an array field, element-level fan-out is
//! performed.
//!
//! TODO(perf): for large Cosmos containers, emit the JOIN-based
//! GROUP BY SQL to push the aggregation down to the server.  The in-process
//! approach works correctly but doesn't scale.

use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cosmos::CosmosBackend;
use crate::mcp::error::McpError;
use crate::mcp::expose::ExposeResolver;
use crate::mcp::filter::{cosmos_sql::SqlBuilder, parse};
use crate::mcp::schema::SchemaCatalog;

/// Request parameters for the `aggregate` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AggregateRequest {
    /// Logical data-source name.
    pub data_source: String,
    /// Optional filter (JSON filter grammar) applied before aggregation.
    #[serde(rename = "where")]
    pub r#where: Option<Value>,
    /// Field to group by (scalar or array).
    pub group_by: Option<String>,
    /// Include a document count per group (default: `true`).
    #[serde(default = "default_count_true")]
    pub count: bool,
    /// Sum this numeric field per group.
    pub sum_field: Option<String>,
    /// Limit to the top N groups by count.
    pub top_groups: Option<usize>,
    /// When `true`, include soft-deleted documents.
    #[serde(default)]
    pub include_deleted: bool,
}

fn default_count_true() -> bool {
    true
}

/// A single aggregation group.
#[derive(Debug, Serialize)]
pub struct AggregateGroup {
    /// The group key value (e.g. `"Open"`, `"backend"`).  `None` for the
    /// no-`group_by` total row.
    pub key: Option<String>,
    /// Document count for this group.
    pub count: u64,
    /// Sum of `sum_field` for this group.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sum: Option<f64>,
}

/// Overall totals (across all groups).
#[derive(Debug, Serialize)]
pub struct AggregateTotal {
    pub count: u64,
    pub sum: Option<f64>,
}

/// Response from the `aggregate` tool.
#[derive(Debug, Serialize)]
pub struct AggregateResponse {
    pub groups: Vec<AggregateGroup>,
    pub total: AggregateTotal,
}

/// Execute the `aggregate` tool.
pub async fn run(
    cosmos: &dyn CosmosBackend,
    expose: &ExposeResolver,
    schema: &SchemaCatalog,
    req: AggregateRequest,
) -> Result<AggregateResponse, McpError> {
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

    // Build the WHERE clause
    let mut where_sql = String::new();
    let mut params: Vec<(String, Value)> = Vec::new();

    if let Some(uf) = user_filter {
        where_sql = format!(" WHERE {}", uf.sql_fragment);
        params.extend(uf.params);
    } else if !req.include_deleted {
        where_sql = " WHERE (NOT IS_DEFINED(c._deleted) OR c._deleted = false)".to_string();
    }

    // TODO(multi-container): fan out across all backing containers when a data
    let container = &resolved.backed_by[0].container;

    // Fetch all matching documents for in-process aggregation.
    // This avoids the need for GROUP BY / JOIN syntax that InMemoryCosmos doesn't support.
    let sql = format!("SELECT * FROM c{where_sql}");
    let mut stream = cosmos.query(container, &sql, params).await?;

    // Collect all docs
    let mut all_docs: Vec<Value> = Vec::new();
    while let Some(page) = stream.next_page().await? {
        all_docs.extend(page);
    }

    // Determine whether group_by targets an array field
    let is_array_group = req
        .group_by
        .as_deref()
        .map(|f| schema.is_array_field(&resolved.kind, f))
        .unwrap_or(false);

    let groups = aggregate_in_process(&all_docs, &req, is_array_group);

    // Apply top_groups limit
    let mut groups = groups;
    if let Some(top) = req.top_groups {
        // Sort by count descending before truncating
        groups.sort_by_key(|b| std::cmp::Reverse(b.count));
        groups.truncate(top);
    }

    let total_count = groups.iter().map(|g| g.count).sum();
    let total_sum = if req.sum_field.is_some() {
        Some(groups.iter().filter_map(|g| g.sum).sum())
    } else {
        None
    };

    Ok(AggregateResponse {
        groups,
        total: AggregateTotal {
            count: total_count,
            sum: total_sum,
        },
    })
}

/// Perform grouping and aggregation in-process over a slice of documents.
fn aggregate_in_process(
    docs: &[Value],
    req: &AggregateRequest,
    is_array_group: bool,
) -> Vec<AggregateGroup> {
    if req.group_by.is_none() {
        // No grouping — return a single total group.
        let count = docs.len() as u64;
        let sum = req.sum_field.as_deref().map(|f| {
            docs.iter()
                .filter_map(|d| d.get(f).and_then(Value::as_f64))
                .sum::<f64>()
        });
        return vec![AggregateGroup {
            key: None,
            count,
            sum,
        }];
    }

    let group_field = req.group_by.as_deref().unwrap();
    let sum_field = req.sum_field.as_deref();

    // key → (count, sum)
    let mut groups: HashMap<String, (u64, f64)> = HashMap::new();

    for doc in docs {
        if is_array_group {
            // Fan out: each element of the array contributes to its own group
            if let Some(arr) = doc.get(group_field).and_then(Value::as_array) {
                for elem in arr {
                    let key = value_to_group_key(elem);
                    let sum_val = sum_field
                        .and_then(|f| doc.get(f))
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0);
                    let entry = groups.entry(key).or_insert((0, 0.0));
                    entry.0 += 1;
                    entry.1 += sum_val;
                }
            }
            // Documents without the array field are excluded from array fan-out groups
        } else {
            // Scalar grouping
            let key = doc
                .get(group_field)
                .map(value_to_group_key)
                .unwrap_or_else(|| "<null>".to_string());
            let sum_val = sum_field
                .and_then(|f| doc.get(f))
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let entry = groups.entry(key).or_insert((0, 0.0));
            entry.0 += 1;
            entry.1 += sum_val;
        }
    }

    groups
        .into_iter()
        .map(|(key, (count, sum))| AggregateGroup {
            key: Some(key),
            count,
            sum: req.sum_field.as_ref().map(|_| sum),
        })
        .collect()
}

/// Convert a JSON value to a string group key.
fn value_to_group_key(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "<null>".to_string(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosmos::InMemoryCosmos;
    use crate::mcp::schema::SchemaCatalog;
    use crate::mcp::tools::test_helpers::{
        build_cosmos_with_jira_issues, build_expose_jira_issues,
    };
    use serde_json::json;

    fn schema() -> SchemaCatalog {
        SchemaCatalog::new()
    }

    #[tokio::test]
    async fn aggregate_count_with_filter() {
        let cosmos = build_cosmos_with_jira_issues().await;
        let expose = build_expose_jira_issues();
        let req = AggregateRequest {
            data_source: "jira_issues".into(),
            r#where: Some(json!({"status": "Open"})),
            group_by: None,
            count: true,
            sum_field: None,
            top_groups: None,
            include_deleted: false,
        };
        let resp = run(&cosmos, &expose, &schema(), req).await.unwrap();
        assert_eq!(resp.total.count, 3); // 3 Open, non-deleted
        assert_eq!(resp.groups.len(), 1);
        assert_eq!(resp.groups[0].count, 3);
    }

    #[tokio::test]
    async fn aggregate_group_by_scalar() {
        let cosmos = build_cosmos_with_jira_issues().await;
        let expose = build_expose_jira_issues();
        let req = AggregateRequest {
            data_source: "jira_issues".into(),
            r#where: None,
            group_by: Some("status".into()),
            count: true,
            sum_field: None,
            top_groups: None,
            include_deleted: false,
        };
        let resp = run(&cosmos, &expose, &schema(), req).await.unwrap();
        // 3 Open + 2 Done (i6 is soft-deleted)
        let open_group = resp
            .groups
            .iter()
            .find(|g| g.key.as_deref() == Some("Open"))
            .unwrap();
        let done_group = resp
            .groups
            .iter()
            .find(|g| g.key.as_deref() == Some("Done"))
            .unwrap();
        assert_eq!(open_group.count, 3);
        assert_eq!(done_group.count, 2);
    }

    #[tokio::test]
    async fn aggregate_group_by_array_field_fans_out() {
        // All 5 non-deleted docs (i1-i5) have labels: ["backend"]
        // i6 has labels: ["backend","frontend"] but is soft-deleted
        // After soft-delete filter: 5 docs contribute "backend", frontend excluded
        let cosmos = build_cosmos_with_jira_issues().await;
        let expose = build_expose_jira_issues();
        let req = AggregateRequest {
            data_source: "jira_issues".into(),
            r#where: None,
            group_by: Some("labels".into()),
            count: true,
            sum_field: None,
            top_groups: None,
            include_deleted: false,
        };
        let resp = run(&cosmos, &expose, &schema(), req).await.unwrap();
        // "backend" should count all 5 non-deleted docs
        let backend_group = resp
            .groups
            .iter()
            .find(|g| g.key.as_deref() == Some("backend"))
            .unwrap();
        assert_eq!(
            backend_group.count, 5,
            "backend should count all 5 non-deleted docs"
        );
        // i6 is soft-deleted; frontend should not appear
        assert!(
            resp.groups
                .iter()
                .all(|g| g.key.as_deref() != Some("frontend")),
            "frontend from soft-deleted doc should not appear"
        );
    }

    #[tokio::test]
    async fn aggregate_group_by_array_field_with_include_deleted() {
        // With include_deleted: true, i6's labels ["backend","frontend"] should be counted
        let cosmos = build_cosmos_with_jira_issues().await;
        let expose = build_expose_jira_issues();
        let req = AggregateRequest {
            data_source: "jira_issues".into(),
            r#where: None,
            group_by: Some("labels".into()),
            count: true,
            sum_field: None,
            top_groups: None,
            include_deleted: true,
        };
        let resp = run(&cosmos, &expose, &schema(), req).await.unwrap();
        let frontend_group = resp
            .groups
            .iter()
            .find(|g| g.key.as_deref() == Some("frontend"));
        assert!(
            frontend_group.is_some(),
            "frontend from i6 should appear with include_deleted=true"
        );
        let backend_group = resp
            .groups
            .iter()
            .find(|g| g.key.as_deref() == Some("backend"))
            .unwrap();
        assert_eq!(backend_group.count, 6, "backend from all 6 docs");
    }

    #[tokio::test]
    async fn aggregate_top_groups_limits() {
        let cosmos = build_cosmos_with_jira_issues().await;
        let expose = build_expose_jira_issues();
        let req = AggregateRequest {
            data_source: "jira_issues".into(),
            r#where: None,
            group_by: Some("status".into()),
            count: true,
            sum_field: None,
            top_groups: Some(1),
            include_deleted: false,
        };
        let resp = run(&cosmos, &expose, &schema(), req).await.unwrap();
        assert_eq!(resp.groups.len(), 1);
        // The top group by count should be "Open" (3 > 2)
        assert_eq!(resp.groups[0].key.as_deref(), Some("Open"));
    }

    #[tokio::test]
    async fn aggregate_sum_field() {
        let cosmos = InMemoryCosmos::new();
        cosmos
            .upsert(
                "jira-issues",
                json!({"id": "a", "_partition_key": "DO", "status": "Open", "story_points": 3.0}),
            )
            .await
            .unwrap();
        cosmos
            .upsert(
                "jira-issues",
                json!({"id": "b", "_partition_key": "DO", "status": "Open", "story_points": 5.0}),
            )
            .await
            .unwrap();
        cosmos
            .upsert(
                "jira-issues",
                json!({"id": "c", "_partition_key": "DO", "status": "Done", "story_points": 2.0}),
            )
            .await
            .unwrap();

        let expose = build_expose_jira_issues();
        let req = AggregateRequest {
            data_source: "jira_issues".into(),
            r#where: None,
            group_by: Some("status".into()),
            count: true,
            sum_field: Some("story_points".into()),
            top_groups: None,
            include_deleted: false,
        };
        let resp = run(&cosmos, &expose, &schema(), req).await.unwrap();
        let open = resp
            .groups
            .iter()
            .find(|g| g.key.as_deref() == Some("Open"))
            .unwrap();
        assert_eq!(open.sum, Some(8.0));
        let done = resp
            .groups
            .iter()
            .find(|g| g.key.as_deref() == Some("Done"))
            .unwrap();
        assert_eq!(done.sum, Some(2.0));
    }

    #[tokio::test]
    async fn aggregate_excludes_soft_deleted() {
        let cosmos = build_cosmos_with_jira_issues().await;
        let expose = build_expose_jira_issues();
        let req = AggregateRequest {
            data_source: "jira_issues".into(),
            r#where: None,
            group_by: None,
            count: true,
            sum_field: None,
            top_groups: None,
            include_deleted: false,
        };
        let resp = run(&cosmos, &expose, &schema(), req).await.unwrap();
        // 5 non-deleted docs out of 6
        assert_eq!(resp.total.count, 5);
    }

    #[tokio::test]
    async fn aggregate_forbidden_for_unexposed_data_source() {
        let cosmos = build_cosmos_with_jira_issues().await;
        let expose = build_expose_jira_issues();
        let req = AggregateRequest {
            data_source: "jira_sprints".into(),
            r#where: None,
            group_by: None,
            count: true,
            sum_field: None,
            top_groups: None,
            include_deleted: false,
        };
        let err = run(&cosmos, &expose, &schema(), req).await.unwrap_err();
        assert!(matches!(err, McpError::Forbidden(_)));
    }

    #[tokio::test]
    async fn aggregate_include_deleted_includes_tombstones() {
        let cosmos = build_cosmos_with_jira_issues().await;
        let expose = build_expose_jira_issues();
        let req = AggregateRequest {
            data_source: "jira_issues".into(),
            r#where: None,
            group_by: None,
            count: true,
            sum_field: None,
            top_groups: None,
            include_deleted: true,
        };
        let resp = run(&cosmos, &expose, &schema(), req).await.unwrap();
        assert_eq!(resp.total.count, 6);
    }
}
