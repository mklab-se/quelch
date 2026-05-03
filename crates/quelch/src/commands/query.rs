//! `quelch query` — structured query against a Cosmos-backed data source.
//!
//! CLI wrapper around [`crate::mcp::tools::query::run`].  Resolves the MCP
//! deployment from config, builds the expose resolver, and calls the shared
//! tool function directly.

use serde_json::Value;

use crate::config::{Config, DeploymentRole};
use crate::cosmos::factory::build_cosmos_backend;
use crate::mcp::expose::ExposeResolver;
use crate::mcp::tools::query::{self, QueryRequest};
use crate::mcp::tools::{OrderBy, SortDir};

/// Options for `quelch query`.
#[derive(Debug)]
pub struct QueryOptions {
    /// Logical data-source name (e.g. `jira_issues`).
    pub data_source: String,
    /// Optional structured filter (parsed from `--where` JSON).
    pub where_: Option<Value>,
    /// Sort order clauses.
    pub order_by: Vec<OrderBy>,
    /// Maximum documents per page.
    pub top: usize,
    /// Pagination cursor from a prior response.
    pub cursor: Option<String>,
    /// Return only the document count.
    pub count_only: bool,
    /// Include soft-deleted documents.
    pub include_deleted: bool,
    /// Emit machine-readable JSON instead of formatted output.
    pub json: bool,
}

/// Run `quelch query`.
pub async fn run(config: &Config, options: QueryOptions) -> anyhow::Result<()> {
    let cosmos = build_cosmos_backend(config).await?;

    // Find the first MCP deployment to derive the expose resolver.
    let deployment_name = config
        .deployments
        .iter()
        .find(|d| d.role == DeploymentRole::Mcp)
        .map(|d| d.name.clone())
        .ok_or_else(|| {
            anyhow::anyhow!("no MCP deployment in config; ad-hoc queries require one")
        })?;

    let sliced = crate::config::slice::for_deployment(config, &deployment_name)?;
    let expose = ExposeResolver::from_sliced(&sliced, &deployment_name)
        .map_err(|e| anyhow::anyhow!("expose resolver: {e}"))?;

    let req = QueryRequest {
        data_source: options.data_source,
        r#where: options.where_,
        order_by: if options.order_by.is_empty() {
            None
        } else {
            Some(options.order_by)
        },
        top: options.top,
        cursor: options.cursor,
        count_only: options.count_only,
        include_deleted: options.include_deleted,
    };

    let resp = query::run(cosmos.as_ref(), &expose, req)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if options.json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    } else {
        println!("Total: {}", resp.total);
        if !resp.items.is_empty() {
            println!();
            for item in &resp.items {
                if let Some(link) = item.get("source_link").and_then(Value::as_str) {
                    print!("• {link}");
                } else if let Some(id) = item.get("id").and_then(Value::as_str) {
                    print!("• {id}");
                }
                if let Some(summary) = item
                    .get("summary")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("title").and_then(Value::as_str))
                {
                    print!(" — {summary}");
                }
                println!();
            }
        }
        if let Some(cursor) = &resp.next_cursor {
            println!();
            println!("More results available. Continue with --cursor {cursor}");
        }
    }

    Ok(())
}

/// Parse a `field:dir` string (e.g. `updated:desc`) into an [`OrderBy`].
pub fn parse_order_by(s: &str) -> anyhow::Result<OrderBy> {
    let (field, dir) = match s.split_once(':') {
        Some((f, d)) => (f, d),
        None => (s, "asc"),
    };
    let dir = match dir.to_lowercase().as_str() {
        "asc" => SortDir::Asc,
        "desc" => SortDir::Desc,
        other => anyhow::bail!("unknown sort direction '{other}'; use 'asc' or 'desc'"),
    };
    Ok(OrderBy {
        field: field.to_string(),
        dir,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosmos::InMemoryCosmos;
    use crate::mcp::tools::test_helpers::{
        build_cosmos_with_jira_issues, build_expose_jira_issues,
    };
    use serde_json::json;

    #[test]
    fn parse_order_by_desc() {
        let ob = parse_order_by("updated:desc").unwrap();
        assert_eq!(ob.field, "updated");
        assert!(matches!(ob.dir, SortDir::Desc));
    }

    #[test]
    fn parse_order_by_defaults_to_asc() {
        let ob = parse_order_by("name").unwrap();
        assert_eq!(ob.field, "name");
        assert!(matches!(ob.dir, SortDir::Asc));
    }

    #[test]
    fn parse_order_by_unknown_dir_errors() {
        assert!(parse_order_by("name:sideways").is_err());
    }

    #[tokio::test]
    async fn query_dispatches_to_query_tool() {
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

        let resp = query::run(&cosmos, &expose, req).await.unwrap();
        // 3 Open, non-deleted docs
        assert_eq!(resp.total, 3);
    }

    #[tokio::test]
    async fn query_count_only_returns_total() {
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

        let resp = query::run(&cosmos, &expose, req).await.unwrap();
        assert!(resp.items.is_empty(), "count_only should produce no items");
        assert_eq!(resp.total, 5);
    }
}
