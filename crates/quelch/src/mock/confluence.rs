use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{SharedState, check_auth};

#[derive(Debug, Deserialize)]
pub(super) struct ConfluenceSearchParams {
    cql: Option<String>,
    start: Option<u64>,
    limit: Option<u64>,
    #[serde(default)]
    #[allow(dead_code)]
    expand: Option<String>,
}

pub(super) async fn confluence_search(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(params): Query<ConfluenceSearchParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    check_auth(&headers)?;

    let all_pages: Vec<Value> = state.lock().unwrap().confluence_pages.clone();
    let cql = params.cql.unwrap_or_default();

    let filtered: Vec<Value> = all_pages
        .into_iter()
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
