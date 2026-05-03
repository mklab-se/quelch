//! MCP `tools/list` handler.
//!
//! Returns the static catalog of tools available on this server. Agents call
//! this once at session start to discover what tools exist and what arguments
//! each accepts.
//!
//! Each entry has:
//! - `name`: the tool identifier passed to `tools/call`.
//! - `description`: human-readable description.
//! - `inputSchema`: a JSON Schema (draft 2020-12) describing the `arguments`.

use schemars::schema_for;
use serde_json::Value;

use crate::mcp::expose::ExposeResolver;
use crate::mcp::schema::SchemaCatalog;
use crate::mcp::tools::{
    aggregate::AggregateRequest, get::GetRequest, list_sources::ListSourcesRequest,
    query::QueryRequest, search::SearchRequest,
};

use super::JsonRpcError;

/// Handle `tools/list`.
pub async fn handle(
    _expose: &ExposeResolver,
    _schema: &SchemaCatalog,
    _params: Value,
) -> Result<Value, JsonRpcError> {
    let tools = build_tool_list();
    Ok(serde_json::json!({ "tools": tools }))
}

/// Build the static tool catalog.
fn build_tool_list() -> Vec<Value> {
    // Use schema_for! which internally creates a generator to produce root schemas.
    // schema_for!(T).to_value() gives us the JSON Schema as serde_json::Value.
    vec![
        tool_entry(
            "search",
            "Hybrid semantic + keyword search across exposed data sources via Azure AI Search. \
             Use for free-text discovery when you don't know exact field values. \
             Supports optional structured filter, pagination, and agentic answer synthesis.",
            schema_for!(SearchRequest).to_value(),
        ),
        tool_entry(
            "query",
            "Structured query against a single data source backed by Cosmos DB. \
             Use when you need exact filtering by field value, ordering, pagination, or counts. \
             Supports the full JSON filter grammar.",
            schema_for!(QueryRequest).to_value(),
        ),
        tool_entry(
            "get",
            "Fetch a single document by its ID from a data source. \
             Returns the full document or null if not found.",
            schema_for!(GetRequest).to_value(),
        ),
        tool_entry(
            "aggregate",
            "Count and sum grouped query results from a data source. \
             Supports group-by on scalar or array fields, sum of numeric fields, \
             and top-N group limiting.",
            schema_for!(AggregateRequest).to_value(),
        ),
        tool_entry(
            "list_sources",
            "List all data sources exposed by this MCP deployment. \
             Returns each source's name, kind, field schema, and example calls. \
             Call this first to discover what data is available.",
            schema_for!(ListSourcesRequest).to_value(),
        ),
    ]
}

fn tool_entry(name: &str, description: &str, input_schema: Value) -> Value {
    serde_json::json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::schema::SchemaCatalog;
    use crate::mcp::tools::test_helpers::build_expose_jira_issues;
    use serde_json::json;

    #[tokio::test]
    async fn tools_list_returns_all_five_tools() {
        let expose = build_expose_jira_issues();
        let schema = SchemaCatalog::new();
        let result = handle(&expose, &schema, json!({})).await.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 5, "expected 5 tools");

        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"search"));
        assert!(names.contains(&"query"));
        assert!(names.contains(&"get"));
        assert!(names.contains(&"aggregate"));
        assert!(names.contains(&"list_sources"));
    }

    #[tokio::test]
    async fn tools_list_each_has_input_schema() {
        let expose = build_expose_jira_issues();
        let schema = SchemaCatalog::new();
        let result = handle(&expose, &schema, json!({})).await.unwrap();
        let tools = result["tools"].as_array().unwrap();
        for tool in tools {
            assert!(
                tool["inputSchema"].is_object(),
                "tool '{}' missing inputSchema",
                tool["name"]
            );
        }
    }
}
