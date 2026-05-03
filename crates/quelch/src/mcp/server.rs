//! MCP HTTP server: axum app, routes, and `serve()`.
//!
//! # Transport
//!
//! Implements MCP Streamable HTTP transport (the standard as of late 2025 /
//! early 2026).  A client sends `POST /mcp` with a JSON-RPC 2.0 body; the
//! server replies with a single JSON-RPC response (`application/json`).
//!
//! The optional `GET /mcp` passive event stream (server-pushed SSE) is not
//! implemented yet.
//! TODO(mcp-spec): implement SSE push if the live spec mandates it for
//! conformance.
//!
//! # Authentication
//!
//! All routes pass through the [`crate::mcp::auth::api_key_middleware`].
//! The middleware is a no-op when `QUELCH_MCP_API_KEY` is not set.

use std::sync::Arc;

use axum::{Router, middleware, routing::post};

use crate::cosmos::CosmosBackend;
use crate::mcp::schema::SchemaCatalog;
use crate::mcp::tools::search::SearchToolConfig;
use crate::mcp::tools::search_api::SearchApiAdapter;

// ---------------------------------------------------------------------------
// Server state
// ---------------------------------------------------------------------------

/// Shared state injected into every request handler.
#[derive(Clone)]
pub struct ServerState {
    /// Cosmos DB backend (real or in-memory).
    pub cosmos: Arc<dyn CosmosBackend>,
    /// Azure AI Search adapter (real or mock).
    pub search: Arc<dyn SearchApiAdapter>,
    /// Exposure resolver (enforces `expose:` list from the deployment config).
    pub expose: Arc<crate::mcp::expose::ExposeResolver>,
    /// Static schema catalog (descriptions, field schemas, examples).
    pub schema: Arc<SchemaCatalog>,
    /// Runtime configuration for the `search` tool.
    pub search_config: Arc<SearchToolConfig>,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build the axum `Router` for the MCP server.
///
/// Accepts `ServerState` as shared state; passes all routes through
/// `api_key_middleware`.
pub fn router(state: ServerState) -> Router {
    Router::new()
        .route("/mcp", post(crate::mcp::handlers::handle_post))
        .with_state(state)
        .layer(middleware::from_fn(crate::mcp::auth::api_key_middleware))
}

// ---------------------------------------------------------------------------
// Serve
// ---------------------------------------------------------------------------

/// Start the MCP HTTP server and block until it receives a shutdown signal
/// (SIGINT / Ctrl-C) or fails.
///
/// # Arguments
///
/// * `state`     — pre-built `ServerState`.
/// * `bind_addr` — e.g. `"0.0.0.0:8080"`.
pub async fn serve(state: ServerState, bind_addr: &str) -> anyhow::Result<()> {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    tracing::info!(addr = %bind_addr, "Quelch MCP server listening");
    axum::serve(listener, app).await?;
    Ok(())
}
