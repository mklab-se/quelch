use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{SharedState, check_auth};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct JiraSearchParams {
    jql: Option<String>,
    start_at: Option<u64>,
    max_results: Option<u64>,
    #[serde(default)]
    #[allow(dead_code)]
    fields: Option<String>,
}

pub(super) async fn jira_search(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(params): Query<JiraSearchParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    check_auth(&headers)?;

    let all_issues: Vec<Value> = state.lock().unwrap().jira_issues.clone();
    let jql = params.jql.unwrap_or_default();

    // Filter by project if JQL contains "project = X"
    let filtered: Vec<Value> = all_issues
        .into_iter()
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
        .collect();

    Ok(Json(json!({
        "expand": "schema,names",
        "startAt": start_at,
        "maxResults": max_results,
        "total": total,
        "issues": page
    })))
}

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
