pub mod data;

mod azure;
mod confluence;
mod jira;
mod sim;

use axum::{
    Router,
    http::{HeaderMap, StatusCode},
    response::Json,
    routing::{delete, get, post, put},
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

pub(super) const MOCK_TOKEN: &str = "mock-pat-token";

// -----------------------------------------------------------------------
// Shared server state (for Azure mock)
// -----------------------------------------------------------------------

/// Per-index storage for Azure mock.
#[derive(Default)]
pub(super) struct IndexStore {
    pub(super) docs: HashMap<String, Value>,
}

#[derive(Default)]
pub(super) struct AzureStore {
    pub(super) indexes: HashMap<String, IndexStore>,
    pub(super) pending_faults: Vec<u16>,
}

/// Top-level mock state — owns Azure + mutable Jira/Confluence stores.
#[derive(Default)]
pub(super) struct MockState {
    pub(super) azure: AzureStore,
    pub(super) jira_issues: Vec<Value>,
    pub(super) confluence_pages: Vec<Value>,
}

pub(super) type SharedState = Arc<Mutex<MockState>>;

pub(super) fn consume_fault(state: &SharedState) -> Option<u16> {
    let mut s = state.lock().unwrap();
    if s.azure.pending_faults.is_empty() {
        None
    } else {
        Some(s.azure.pending_faults.remove(0))
    }
}

// -----------------------------------------------------------------------
// Auth helper
// -----------------------------------------------------------------------

pub(super) fn check_auth(headers: &HeaderMap) -> Result<(), (StatusCode, Json<Value>)> {
    let expected = format!("Bearer {MOCK_TOKEN}");
    match headers.get("authorization").and_then(|v| v.to_str().ok()) {
        Some(val) if val == expected => Ok(()),
        _ => Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "errorMessages": ["Authentication required. Use 'Authorization: Bearer mock-pat-token'"],
                "errors": {}
            })),
        )),
    }
}

// -----------------------------------------------------------------------
// Router builder (testable, used by integration tests)
// -----------------------------------------------------------------------

/// Build the axum Router used by the mock server. `pub` so integration
/// tests outside this module can spin up an in-process instance.
pub fn build_router() -> Router {
    use azure::{
        azure_fault_post, azure_index_delete, azure_index_docs_list, azure_index_docs_post,
        azure_index_get, azure_index_put, azure_index_search_post, azure_indexes_collection_post,
    };
    use confluence::confluence_search;
    use jira::jira_search;
    use sim::{sim_add_comment, sim_upsert_issue, sim_upsert_page};

    let state = Arc::new(Mutex::new(MockState {
        azure: AzureStore::default(),
        jira_issues: data::jira_issues(),
        confluence_pages: data::confluence_pages(),
    }));

    Router::new()
        // Jira + Confluence routes:
        .route("/jira/rest/api/2/search", get(jira_search))
        .route(
            "/confluence/rest/api/content/search",
            get(confluence_search),
        )
        // Azure routes (all share the same state):
        .route("/azure/indexes", post(azure_indexes_collection_post))
        .route("/azure/indexes/{name}", get(azure_index_get))
        .route("/azure/indexes/{name}", put(azure_index_put))
        .route("/azure/indexes/{name}", delete(azure_index_delete))
        .route(
            "/azure/indexes/{name}/docs/index",
            post(azure_index_docs_post),
        )
        .route(
            "/azure/indexes/{name}/docs/search",
            post(azure_index_search_post),
        )
        .route("/azure/indexes/{name}/docs", get(azure_index_docs_list))
        .route("/azure/_fault", post(azure_fault_post))
        // Simulator mutation endpoints:
        .route("/_sim/jira/upsert_issue", post(sim_upsert_issue))
        .route("/_sim/confluence/upsert_page", post(sim_upsert_page))
        .route("/_sim/jira/comment", post(sim_add_comment))
        .with_state(state)
}

// -----------------------------------------------------------------------
// Server entry point
// -----------------------------------------------------------------------

/// Start the mock Jira DC + Confluence DC server.
pub async fn run_mock_server(port: u16) -> anyhow::Result<()> {
    let app = build_router();
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    println!("Mock Jira DC server running at http://localhost:{port}/jira");
    println!("Mock Confluence DC server running at http://localhost:{port}/confluence");
    println!();
    println!("Auth token: {MOCK_TOKEN}");
    println!("Jira projects: QUELCH (17 issues), DEMO (2 issues)");
    println!("Confluence spaces: QUELCH (8 pages), INFRA (2 pages)");
    println!();
    println!("Example quelch.yaml config:");
    println!();
    println!("  azure:");
    println!("    endpoint: \"https://your-search.search.windows.net\"");
    println!("    api_key: \"${{AZURE_SEARCH_API_KEY}}\"");
    println!();
    println!("  sources:");
    println!("    - type: jira");
    println!("      name: \"mock-jira\"");
    println!("      url: \"http://localhost:{port}/jira\"");
    println!("      auth:");
    println!("        pat: \"{MOCK_TOKEN}\"");
    println!("      projects:");
    println!("        - \"QUELCH\"");
    println!("        - \"DEMO\"");
    println!("      index: \"jira-issues\"");
    println!();
    println!("    - type: confluence");
    println!("      name: \"mock-confluence\"");
    println!("      url: \"http://localhost:{port}/confluence\"");
    println!("      auth:");
    println!("        pat: \"{MOCK_TOKEN}\"");
    println!("      spaces:");
    println!("        - \"QUELCH\"");
    println!("        - \"INFRA\"");
    println!("      index: \"confluence-pages\"");
    println!();
    println!("Press Ctrl+C to stop.");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    async fn spawn_test_server() -> String {
        let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, build_router()).await.unwrap();
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn azure_index_create_get_delete_roundtrip() {
        let base = spawn_test_server().await;
        let client = reqwest::Client::new();

        let put = client
            .put(format!(
                "{}/azure/indexes/test-idx?api-version=2024-07-01",
                base
            ))
            .header("api-key", "ignored-by-mock")
            .json(&serde_json::json!({ "name": "test-idx", "fields": [] }))
            .send()
            .await
            .unwrap();
        assert!(put.status().is_success(), "PUT failed: {}", put.status());

        let get = client
            .get(format!(
                "{}/azure/indexes/test-idx?api-version=2024-07-01",
                base
            ))
            .send()
            .await
            .unwrap();
        assert!(get.status().is_success());

        let del = client
            .delete(format!(
                "{}/azure/indexes/test-idx?api-version=2024-07-01",
                base
            ))
            .send()
            .await
            .unwrap();
        assert!(del.status().is_success());

        let after = client
            .get(format!(
                "{}/azure/indexes/test-idx?api-version=2024-07-01",
                base
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(after.status().as_u16(), 404);
    }

    #[tokio::test]
    async fn azure_push_and_search_documents() {
        let base = spawn_test_server().await;
        let client = reqwest::Client::new();

        client
            .put(format!(
                "{}/azure/indexes/docs?api-version=2024-07-01",
                base
            ))
            .json(&serde_json::json!({ "name": "docs", "fields": [] }))
            .send()
            .await
            .unwrap();

        let body = serde_json::json!({
            "value": [
                { "@search.action": "mergeOrUpload", "id": "a", "content": "hello world" },
                { "@search.action": "mergeOrUpload", "id": "b", "content": "quelch rocks" },
            ]
        });
        let push = client
            .post(format!(
                "{}/azure/indexes/docs/docs/index?api-version=2024-07-01",
                base
            ))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert!(push.status().is_success());

        let search = client
            .post(format!(
                "{}/azure/indexes/docs/docs/search?api-version=2024-07-01",
                base
            ))
            .json(&serde_json::json!({ "search": "hello" }))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = search.json().await.unwrap();
        let values = body.get("value").and_then(|v| v.as_array()).unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].get("id").unwrap(), "a");
    }

    #[tokio::test]
    async fn azure_fault_injection_next_n_calls() {
        let base = spawn_test_server().await;
        let client = reqwest::Client::new();

        client
            .post(format!("{}/azure/_fault", base))
            .json(&serde_json::json!({ "count": 2, "status": 429 }))
            .send()
            .await
            .unwrap();

        let r1 = client
            .get(format!("{}/azure/indexes/x?api-version=2024-07-01", base))
            .send()
            .await
            .unwrap();
        assert_eq!(r1.status().as_u16(), 429);

        let r2 = client
            .get(format!("{}/azure/indexes/x?api-version=2024-07-01", base))
            .send()
            .await
            .unwrap();
        assert_eq!(r2.status().as_u16(), 429);

        let r3 = client
            .get(format!("{}/azure/indexes/x?api-version=2024-07-01", base))
            .send()
            .await
            .unwrap();
        assert_eq!(r3.status().as_u16(), 404);
    }

    #[tokio::test]
    async fn azure_post_indexes_collection_creates_from_body() {
        let base = spawn_test_server().await;
        let client = reqwest::Client::new();

        let resp = client
            .post(format!("{}/azure/indexes?api-version=2024-07-01", base))
            .json(&serde_json::json!({ "name": "coll-idx", "fields": [] }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());

        let get = client
            .get(format!(
                "{}/azure/indexes/coll-idx?api-version=2024-07-01",
                base
            ))
            .send()
            .await
            .unwrap();
        assert!(get.status().is_success());
    }

    #[tokio::test]
    async fn jira_data_has_two_projects() {
        let base = spawn_test_server().await;
        let client = reqwest::Client::new();

        let quelch_resp = client
            .get(format!("{}/jira/rest/api/2/search", base))
            .header("authorization", format!("Bearer {}", MOCK_TOKEN))
            .query(&[("jql", "project = QUELCH"), ("maxResults", "100")])
            .send()
            .await
            .unwrap();
        let q: serde_json::Value = quelch_resp.json().await.unwrap();
        assert!(q.get("total").unwrap().as_u64().unwrap() > 0);

        let demo_resp = client
            .get(format!("{}/jira/rest/api/2/search", base))
            .header("authorization", format!("Bearer {}", MOCK_TOKEN))
            .query(&[("jql", "project = DEMO"), ("maxResults", "100")])
            .send()
            .await
            .unwrap();
        let d: serde_json::Value = demo_resp.json().await.unwrap();
        assert!(
            d.get("total").unwrap().as_u64().unwrap() > 0,
            "DEMO project should exist"
        );
    }

    #[tokio::test]
    async fn confluence_data_has_two_spaces() {
        let base = spawn_test_server().await;
        let client = reqwest::Client::new();

        let quelch = client
            .get(format!("{}/confluence/rest/api/content/search", base))
            .header("authorization", format!("Bearer {}", MOCK_TOKEN))
            .query(&[("cql", "space = QUELCH")])
            .send()
            .await
            .unwrap();
        let q: serde_json::Value = quelch.json().await.unwrap();
        assert!(q.get("size").unwrap().as_u64().unwrap() > 0);

        let infra = client
            .get(format!("{}/confluence/rest/api/content/search", base))
            .header("authorization", format!("Bearer {}", MOCK_TOKEN))
            .query(&[("cql", "space = INFRA")])
            .send()
            .await
            .unwrap();
        let i: serde_json::Value = infra.json().await.unwrap();
        assert!(
            i.get("size").unwrap().as_u64().unwrap() > 0,
            "INFRA space should exist"
        );
    }

    #[tokio::test]
    async fn sim_upsert_issue_adds_to_jira_store() {
        let base = spawn_test_server().await;
        let client = reqwest::Client::new();

        let resp = client
            .post(format!("{base}/_sim/jira/upsert_issue"))
            .json(&serde_json::json!({
                "project": "QUELCH",
                "key": "QUELCH-999",
                "summary": "sim-created",
                "description": "injected by sim",
            }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());

        let search = client
            .get(format!("{base}/jira/rest/api/2/search"))
            .header("authorization", format!("Bearer {}", MOCK_TOKEN))
            .query(&[("jql", "project = QUELCH"), ("maxResults", "500")])
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = search.json().await.unwrap();
        let issues = body.get("issues").unwrap().as_array().unwrap();
        assert!(
            issues
                .iter()
                .any(|i| i.get("key").and_then(|k| k.as_str()) == Some("QUELCH-999")),
            "injected issue not in search result"
        );
    }

    #[tokio::test]
    async fn sim_upsert_page_adds_to_confluence_store() {
        let base = spawn_test_server().await;
        let client = reqwest::Client::new();

        let resp = client
            .post(format!("{base}/_sim/confluence/upsert_page"))
            .json(&serde_json::json!({
                "space": "INFRA",
                "id": "200500",
                "title": "sim-created",
                "body": "<p>injected</p>",
            }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());

        let search = client
            .get(format!("{base}/confluence/rest/api/content/search"))
            .header("authorization", format!("Bearer {}", MOCK_TOKEN))
            .query(&[("cql", "space = INFRA")])
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = search.json().await.unwrap();
        let results = body.get("results").unwrap().as_array().unwrap();
        assert!(
            results
                .iter()
                .any(|r| r.get("id").and_then(|i| i.as_str()) == Some("200500"))
        );
    }
}
