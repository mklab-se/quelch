//! JSON-RPC 2.0 request dispatcher for the MCP HTTP transport.
//!
//! # Supported methods
//!
//! | Method      | Handler              |
//! |-------------|----------------------|
//! | `initialize`| `initialize::handle` |
//! | `tools/list`| `tools_list::handle` |
//! | `tools/call`| `tools_call::handle` |
//!
//! Unknown methods return JSON-RPC error code `-32601` (Method not found).
//!
//! # Batches
//!
//! The current implementation handles single requests only.  Batch arrays are
//! not yet supported (respond with -32600 Invalid Request).
//!
//! TODO(mcp-spec): verify batch semantics in the live spec and implement if
//! required.

pub mod initialize;
pub mod tools_call;
pub mod tools_list;

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::mcp::server::ServerState;

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 envelope types
// ---------------------------------------------------------------------------

/// A JSON-RPC 2.0 request.
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    #[allow(dead_code)]
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// A JSON-RPC 2.0 response.
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

// ---------------------------------------------------------------------------
// Dispatch handler
// ---------------------------------------------------------------------------

/// Main POST handler: deserialise a single JSON-RPC request, dispatch, respond.
pub async fn handle_post(
    State(state): State<ServerState>,
    Json(req): Json<JsonRpcRequest>,
) -> Json<JsonRpcResponse> {
    let id = req.id.clone();

    let result = match req.method.as_str() {
        "initialize" => initialize::handle(req.params).await,
        "tools/list" => {
            tools_list::handle(state.expose.as_ref(), state.schema.as_ref(), req.params).await
        }
        "tools/call" => tools_call::handle(&state, req.params).await,
        // notifications/initialized is a fire-and-forget; respond with null result.
        "notifications/initialized" => Ok(Value::Null),
        // ping → pong
        "ping" => Ok(serde_json::json!({})),
        _ => Err(method_not_found(&req.method)),
    };

    Json(match result {
        Ok(v) => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: Some(v),
            error: None,
        },
        Err(e) => JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(e),
        },
    })
}

fn method_not_found(method: &str) -> JsonRpcError {
    JsonRpcError {
        code: -32601,
        message: format!("Method not found: {method}"),
        data: None,
    }
}
