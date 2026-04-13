pub mod data;

use axum::{
    Router,
    extract::Query,
    http::{HeaderMap, StatusCode},
    response::Json,
    routing::get,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;

const MOCK_TOKEN: &str = "mock-pat-token";

// ---------------------------------------------------------------------------
// Auth helper
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Jira endpoint
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Confluence endpoint
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Query parsing helpers
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Server entry point
// ---------------------------------------------------------------------------

/// Start the mock Jira DC + Confluence DC server.
pub async fn run_mock_server(port: u16) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/jira/rest/api/2/search", get(jira_search))
        .route(
            "/confluence/rest/api/content/search",
            get(confluence_search),
        );

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
