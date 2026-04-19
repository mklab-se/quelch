pub mod data;

use axum::{
    Router,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
    routing::{delete, get, post, put},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

const MOCK_TOKEN: &str = "mock-pat-token";

// -----------------------------------------------------------------------
// Shared server state (for Azure mock)
// -----------------------------------------------------------------------

/// Per-index storage: map of doc id → full doc object.
#[derive(Default)]
struct IndexStore {
    docs: HashMap<String, Value>,
}

#[derive(Default)]
struct AzureMockState {
    indexes: HashMap<String, IndexStore>,
    /// Remaining forced faults: each fault applies to the next single request.
    pending_faults: Vec<u16>,
}

type SharedState = Arc<Mutex<AzureMockState>>;

/// Returns Some(status) if a fault was consumed; None otherwise.
fn consume_fault(state: &SharedState) -> Option<u16> {
    let mut s = state.lock().unwrap();
    if s.pending_faults.is_empty() {
        None
    } else {
        Some(s.pending_faults.remove(0))
    }
}

// -----------------------------------------------------------------------
// Auth helper
// -----------------------------------------------------------------------

fn check_auth(headers: &HeaderMap) -> Result<(), (StatusCode, Json<Value>)> {
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
// Jira endpoint
// -----------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JiraSearchParams {
    jql: Option<String>,
    start_at: Option<u64>,
    max_results: Option<u64>,
    #[serde(default)]
    #[allow(dead_code)]
    fields: Option<String>,
}

async fn jira_search(
    headers: HeaderMap,
    Query(params): Query<JiraSearchParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    check_auth(&headers)?;

    let all_issues = data::jira_issues();
    let jql = params.jql.unwrap_or_default();

    // Filter by project if JQL contains "project = X"
    let filtered: Vec<&Value> = all_issues
        .iter()
        .filter(|issue| {
            if jql.is_empty() {
                return true;
            }
            // Parse project filter
            if let Some(project) = extract_jql_project(&jql) {
                let issue_project = issue["fields"]["project"]["key"].as_str().unwrap_or("");
                if !project.eq_ignore_ascii_case(issue_project) {
                    return false;
                }
            }
            // Parse updated >= filter
            if let Some(updated_since) = extract_jql_updated(&jql) {
                let issue_updated = issue["fields"]["updated"].as_str().unwrap_or("");
                if issue_updated < updated_since.as_str() {
                    return false;
                }
            }
            true
        })
        .collect();

    let start_at = params.start_at.unwrap_or(0);
    let max_results = params.max_results.unwrap_or(50);
    let total = filtered.len() as u64;

    let page: Vec<Value> = filtered
        .into_iter()
        .skip(start_at as usize)
        .take(max_results as usize)
        .cloned()
        .collect();

    Ok(Json(json!({
        "expand": "schema,names",
        "startAt": start_at,
        "maxResults": max_results,
        "total": total,
        "issues": page
    })))
}

// -----------------------------------------------------------------------
// Confluence endpoint
// -----------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ConfluenceSearchParams {
    cql: Option<String>,
    start: Option<u64>,
    limit: Option<u64>,
    #[serde(default)]
    #[allow(dead_code)]
    expand: Option<String>,
}

async fn confluence_search(
    headers: HeaderMap,
    Query(params): Query<ConfluenceSearchParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    check_auth(&headers)?;

    let all_pages = data::confluence_pages();
    let cql = params.cql.unwrap_or_default();

    let filtered: Vec<&Value> = all_pages
        .iter()
        .filter(|page| {
            if cql.is_empty() {
                return true;
            }
            // Parse space filter
            if let Some(space) = extract_cql_space(&cql) {
                let page_space = page["space"]["key"].as_str().unwrap_or("");
                if !space.eq_ignore_ascii_case(page_space) {
                    return false;
                }
            }
            // Parse lastmodified filter
            if let Some(since) = extract_cql_lastmodified(&cql) {
                let page_updated = page["version"]["when"].as_str().unwrap_or("");
                if page_updated < since.as_str() {
                    return false;
                }
            }
            true
        })
        .collect();

    let start = params.start.unwrap_or(0);
    let limit = params.limit.unwrap_or(25);
    let total = filtered.len() as u64;

    let page_slice: Vec<Value> = filtered
        .into_iter()
        .skip(start as usize)
        .take(limit as usize)
        .cloned()
        .collect();

    let has_more = (start + page_slice.len() as u64) < total;
    let mut links = json!({
        "base": format!("http://localhost:9999/confluence"),
        "context": "/confluence"
    });
    if has_more {
        links["next"] = json!(format!(
            "/rest/api/content/search?cql={}&start={}&limit={}",
            cql,
            start + limit,
            limit
        ));
    }

    Ok(Json(json!({
        "results": page_slice,
        "start": start,
        "limit": limit,
        "size": page_slice.len(),
        "_links": links
    })))
}

// -----------------------------------------------------------------------
// Azure AI Search mock handlers
// -----------------------------------------------------------------------

async fn azure_index_get(
    State(state): State<SharedState>,
    AxumPath(name): AxumPath<String>,
) -> impl IntoResponse {
    if let Some(status) = consume_fault(&state) {
        return (StatusCode::from_u16(status).unwrap(), Json(json!({}))).into_response();
    }
    let s = state.lock().unwrap();
    if s.indexes.contains_key(&name) {
        (StatusCode::OK, Json(json!({ "name": name }))).into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response()
    }
}

async fn azure_index_put(
    State(state): State<SharedState>,
    AxumPath(name): AxumPath<String>,
    Json(_body): Json<Value>,
) -> impl IntoResponse {
    if let Some(status) = consume_fault(&state) {
        return (StatusCode::from_u16(status).unwrap(), Json(json!({}))).into_response();
    }
    state
        .lock()
        .unwrap()
        .indexes
        .entry(name.clone())
        .or_default();
    (StatusCode::CREATED, Json(json!({ "name": name }))).into_response()
}

async fn azure_index_delete(
    State(state): State<SharedState>,
    AxumPath(name): AxumPath<String>,
) -> impl IntoResponse {
    if let Some(status) = consume_fault(&state) {
        return (StatusCode::from_u16(status).unwrap(), Json(json!({}))).into_response();
    }
    state.lock().unwrap().indexes.remove(&name);
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Debug, Deserialize)]
struct AzureBatch {
    value: Vec<Value>,
}

async fn azure_index_docs_post(
    State(state): State<SharedState>,
    AxumPath(name): AxumPath<String>,
    Json(batch): Json<AzureBatch>,
) -> impl IntoResponse {
    if let Some(status) = consume_fault(&state) {
        return (StatusCode::from_u16(status).unwrap(), Json(json!({}))).into_response();
    }
    let mut s = state.lock().unwrap();
    let store = s.indexes.entry(name).or_default();
    let mut results = Vec::new();
    for mut doc in batch.value {
        let action = doc
            .get("@search.action")
            .and_then(|v| v.as_str())
            .unwrap_or("mergeOrUpload")
            .to_string();
        let id = doc
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if let Some(obj) = doc.as_object_mut() {
            obj.remove("@search.action");
        }
        match action.as_str() {
            "delete" => {
                store.docs.remove(&id);
            }
            _ => {
                store.docs.insert(id.clone(), doc);
            }
        }
        results.push(json!({ "key": id, "status": true, "statusCode": 200 }));
    }
    (StatusCode::OK, Json(json!({ "value": results }))).into_response()
}

#[derive(Debug, Deserialize)]
struct AzureSearchBody {
    search: Option<String>,
}

async fn azure_index_search_post(
    State(state): State<SharedState>,
    AxumPath(name): AxumPath<String>,
    Json(body): Json<AzureSearchBody>,
) -> impl IntoResponse {
    if let Some(status) = consume_fault(&state) {
        return (StatusCode::from_u16(status).unwrap(), Json(json!({}))).into_response();
    }
    let s = state.lock().unwrap();
    let store = match s.indexes.get(&name) {
        Some(v) => v,
        None => {
            return (StatusCode::NOT_FOUND, Json(json!({ "error": "no index" }))).into_response();
        }
    };
    let q = body.search.unwrap_or_default().to_lowercase();
    let results: Vec<Value> = store
        .docs
        .values()
        .filter(|doc| {
            if q.is_empty() || q == "*" {
                return true;
            }
            doc.as_object()
                .map(|o| {
                    o.values().any(|v| {
                        v.as_str()
                            .map(|s| s.to_lowercase().contains(&q))
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    (StatusCode::OK, Json(json!({ "value": results }))).into_response()
}

/// GET /azure/indexes/{name}/docs — ID-listing (used by SearchClient::fetch_all_ids).
async fn azure_index_docs_list(
    State(state): State<SharedState>,
    AxumPath(name): AxumPath<String>,
) -> impl IntoResponse {
    if let Some(status) = consume_fault(&state) {
        return (StatusCode::from_u16(status).unwrap(), Json(json!({}))).into_response();
    }
    let s = state.lock().unwrap();
    let store = match s.indexes.get(&name) {
        Some(v) => v,
        None => {
            return (StatusCode::NOT_FOUND, Json(json!({ "error": "no index" }))).into_response();
        }
    };
    let values: Vec<Value> = store.docs.keys().map(|id| json!({ "id": id })).collect();
    (StatusCode::OK, Json(json!({ "value": values }))).into_response()
}

#[derive(Debug, Deserialize)]
struct FaultSpec {
    count: usize,
    status: u16,
}

async fn azure_fault_post(
    State(state): State<SharedState>,
    Json(spec): Json<FaultSpec>,
) -> impl IntoResponse {
    let mut s = state.lock().unwrap();
    for _ in 0..spec.count {
        s.pending_faults.push(spec.status);
    }
    StatusCode::OK
}

// -----------------------------------------------------------------------
// Query parsing helpers
// -----------------------------------------------------------------------

/// Extract project name from JQL like "project = QUELCH ..."
fn extract_jql_project(jql: &str) -> Option<String> {
    let lower = jql.to_lowercase();
    let idx = lower.find("project")?;
    let rest = &jql[idx..];
    // Find the = sign
    let eq_idx = rest.find('=')?;
    let after_eq = rest[eq_idx + 1..].trim_start();
    // Take the first word (project key)
    let key: String = after_eq
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if key.is_empty() { None } else { Some(key) }
}

/// Extract updated >= timestamp from JQL like `updated >= "2026-03-15 14:30"`
fn extract_jql_updated(jql: &str) -> Option<String> {
    let lower = jql.to_lowercase();
    let idx = lower.find("updated")?;
    let rest = &jql[idx..];
    // Find >=
    let ge_idx = rest.find(">=")?;
    let after_ge = rest[ge_idx + 2..].trim_start();
    // Extract quoted value
    if let Some(stripped) = after_ge.strip_prefix('"') {
        let end = stripped.find('"')?;
        let ts = &stripped[..end];
        // Convert "2026-03-15 14:30" to comparable format "2026-03-15T14:30"
        Some(ts.replace(' ', "T"))
    } else {
        None
    }
}

/// Extract space name from CQL like `space = "QUELCH" ...`
fn extract_cql_space(cql: &str) -> Option<String> {
    let lower = cql.to_lowercase();
    let idx = lower.find("space")?;
    let rest = &cql[idx..];
    let eq_idx = rest.find('=')?;
    let after_eq = rest[eq_idx + 1..].trim_start();
    if let Some(stripped) = after_eq.strip_prefix('"') {
        let end = stripped.find('"')?;
        Some(stripped[..end].to_string())
    } else {
        let key: String = after_eq
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if key.is_empty() { None } else { Some(key) }
    }
}

/// Extract lastmodified >= timestamp from CQL
fn extract_cql_lastmodified(cql: &str) -> Option<String> {
    let lower = cql.to_lowercase();
    let idx = lower.find("lastmodified")?;
    let rest = &cql[idx..];
    let ge_idx = rest.find(">=")?;
    let after_ge = rest[ge_idx + 2..].trim_start();
    if let Some(stripped) = after_ge.strip_prefix('"') {
        let end = stripped.find('"')?;
        let ts = &stripped[..end];
        Some(ts.replace(' ', "T"))
    } else {
        None
    }
}

// -----------------------------------------------------------------------
// Router builder (testable, used by integration tests)
// -----------------------------------------------------------------------

/// Build the axum Router used by the mock server. `pub` so integration
/// tests outside this module can spin up an in-process instance.
pub fn build_router() -> Router {
    let state: SharedState = Arc::new(Mutex::new(AzureMockState::default()));

    Router::new()
        // Jira + Confluence routes:
        .route("/jira/rest/api/2/search", get(jira_search))
        .route(
            "/confluence/rest/api/content/search",
            get(confluence_search),
        )
        // Azure routes (all share the same state):
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
    println!("Jira project: QUELCH (17 issues)");
    println!("Confluence space: QUELCH (8 pages)");
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
    println!("      index: \"jira-issues\"");
    println!();
    println!("    - type: confluence");
    println!("      name: \"mock-confluence\"");
    println!("      url: \"http://localhost:{port}/confluence\"");
    println!("      auth:");
    println!("        pat: \"{MOCK_TOKEN}\"");
    println!("      spaces:");
    println!("        - \"QUELCH\"");
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
}
