use axum::{
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{SharedState, consume_fault};

pub(super) async fn azure_index_get(
    State(state): State<SharedState>,
    AxumPath(name): AxumPath<String>,
) -> impl IntoResponse {
    if let Some(status) = consume_fault(&state) {
        return (StatusCode::from_u16(status).unwrap(), Json(json!({}))).into_response();
    }
    let s = state.lock().unwrap();
    if s.azure.indexes.contains_key(&name) {
        (StatusCode::OK, Json(json!({ "name": name }))).into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response()
    }
}

pub(super) async fn azure_index_put(
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
        .azure
        .indexes
        .entry(name.clone())
        .or_default();
    (StatusCode::CREATED, Json(json!({ "name": name }))).into_response()
}

/// Azure's real `create_index` posts the schema (with `name` inside) to
/// `/indexes?api-version=...`. Honor that alongside the `PUT /{name}` we
/// already have.
pub(super) async fn azure_indexes_collection_post(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    if let Some(status) = consume_fault(&state) {
        return (StatusCode::from_u16(status).unwrap(), Json(json!({}))).into_response();
    }
    let name = match body.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "missing 'name' field" })),
            )
                .into_response();
        }
    };
    state
        .lock()
        .unwrap()
        .azure
        .indexes
        .entry(name.clone())
        .or_default();
    (StatusCode::CREATED, Json(json!({ "name": name }))).into_response()
}

pub(super) async fn azure_index_delete(
    State(state): State<SharedState>,
    AxumPath(name): AxumPath<String>,
) -> impl IntoResponse {
    if let Some(status) = consume_fault(&state) {
        return (StatusCode::from_u16(status).unwrap(), Json(json!({}))).into_response();
    }
    state.lock().unwrap().azure.indexes.remove(&name);
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Debug, Deserialize)]
pub(super) struct AzureBatch {
    value: Vec<Value>,
}

pub(super) async fn azure_index_docs_post(
    State(state): State<SharedState>,
    AxumPath(name): AxumPath<String>,
    Json(batch): Json<AzureBatch>,
) -> impl IntoResponse {
    if let Some(status) = consume_fault(&state) {
        return (StatusCode::from_u16(status).unwrap(), Json(json!({}))).into_response();
    }
    let mut s = state.lock().unwrap();
    let store = s.azure.indexes.entry(name).or_default();
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
pub(super) struct AzureSearchBody {
    search: Option<String>,
}

pub(super) async fn azure_index_search_post(
    State(state): State<SharedState>,
    AxumPath(name): AxumPath<String>,
    Json(body): Json<AzureSearchBody>,
) -> impl IntoResponse {
    if let Some(status) = consume_fault(&state) {
        return (StatusCode::from_u16(status).unwrap(), Json(json!({}))).into_response();
    }
    let s = state.lock().unwrap();
    let store = match s.azure.indexes.get(&name) {
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
pub(super) async fn azure_index_docs_list(
    State(state): State<SharedState>,
    AxumPath(name): AxumPath<String>,
) -> impl IntoResponse {
    if let Some(status) = consume_fault(&state) {
        return (StatusCode::from_u16(status).unwrap(), Json(json!({}))).into_response();
    }
    let s = state.lock().unwrap();
    let store = match s.azure.indexes.get(&name) {
        Some(v) => v,
        None => {
            return (StatusCode::NOT_FOUND, Json(json!({ "error": "no index" }))).into_response();
        }
    };
    let values: Vec<Value> = store.docs.keys().map(|id| json!({ "id": id })).collect();
    (StatusCode::OK, Json(json!({ "value": values }))).into_response()
}

/// GET /azure/indexes/{name}/docs/$count — plain-text doc count, with
/// optional `$filter=field eq 'value'` (joined by ` and `). Returns the
/// number as a bare integer in the response body, matching Azure's
/// real contract.
#[derive(Debug, Deserialize)]
pub(super) struct CountParams {
    #[serde(rename = "$filter")]
    filter: Option<String>,
}

pub(super) async fn azure_index_docs_count(
    State(state): State<SharedState>,
    AxumPath(name): AxumPath<String>,
    Query(params): Query<CountParams>,
) -> impl IntoResponse {
    if let Some(status) = consume_fault(&state) {
        return (StatusCode::from_u16(status).unwrap(), String::new()).into_response();
    }
    let s = state.lock().unwrap();
    let store = match s.azure.indexes.get(&name) {
        Some(v) => v,
        None => return (StatusCode::NOT_FOUND, String::from("0")).into_response(),
    };
    let filters = params
        .filter
        .as_deref()
        .map(parse_simple_filter)
        .unwrap_or_default();
    let count = store
        .docs
        .values()
        .filter(|doc| {
            filters
                .iter()
                .all(|(field, value)| doc_field_eq(doc, field, value))
        })
        .count();
    (StatusCode::OK, count.to_string()).into_response()
}

/// Parse a tiny subset of OData `$filter` — just `field eq 'value'` clauses
/// joined by ` and `. Any other expression returns an empty filter (matches
/// everything), which is safer than failing the mock for unrecognised syntax.
fn parse_simple_filter(expr: &str) -> Vec<(String, String)> {
    expr.split(" and ")
        .filter_map(|clause| {
            let clause = clause.trim();
            let (field, rest) = clause.split_once(" eq ")?;
            let rest = rest.trim();
            let value = rest.strip_prefix('\'')?.strip_suffix('\'')?;
            Some((field.trim().to_string(), value.to_string()))
        })
        .collect()
}

fn doc_field_eq(doc: &Value, field: &str, value: &str) -> bool {
    doc.get(field)
        .and_then(|v| v.as_str())
        .map(|s| s == value)
        .unwrap_or(false)
}

#[derive(Debug, Deserialize)]
pub(super) struct FaultSpec {
    count: usize,
    status: u16,
}

pub(super) async fn azure_fault_post(
    State(state): State<SharedState>,
    Json(spec): Json<FaultSpec>,
) -> impl IntoResponse {
    let mut s = state.lock().unwrap();
    for _ in 0..spec.count {
        s.azure.pending_faults.push(spec.status);
    }
    StatusCode::OK
}
