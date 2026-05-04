//! `quelch get` — fetch a single document by ID from a Cosmos-backed data source.
//!
//! CLI wrapper around [`crate::mcp::tools::get::run`].

use serde_json::Value;

use crate::config::{Config, DeploymentRole};
use crate::cosmos::factory::build_cosmos_backend;
use crate::mcp::expose::ExposeResolver;
use crate::mcp::tools::get::{self, GetRequest};

/// Options for `quelch get`.
#[derive(Debug)]
pub struct GetOptions {
    /// Document ID.
    pub id: String,
    /// Logical data-source name.
    pub data_source: String,
    /// Include soft-deleted documents.
    pub include_deleted: bool,
    /// Emit machine-readable JSON instead of formatted output.
    pub json: bool,
}

/// Run `quelch get`.
pub async fn run(config: &Config, options: GetOptions) -> anyhow::Result<()> {
    let cosmos = build_cosmos_backend(config).await?;

    // Find the first MCP deployment to derive the expose resolver.
    let deployment_name = config
        .deployments
        .iter()
        .find(|d| d.role == DeploymentRole::Mcp)
        .map(|d| d.name.clone())
        .ok_or_else(|| anyhow::anyhow!("no MCP deployment in config; `quelch get` requires one"))?;

    let sliced = crate::config::slice::for_deployment(config, &deployment_name)?;
    let expose = ExposeResolver::from_sliced(&sliced, &deployment_name)
        .map_err(|e| anyhow::anyhow!("expose resolver: {e}"))?;

    let req = GetRequest {
        data_source: options.data_source,
        id: options.id.clone(),
        include_deleted: options.include_deleted,
    };

    let resp = get::run(cosmos.as_ref(), &expose, req)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if options.json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    } else {
        match &resp.document {
            None => println!("Document '{}' not found.", options.id),
            Some(doc) => {
                println!("Document: {}", options.id);
                if let Some(link) = doc.get("source_link").and_then(Value::as_str) {
                    println!("  Source:  {link}");
                }
                if let Some(title) = doc
                    .get("title")
                    .and_then(Value::as_str)
                    .or_else(|| doc.get("summary").and_then(Value::as_str))
                {
                    println!("  Title:   {title}");
                }
                if let Some(status) = doc.get("status").and_then(Value::as_str) {
                    println!("  Status:  {status}");
                }
                println!();
                println!("{}", serde_json::to_string_pretty(doc)?);
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::cosmos::{CosmosBackend, InMemoryCosmos};
    use crate::mcp::tools::get::{GetRequest, run as get_run};
    use crate::mcp::tools::test_helpers::build_expose_jira_issues;
    use serde_json::json;

    #[tokio::test]
    async fn get_dispatches_to_get_tool() {
        let cosmos = InMemoryCosmos::new();
        CosmosBackend::upsert(
            &cosmos,
            "jira-issues",
            json!({
                "id": "DO-1",
                "_partition_key": "DO",
                "status": "Open",
                "title": "Test issue",
            }),
        )
        .await
        .unwrap();

        let expose = build_expose_jira_issues();

        let req = GetRequest {
            data_source: "jira_issues".into(),
            id: "DO-1".into(),
            include_deleted: false,
        };

        let resp = get_run(&cosmos, &expose, req).await.unwrap();
        assert!(resp.document.is_some(), "document should be found");
        assert_eq!(resp.document.unwrap()["id"], "DO-1");
    }

    #[tokio::test]
    async fn get_returns_none_for_missing_document() {
        let cosmos = InMemoryCosmos::new();
        let expose = build_expose_jira_issues();

        let req = GetRequest {
            data_source: "jira_issues".into(),
            id: "NOPE-999".into(),
            include_deleted: false,
        };

        let resp = get_run(&cosmos, &expose, req).await.unwrap();
        assert!(
            resp.document.is_none(),
            "missing document should return None"
        );
    }
}
