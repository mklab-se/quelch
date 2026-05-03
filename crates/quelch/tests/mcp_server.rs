//! MCP HTTP server integration tests.
//!
//! These tests exercise the full request/response stack — from an HTTP POST
//! to the `/mcp` route — using an in-process axum router with mocked
//! dependencies.  No real Azure infrastructure is required.

use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{Request, StatusCode};
use axum::{Router, body::Body};
use serde_json::{Value, json};
use tower::ServiceExt;

use quelch::cosmos::{CosmosBackend, InMemoryCosmos};
use quelch::mcp::error::McpError;
use quelch::mcp::expose::ExposeResolver;
use quelch::mcp::schema::SchemaCatalog;
use quelch::mcp::server::{ServerState, router};
use quelch::mcp::tools::search::SearchToolConfig;
use quelch::mcp::tools::search_api::{RawSearchResponse, SearchApiAdapter};

// ---------------------------------------------------------------------------
// No-op search adapter
// ---------------------------------------------------------------------------

struct NoOpSearch;

#[async_trait]
impl SearchApiAdapter for NoOpSearch {
    async fn search_knowledge_base(
        &self,
        _knowledge_base_name: &str,
        _query: &str,
        _odata_filter: Option<&str>,
        _top: usize,
        _cursor: Option<&str>,
        _include_synthesis: bool,
        _include_full_body: bool,
    ) -> Result<RawSearchResponse, McpError> {
        Ok(RawSearchResponse {
            hits: vec![],
            answer: None,
            citations: None,
            next_cursor: None,
            total_estimate: 0,
        })
    }

    async fn search_index(
        &self,
        _index_name: &str,
        _query: &str,
        _odata_filter: Option<&str>,
        _top: usize,
        _cursor: Option<&str>,
        _include_full_body: bool,
    ) -> Result<RawSearchResponse, McpError> {
        Ok(RawSearchResponse {
            hits: vec![],
            answer: None,
            citations: None,
            next_cursor: None,
            total_estimate: 0,
        })
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Serialise ALL tests through a single mutex because `QUELCH_MCP_API_KEY` is
/// process-global.  If the auth test sets it and another test runs concurrently
/// without the key, all requests fail with 401.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Build a `ServerState` with in-memory / mock backends.
/// Exposes only `jira_issues` (jira_issue kind, jira-issues container).
fn build_test_state() -> ServerState {
    use quelch::config::{BackedBy, data_sources::ResolvedDataSource};
    use std::collections::HashMap;

    let mut map = HashMap::new();
    map.insert(
        "jira_issues".to_string(),
        ResolvedDataSource {
            kind: "jira_issue".to_string(),
            backed_by: vec![BackedBy {
                container: "jira-issues".to_string(),
            }],
        },
    );

    ServerState {
        cosmos: Arc::new(InMemoryCosmos::new()),
        search: Arc::new(NoOpSearch),
        expose: Arc::new(ExposeResolver::from_map(map)),
        schema: Arc::new(SchemaCatalog::new()),
        search_config: Arc::new(SearchToolConfig::default()),
    }
}

/// Send a POST to /mcp with a JSON-RPC body; return (status, parsed response).
async fn post_mcp(app: Router, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    (status, json)
}

// ---------------------------------------------------------------------------
// initialize
// ---------------------------------------------------------------------------

#[tokio::test]
async fn initialize_returns_server_info() {
    let _guard = ENV_LOCK.lock().unwrap();
    // SAFETY: protected by ENV_LOCK.
    unsafe { std::env::remove_var("QUELCH_MCP_API_KEY") };

    let app = router(build_test_state());
    let (status, json) = post_mcp(
        app,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["jsonrpc"], "2.0");
    assert_eq!(json["id"], 1);
    assert!(json["result"]["serverInfo"]["name"].as_str() == Some("quelch-mcp"));
    assert!(json["result"]["protocolVersion"].is_string());
    assert!(json["result"]["capabilities"]["tools"].is_object());
    assert!(json["error"].is_null());
}

// ---------------------------------------------------------------------------
// tools/list
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_sources_via_http() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe { std::env::remove_var("QUELCH_MCP_API_KEY") };

    let app = router(build_test_state());
    let (status, json) = post_mcp(
        app,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let tools = json["result"]["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 5, "expected 5 tools");

    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"search"));
    assert!(names.contains(&"query"));
    assert!(names.contains(&"get"));
    assert!(names.contains(&"aggregate"));
    assert!(names.contains(&"list_sources"));
}

// ---------------------------------------------------------------------------
// tools/call — list_sources
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tools_call_list_sources_returns_result() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe { std::env::remove_var("QUELCH_MCP_API_KEY") };

    let app = router(build_test_state());
    let (status, json) = post_mcp(
        app,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "list_sources",
                "arguments": {}
            }
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(json["result"]["content"].is_array());
    let content_text = json["result"]["content"][0]["text"].as_str().unwrap();
    let inner: Value = serde_json::from_str(content_text).unwrap();
    assert!(inner["data_sources"].is_array());
    let ds = inner["data_sources"].as_array().unwrap();
    assert_eq!(ds.len(), 1);
    assert_eq!(ds[0]["name"], "jira_issues");
}

// ---------------------------------------------------------------------------
// tools/call — query
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tools_call_query_returns_result() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe { std::env::remove_var("QUELCH_MCP_API_KEY") };

    // Populate Cosmos with some docs.
    let cosmos = InMemoryCosmos::new();
    cosmos
        .upsert(
            "jira-issues",
            json!({"id": "DO-1", "_partition_key": "DO", "status": "Open"}),
        )
        .await
        .unwrap();
    cosmos
        .upsert(
            "jira-issues",
            json!({"id": "DO-2", "_partition_key": "DO", "status": "Done"}),
        )
        .await
        .unwrap();

    use quelch::config::{BackedBy, data_sources::ResolvedDataSource};
    use std::collections::HashMap;
    let mut map = HashMap::new();
    map.insert(
        "jira_issues".to_string(),
        ResolvedDataSource {
            kind: "jira_issue".to_string(),
            backed_by: vec![BackedBy {
                container: "jira-issues".to_string(),
            }],
        },
    );

    let state = ServerState {
        cosmos: Arc::new(cosmos),
        search: Arc::new(NoOpSearch),
        expose: Arc::new(ExposeResolver::from_map(map)),
        schema: Arc::new(SchemaCatalog::new()),
        search_config: Arc::new(SearchToolConfig::default()),
    };

    let app = router(state);
    let (status, json) = post_mcp(
        app,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "query",
                "arguments": {
                    "data_source": "jira_issues",
                    "top": 50
                }
            }
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let content_text = json["result"]["content"][0]["text"].as_str().unwrap();
    let inner: Value = serde_json::from_str(content_text).unwrap();
    assert_eq!(inner["total"], 2);
    assert_eq!(inner["items"].as_array().unwrap().len(), 2);
}

// ---------------------------------------------------------------------------
// tools/call — search
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tools_call_search_returns_result() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe { std::env::remove_var("QUELCH_MCP_API_KEY") };

    let app = router(build_test_state());
    let (status, json) = post_mcp(
        app,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "search",
                "arguments": {
                    "query": "open bugs",
                    "data_sources": ["jira_issues"],
                    "include_content": "snippet"
                }
            }
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        json["error"].is_null(),
        "unexpected error: {}",
        json["error"]
    );
    let content_text = json["result"]["content"][0]["text"].as_str().unwrap();
    let inner: Value = serde_json::from_str(content_text).unwrap();
    assert!(inner["items"].is_array());
}

// ---------------------------------------------------------------------------
// Unknown method → method-not-found error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unknown_method_returns_method_not_found_error() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe { std::env::remove_var("QUELCH_MCP_API_KEY") };

    let app = router(build_test_state());
    let (status, json) = post_mcp(
        app,
        json!({ "jsonrpc": "2.0", "id": 5, "method": "bogus/method", "params": {} }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "HTTP is always 200 for JSON-RPC");
    assert!(json["result"].is_null());
    assert_eq!(json["error"]["code"], -32601);
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("bogus/method")
    );
}

// ---------------------------------------------------------------------------
// Forbidden data source
// ---------------------------------------------------------------------------

#[tokio::test]
async fn forbidden_data_source_returns_proper_error() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe { std::env::remove_var("QUELCH_MCP_API_KEY") };

    // State only exposes jira_issues; querying confluence_pages is forbidden.
    let app = router(build_test_state());
    let (status, json) = post_mcp(
        app,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "query",
                "arguments": {
                    "data_source": "confluence_pages",
                    "top": 10
                }
            }
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(json["result"].is_null());
    assert_eq!(json["error"]["code"], -32003, "expected Forbidden code");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("confluence_pages")
    );
}

// ---------------------------------------------------------------------------
// Auth middleware — blocks missing key
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_middleware_blocks_missing_key_when_configured() {
    let _guard = ENV_LOCK.lock().unwrap();
    let prev = std::env::var("QUELCH_MCP_API_KEY").ok();
    // SAFETY: protected by ENV_LOCK.
    unsafe { std::env::set_var("QUELCH_MCP_API_KEY", "integration-test-key") };

    let app = router(build_test_state());
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("Content-Type", "application/json")
        .body(Body::from(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "missing auth header should return 401"
    );

    unsafe { std::env::remove_var("QUELCH_MCP_API_KEY") };
    if let Some(v) = prev {
        unsafe { std::env::set_var("QUELCH_MCP_API_KEY", v) };
    }
}
