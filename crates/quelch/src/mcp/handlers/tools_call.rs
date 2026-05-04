//! MCP `tools/call` handler.
//!
//! Dispatches a `tools/call` JSON-RPC request to one of the five MCP tool
//! implementations: `search`, `query`, `get`, `aggregate`, `list_sources`.
//!
//! # Response shape
//!
//! Successful tool responses are wrapped in the MCP `content` envelope:
//! ```json
//! { "content": [{ "type": "text", "text": "<json string>" }] }
//! ```
//!
//! TODO(mcp-spec): The MCP spec 2025-11-05 defines two content types for tool
//! results: `text` (string) and `image` (base64).  JSON results are idiomatic
//! as `text` with a JSON string inside, or potentially as structured content if
//! the spec evolves to support it directly.  Verify against the live spec and
//! adjust the envelope if needed.

use serde::Deserialize;
use serde_json::Value;

use crate::mcp::error::McpError;
use crate::mcp::server::ServerState;
use crate::mcp::tools::{aggregate, get, list_sources, query, search};

use super::JsonRpcError;

// ---------------------------------------------------------------------------
// Call params
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Handle `tools/call`.
pub async fn handle(state: &ServerState, params: Value) -> Result<Value, JsonRpcError> {
    let CallParams { name, arguments } = serde_json::from_value(params)
        .map_err(|e| invalid_params(format!("invalid call params: {e}")))?;

    let result: Value = match name.as_str() {
        "search" => {
            let req: search::SearchRequest = serde_json::from_value(arguments)
                .map_err(|e| invalid_params(format!("search args: {e}")))?;
            let resp = search::run(
                state.search.as_ref(),
                state.expose.as_ref(),
                state.schema.as_ref(),
                state.search_config.as_ref(),
                req,
            )
            .await
            .map_err(map_mcp_error)?;
            to_value_or_internal(&resp)?
        }
        "query" => {
            let req: query::QueryRequest = serde_json::from_value(arguments)
                .map_err(|e| invalid_params(format!("query args: {e}")))?;
            let resp = query::run(state.cosmos.as_ref(), state.expose.as_ref(), req)
                .await
                .map_err(map_mcp_error)?;
            to_value_or_internal(&resp)?
        }
        "get" => {
            let req: get::GetRequest = serde_json::from_value(arguments)
                .map_err(|e| invalid_params(format!("get args: {e}")))?;
            let resp = get::run(state.cosmos.as_ref(), state.expose.as_ref(), req)
                .await
                .map_err(map_mcp_error)?;
            to_value_or_internal(&resp)?
        }
        "aggregate" => {
            let req: aggregate::AggregateRequest = serde_json::from_value(arguments)
                .map_err(|e| invalid_params(format!("aggregate args: {e}")))?;
            let resp = aggregate::run(
                state.cosmos.as_ref(),
                state.expose.as_ref(),
                state.schema.as_ref(),
                req,
            )
            .await
            .map_err(map_mcp_error)?;
            to_value_or_internal(&resp)?
        }
        "list_sources" => {
            let resp = list_sources::run(state.expose.as_ref(), state.schema.as_ref())
                .await
                .map_err(map_mcp_error)?;
            to_value_or_internal(&resp)?
        }
        other => return Err(invalid_params(format!("unknown tool: {other}"))),
    };

    // Wrap in MCP content envelope.
    // TODO(mcp-spec): verify content envelope shape against the live spec.
    let json_str = serde_json::to_string(&result)
        .map_err(|e| internal_error(format!("serialise tool result: {e}")))?;
    Ok(serde_json::json!({
        "content": [{ "type": "text", "text": json_str }]
    }))
}

fn to_value_or_internal<T: serde::Serialize>(value: &T) -> Result<Value, JsonRpcError> {
    serde_json::to_value(value).map_err(|e| internal_error(format!("serialise tool response: {e}")))
}

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

fn invalid_params(msg: String) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: msg,
        data: None,
    }
}

fn internal_error(msg: String) -> JsonRpcError {
    JsonRpcError {
        code: -32603,
        message: msg,
        data: None,
    }
}

fn map_mcp_error(err: McpError) -> JsonRpcError {
    let (code, msg) = match &err {
        McpError::NotFound(_) => (-32004, format!("{err}")),
        McpError::Forbidden(_) => (-32003, format!("{err}")),
        McpError::InvalidArgument(_) => (-32602, format!("{err}")),
        McpError::Unauthenticated(_) => (-32001, format!("{err}")),
        McpError::Unavailable(_) => (-32005, format!("{err}")),
        McpError::Internal(_) => (-32603, format!("{err}")),
        McpError::Filter(_) => (-32602, format!("{err}")),
        McpError::Cosmos(_) => (-32005, format!("{err}")),
    };
    JsonRpcError {
        code,
        message: msg,
        data: None,
    }
}
