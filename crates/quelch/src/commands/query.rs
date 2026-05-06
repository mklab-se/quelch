//! `quelch query` — structured query against a Cosmos-backed data source.
//!
//! Operator command. Picks the first MCP instance in the config (so that
//! the same `expose:` rules apply that an agent would see), builds a Cosmos
//! client, and calls the `query` tool implementation.

use serde_json::Value;

use crate::config::Config;
use crate::config::schema::InstanceKind;
use crate::cosmos::factory::build_cosmos_backend;
use crate::mcp::expose::ExposeResolver;
use crate::mcp::tools::query::{QueryRequest, run as query_run};
use crate::mcp::tools::{OrderBy, SortDir};

/// Options for `quelch query`.
#[derive(Debug)]
pub struct QueryOptions {
    pub data_source: String,
    pub where_: Option<Value>,
    pub order_by: Vec<OrderBy>,
    pub top: usize,
    pub cursor: Option<String>,
    pub count_only: bool,
    pub include_deleted: bool,
    pub json: bool,
}

/// Run `quelch query`.
pub async fn run(config: &Config, options: QueryOptions) -> anyhow::Result<()> {
    let mcp_instance_name = pick_mcp_instance_name(config)?;
    let sliced = crate::config::slice::slice_for_instance(config, &mcp_instance_name)?;
    let cosmos = build_cosmos_backend(&sliced).await?;
    let expose = ExposeResolver::from_sliced(&sliced, &mcp_instance_name)
        .map_err(|e| anyhow::anyhow!("expose resolver: {e}"))?;

    let order_by = if options.order_by.is_empty() {
        None
    } else {
        Some(options.order_by)
    };

    let req = QueryRequest {
        data_source: options.data_source,
        r#where: options.where_,
        order_by,
        top: options.top,
        cursor: options.cursor,
        count_only: options.count_only,
        include_deleted: options.include_deleted,
    };

    let resp = query_run(cosmos.as_ref(), &expose, req)
        .await
        .map_err(|e| anyhow::anyhow!("query: {e}"))?;

    if options.json {
        let payload = serde_json::json!({
            "items": resp.items,
            "next_cursor": resp.next_cursor,
            "total": resp.total,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("{} document(s)", resp.total);
        for item in &resp.items {
            println!("{}", serde_json::to_string(item)?);
        }
        if let Some(c) = &resp.next_cursor {
            println!("(next cursor: {c})");
        }
    }

    Ok(())
}

/// Pick the first MCP instance, or error if none exists.
fn pick_mcp_instance_name(config: &Config) -> anyhow::Result<String> {
    config
        .instances
        .iter()
        .find(|i| i.kind() == InstanceKind::Mcp)
        .map(|i| i.name.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "quelch query requires an MCP instance in the config (none found in instances:)"
            )
        })
}

/// Parse a `field:dir` order-by string into an [`OrderBy`].
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::tools::query::{self, QueryRequest};
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
