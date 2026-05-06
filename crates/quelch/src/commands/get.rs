//! `quelch get` — fetch a single document by ID.
//!
//! Operator command. Picks the first MCP instance in the config (so that
//! the same `expose:` rules apply that an agent would see), builds a Cosmos
//! client, and calls the `get` tool implementation.

use crate::config::Config;
use crate::config::schema::InstanceKind;
use crate::cosmos::factory::build_cosmos_backend;
use crate::mcp::expose::ExposeResolver;
use crate::mcp::tools::get::{GetRequest, run as get_run};

/// Options for `quelch get`.
#[derive(Debug)]
pub struct GetOptions {
    pub id: String,
    pub data_source: String,
    pub include_deleted: bool,
    pub json: bool,
}

/// Run `quelch get`.
pub async fn run(config: &Config, options: GetOptions) -> anyhow::Result<()> {
    let mcp_instance_name = pick_mcp_instance_name(config)?;
    let sliced = crate::config::slice::slice_for_instance(config, &mcp_instance_name)?;

    let cosmos = build_cosmos_backend(&sliced).await?;
    let expose = ExposeResolver::from_sliced(&sliced, &mcp_instance_name)
        .map_err(|e| anyhow::anyhow!("expose resolver: {e}"))?;

    let req = GetRequest {
        id: options.id,
        data_source: options.data_source,
        include_deleted: options.include_deleted,
    };

    let resp = get_run(cosmos.as_ref(), &expose, req)
        .await
        .map_err(|e| anyhow::anyhow!("get: {e}"))?;

    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&resp.document.unwrap_or(serde_json::Value::Null))?
        );
    } else if let Some(doc) = resp.document {
        println!("{}", serde_json::to_string_pretty(&doc)?);
    } else {
        println!("(not found)");
    }

    Ok(())
}

/// Return the name of the first MCP instance in the config, or error if
/// none exist.
fn pick_mcp_instance_name(config: &Config) -> anyhow::Result<String> {
    config
        .instances
        .iter()
        .find(|i| i.kind() == InstanceKind::Mcp)
        .map(|i| i.name.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "quelch get requires an MCP instance in the config (none found in instances:)"
            )
        })
}

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
