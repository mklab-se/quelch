use axum::{extract::State, http::StatusCode, response::Json};
use serde::Deserialize;
use serde_json::json;

use super::SharedState;

#[derive(Debug, Deserialize)]
pub(super) struct SimUpsertIssue {
    project: String,
    key: String,
    summary: String,
    #[serde(default)]
    description: String,
}

pub(super) async fn sim_upsert_issue(
    State(state): State<SharedState>,
    Json(body): Json<SimUpsertIssue>,
) -> (StatusCode, Json<serde_json::Value>) {
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f+0000")
        .to_string();
    let mut s = state.lock().unwrap();
    if let Some(existing) = s
        .jira_issues
        .iter_mut()
        .find(|i| i["key"].as_str() == Some(body.key.as_str()))
    {
        existing["fields"]["summary"] = json!(body.summary);
        existing["fields"]["description"] = json!(body.description);
        existing["fields"]["updated"] = json!(now);
    } else {
        s.jira_issues.push(json!({
            "id": body.key.clone(),
            "key": body.key.clone(),
            "fields": {
                "summary": body.summary,
                "description": body.description,
                "status": {
                    "name": "Open",
                    "statusCategory": { "name": "New", "id": 2, "key": "new" }
                },
                "priority": { "name": "Medium" },
                "issuetype": { "name": "Story" },
                "project": {
                    "id": body.project.clone(),
                    "key": body.project.clone(),
                    "name": body.project.clone(),
                },
                "labels": Vec::<String>::new(),
                "created": now.clone(),
                "updated": now,
                "comment": { "comments": [], "maxResults": 0, "total": 0, "startAt": 0 }
            }
        }));
    }
    (StatusCode::OK, Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub(super) struct SimUpsertPage {
    space: String,
    id: String,
    title: String,
    #[serde(default)]
    body: String,
}

pub(super) async fn sim_upsert_page(
    State(state): State<SharedState>,
    Json(body): Json<SimUpsertPage>,
) -> (StatusCode, Json<serde_json::Value>) {
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f+0000")
        .to_string();
    let mut s = state.lock().unwrap();
    if let Some(existing) = s
        .confluence_pages
        .iter_mut()
        .find(|p| p["id"].as_str() == Some(body.id.as_str()))
    {
        existing["title"] = json!(body.title);
        if let Some(storage) = existing
            .get_mut("body")
            .and_then(|b| b.get_mut("storage"))
            .and_then(|s| s.get_mut("value"))
        {
            *storage = json!(body.body);
        }
        existing["version"]["when"] = json!(now);
    } else {
        s.confluence_pages.push(json!({
            "id": body.id,
            "type": "page",
            "status": "current",
            "title": body.title,
            "space": { "key": body.space, "name": "sim" },
            "body": { "storage": { "value": body.body, "representation": "storage" } },
            "version": { "number": 1, "when": now.clone() },
            "history": { "createdDate": now, "latest": true },
            "ancestors": [],
            "metadata": { "labels": { "results": [], "start": 0, "limit": 200, "size": 0 } },
            "_links": {}
        }));
    }
    (StatusCode::OK, Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub(super) struct SimAddComment {
    key: String,
    body: String,
    #[serde(default)]
    author: String,
}

pub(super) async fn sim_add_comment(
    State(state): State<SharedState>,
    Json(body): Json<SimAddComment>,
) -> (StatusCode, Json<serde_json::Value>) {
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f+0000")
        .to_string();
    let mut s = state.lock().unwrap();
    if let Some(issue) = s
        .jira_issues
        .iter_mut()
        .find(|i| i["key"].as_str() == Some(body.key.as_str()))
    {
        let comment = json!({
            "author": { "displayName": body.author },
            "body": body.body,
            "created": now,
            "updated": now,
        });
        if let Some(comments) = issue
            .get_mut("fields")
            .and_then(|f| f.get_mut("comment"))
            .and_then(|c| c.get_mut("comments"))
            .and_then(|v| v.as_array_mut())
        {
            comments.push(comment);
        }
        issue["fields"]["updated"] = json!(now);
        (StatusCode::OK, Json(json!({ "ok": true })))
    } else {
        (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" })))
    }
}
