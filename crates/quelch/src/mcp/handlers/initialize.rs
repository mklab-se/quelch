//! MCP `initialize` handshake handler.
//!
//! The client sends `{"method":"initialize","params":{"protocolVersion":"...",...}}`
//! and the server replies with its capabilities and version.
//!
//! # Protocol version
//!
//! We advertise `"2025-11-05"` — the latest stable revision of the MCP spec.
//! TODO(mcp-spec): verify against https://modelcontextprotocol.io/specification/
//! and update if a newer revision has been published.

use serde_json::Value;

use super::JsonRpcError;

/// Handle the `initialize` method.
///
/// Returns server info and capability advertisement.
pub async fn handle(_params: Value) -> Result<Value, JsonRpcError> {
    Ok(serde_json::json!({
        // TODO(mcp-spec): check https://modelcontextprotocol.io/specification/ for the
        // latest published protocol version string and update if needed.
        "protocolVersion": "2025-11-05",
        "capabilities": {
            // Advertising tool support.  `listChanged: false` means the tool
            // list is static and the server will not push list-changed notifications.
            "tools": {
                "listChanged": false
            }
        },
        "serverInfo": {
            "name": "quelch-mcp",
            "version": env!("CARGO_PKG_VERSION")
        }
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn initialize_returns_server_info() {
        let result = handle(json!({})).await.unwrap();
        assert!(result["serverInfo"]["name"].as_str() == Some("quelch-mcp"));
        assert!(result["serverInfo"]["version"].is_string());
        assert!(result["protocolVersion"].is_string());
        assert!(result["capabilities"]["tools"].is_object());
    }
}
