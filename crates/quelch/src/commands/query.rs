use serde_json::Value;

use crate::config::Config;
use crate::mcp::tools::{OrderBy, SortDir};

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

pub async fn run(_config: &Config, _options: QueryOptions) -> anyhow::Result<()> {
    todo!("phase 7: rewire `quelch query` against the new instances schema")
}

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
