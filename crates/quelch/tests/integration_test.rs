//! Integration tests for quelch sync engine using wiremock-based mock servers.
//!
//! Each test spins up a local WireMock server to simulate Jira/Confluence and
//! Azure AI Search, then verifies that `run_sync` or `setup_indexes` behave
//! correctly end-to-end without any real network calls.

use std::io::Write as _;

use tempfile::{NamedTempFile, TempDir};
use wiremock::matchers::{method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use quelch::azure::schema::EmbeddingConfig;
use quelch::config::{Config, load_config};
use quelch::sync::state::SyncState;
use quelch::sync::{IndexMode, run_sync, setup_indexes};

fn test_embedding() -> EmbeddingConfig {
    EmbeddingConfig {
        dimensions: 3072,
        vectorizer_json: serde_json::json!({
            "name": "test-vectorizer",
            "kind": "azureOpenAI",
            "azureOpenAIParameters": {
                "resourceUri": "https://test.openai.azure.com",
                "deploymentId": "text-embedding-3-large",
                "modelName": "text-embedding-3-large"
            }
        }),
    }
}

// ---------------------------------------------------------------------------
// Config helpers
// ---------------------------------------------------------------------------

/// Write a Jira DC config pointing at `jira_url` and `azure_url`.
/// Returns the NamedTempFile (config) and TempDir (for state file).
fn write_dc_config(jira_url: &str, azure_url: &str) -> (NamedTempFile, TempDir) {
    let state_dir = TempDir::new().expect("temp dir");
    let mut f = NamedTempFile::new().expect("named temp file");
    let yaml = format!(
        r#"
azure:
  endpoint: "{azure_url}"
  api_key: "test-azure-key"
sources:
  - type: jira
    name: "test-jira"
    url: "{jira_url}"
    auth:
      pat: "test-pat-token"
    projects:
      - "TEST"
    index: "jira-issues"
"#
    );
    f.write_all(yaml.as_bytes()).expect("write config");
    (f, state_dir)
}

/// Write a Jira Cloud config pointing at `jira_url` and `azure_url`.
fn write_cloud_config(jira_url: &str, azure_url: &str) -> (NamedTempFile, TempDir) {
    let state_dir = TempDir::new().expect("temp dir");
    let mut f = NamedTempFile::new().expect("named temp file");
    let yaml = format!(
        r#"
azure:
  endpoint: "{azure_url}"
  api_key: "test-azure-key"
sources:
  - type: jira
    name: "test-jira-cloud"
    url: "{jira_url}"
    auth:
      email: "user@example.com"
      api_token: "cloud-token"
    projects:
      - "TEST"
    index: "jira-issues"
"#
    );
    f.write_all(yaml.as_bytes()).expect("write config");
    (f, state_dir)
}

/// Write a Confluence DC config pointing at `conf_url` and `azure_url`.
fn write_confluence_config(conf_url: &str, azure_url: &str) -> (NamedTempFile, TempDir) {
    let state_dir = TempDir::new().expect("temp dir");
    let mut f = NamedTempFile::new().expect("named temp file");
    let yaml = format!(
        r#"
azure:
  endpoint: "{azure_url}"
  api_key: "test-azure-key"
sources:
  - type: confluence
    name: "test-confluence"
    url: "{conf_url}"
    auth:
      pat: "test-conf-pat"
    spaces:
      - "DOCS"
    index: "confluence-pages"
"#
    );
    f.write_all(yaml.as_bytes()).expect("write config");
    (f, state_dir)
}

// ---------------------------------------------------------------------------
// JSON response builders
// ---------------------------------------------------------------------------

fn jira_dc_issue(key: &str, summary: &str, updated: &str) -> serde_json::Value {
    serde_json::json!({
        "key": key,
        "fields": {
            "summary": summary,
            "description": "A plain text description for DC.",
            "status": {
                "name": "Open",
                "statusCategory": {
                    "name": "To Do",
                    "id": 2,
                    "key": "new"
                }
            },
            "priority": { "name": "Medium" },
            "assignee": { "displayName": "Alice" },
            "reporter": { "displayName": "Bob" },
            "issuetype": { "name": "Bug" },
            "labels": ["backend"],
            "created": "2025-01-10T08:00:00.000+0000",
            "updated": updated,
            "comment": {
                "comments": [
                    { "body": "First comment on the DC issue." }
                ]
            },
            "project": { "key": "TEST" }
        }
    })
}

fn jira_cloud_issue(key: &str, summary: &str, updated: &str) -> serde_json::Value {
    serde_json::json!({
        "key": key,
        "fields": {
            "summary": summary,
            "description": {
                "version": 1,
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "ADF description content." }]
                }]
            },
            "status": {
                "name": "In Progress",
                "statusCategory": {
                    "name": "In Progress",
                    "id": 4,
                    "key": "indeterminate"
                }
            },
            "priority": { "name": "High" },
            "assignee": { "displayName": "Charlie" },
            "reporter": { "displayName": "Dana" },
            "issuetype": { "name": "Story" },
            "labels": ["cloud", "feature"],
            "created": "2025-02-01T09:00:00.000+0000",
            "updated": updated,
            "comment": {
                "comments": [{
                    "body": {
                        "version": 1,
                        "type": "doc",
                        "content": [{
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "ADF comment text." }]
                        }]
                    }
                }]
            },
            "project": { "key": "TEST" }
        }
    })
}

fn jira_dc_search_response(issues: Vec<serde_json::Value>) -> serde_json::Value {
    let total = issues.len() as u64;
    serde_json::json!({
        "startAt": 0,
        "maxResults": 100,
        "total": total,
        "issues": issues
    })
}

fn jira_cloud_search_response(issues: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "issues": issues,
        "isLast": true
    })
}

fn azure_index_response(keys: &[&str]) -> serde_json::Value {
    let value: Vec<serde_json::Value> = keys
        .iter()
        .map(|k| {
            serde_json::json!({
                "key": k,
                "status": true,
                "statusCode": 201,
                "errorMessage": null
            })
        })
        .collect();
    serde_json::json!({ "value": value })
}

fn confluence_page(
    id: &str,
    title: &str,
    space_key: &str,
    body_html: &str,
    updated: &str,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "title": title,
        "space": { "key": space_key },
        "body": {
            "storage": {
                "value": body_html
            }
        },
        "version": {
            "when": updated,
            "by": {
                "displayName": "Page Author",
                "accountId": "uid-123"
            }
        },
        "history": {
            "createdDate": "2025-01-01T00:00:00.000Z"
        },
        "ancestors": [
            { "title": "Parent Page" }
        ],
        "metadata": {
            "labels": {
                "results": [
                    { "name": "guide" },
                    { "name": "docs" }
                ]
            }
        },
        "_links": {
            "webui": format!("/spaces/{space_key}/pages/{id}")
        }
    })
}

fn confluence_search_response(results: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "results": results,
        "_links": {}
    })
}

// ---------------------------------------------------------------------------
// Test 1: full_sync_jira_dc_to_azure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn full_sync_jira_dc_to_azure() {
    let jira_server = MockServer::start().await;
    let azure_server = MockServer::start().await;

    // --- Jira DC search: returns 2 issues ---
    let issues = vec![
        jira_dc_issue("TEST-1", "First issue", "2025-01-15T10:30:00.000+0000"),
        jira_dc_issue("TEST-2", "Second issue", "2025-01-16T11:00:00.000+0000"),
    ];
    Mock::given(method("GET"))
        .and(path("/rest/api/2/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jira_dc_search_response(issues)))
        .mount(&jira_server)
        .await;

    // --- Azure: GET index returns 404 (doesn't exist) ---
    Mock::given(method("GET"))
        .and(path_regex(r"^/indexes/jira-issues"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&azure_server)
        .await;

    // --- Azure: POST /indexes returns 201 (created) ---
    Mock::given(method("POST"))
        .and(path("/indexes"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "name": "jira-issues"
        })))
        .mount(&azure_server)
        .await;

    // --- Azure: POST docs/index returns 200 success ---
    Mock::given(method("POST"))
        .and(path_regex(r"^/indexes/jira-issues/docs/index"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(azure_index_response(&[
                "test-jira-TEST-1",
                "test-jira-TEST-2",
            ])),
        )
        .mount(&azure_server)
        .await;

    let (config_file, state_dir) = write_dc_config(&jira_server.uri(), &azure_server.uri());
    let config = load_config(config_file.path()).expect("load config");
    let state_path = state_dir.path().join("state.json");

    run_sync(
        &config,
        &state_path,
        &test_embedding(),
        IndexMode::AutoCreate,
        None,
        None,
    )
    .await
    .expect("run_sync failed");

    // Verify state was persisted with cursor and doc count
    let state = SyncState::load(&state_path, &[]).expect("load state");
    let source_state = state.get_source("test-jira");
    let sub = source_state
        .subsources
        .get("TEST")
        .expect("TEST subsource state");
    assert!(sub.last_cursor.is_some(), "cursor should be persisted");
    assert_eq!(sub.documents_synced, 2, "should have synced 2 documents");
    assert_eq!(source_state.sync_count, 1, "should have run one sync batch");
}

// ---------------------------------------------------------------------------
// Test 2: full_sync_jira_cloud_to_azure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn full_sync_jira_cloud_to_azure() {
    let jira_server = MockServer::start().await;
    let azure_server = MockServer::start().await;

    // --- Jira Cloud search: returns 2 issues with isLast=true ---
    let issues = vec![
        jira_cloud_issue("TEST-10", "Cloud issue one", "2025-02-10T09:00:00.000+0000"),
        jira_cloud_issue("TEST-11", "Cloud issue two", "2025-02-11T10:30:00.000+0000"),
    ];
    Mock::given(method("GET"))
        .and(path("/rest/api/3/search/jql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jira_cloud_search_response(issues)))
        .mount(&jira_server)
        .await;

    // --- Azure: GET index returns 200 (already exists) ---
    Mock::given(method("GET"))
        .and(path_regex(r"^/indexes/jira-issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "jira-issues"
        })))
        .mount(&azure_server)
        .await;

    // --- Azure: POST docs/index returns 200 success ---
    Mock::given(method("POST"))
        .and(path_regex(r"^/indexes/jira-issues/docs/index"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(azure_index_response(&[
                "test-jira-cloud-TEST-10",
                "test-jira-cloud-TEST-11",
            ])),
        )
        .mount(&azure_server)
        .await;

    let (config_file, state_dir) = write_cloud_config(&jira_server.uri(), &azure_server.uri());
    let config = load_config(config_file.path()).expect("load config");
    let state_path = state_dir.path().join("state.json");

    run_sync(
        &config,
        &state_path,
        &test_embedding(),
        IndexMode::AutoCreate,
        None,
        None,
    )
    .await
    .expect("run_sync failed");

    let state = SyncState::load(&state_path, &[]).expect("load state");
    let source_state = state.get_source("test-jira-cloud");
    let sub = source_state
        .subsources
        .get("TEST")
        .expect("TEST subsource state");
    assert_eq!(
        sub.documents_synced, 2,
        "should have synced 2 cloud documents"
    );
}

// ---------------------------------------------------------------------------
// Test 3: incremental_sync_uses_cursor
// ---------------------------------------------------------------------------

#[tokio::test]
async fn incremental_sync_uses_cursor() {
    let jira_server = MockServer::start().await;
    let azure_server = MockServer::start().await;

    // --- Jira returns empty results (nothing new since the cursor) ---
    Mock::given(method("GET"))
        .and(path("/rest/api/2/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jira_dc_search_response(vec![])))
        .mount(&jira_server)
        .await;

    // --- Azure: index exists ---
    Mock::given(method("GET"))
        .and(path_regex(r"^/indexes/jira-issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "jira-issues"
        })))
        .mount(&azure_server)
        .await;

    let (config_file, state_dir) = write_dc_config(&jira_server.uri(), &azure_server.uri());
    let config = load_config(config_file.path()).expect("load config");
    let state_path = state_dir.path().join("state.json");

    // Pre-populate state with an existing cursor
    let mut pre_state = SyncState::default();
    let prior_cursor = chrono::DateTime::parse_from_rfc3339("2025-01-10T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    pre_state.update_subsource("test-jira", "TEST", prior_cursor, 5, None);
    pre_state.save(&state_path).expect("save pre-state");

    run_sync(
        &config,
        &state_path,
        &test_embedding(),
        IndexMode::AutoCreate,
        None,
        None,
    )
    .await
    .expect("run_sync failed");

    // Verify: no docs were pushed, count unchanged
    let state = SyncState::load(&state_path, &[]).expect("load state");
    let source_state = state.get_source("test-jira");
    let sub = source_state
        .subsources
        .get("TEST")
        .expect("TEST subsource state");
    assert_eq!(
        sub.documents_synced, 5,
        "doc count should be unchanged when no new results"
    );

    // Verify Azure docs/index endpoint was NOT called
    let calls = azure_server.received_requests().await.unwrap();
    let push_calls = calls
        .iter()
        .filter(|req| req.url.path().contains("docs/index"))
        .count();
    assert_eq!(push_calls, 0, "should not push docs when nothing new");
}

// ---------------------------------------------------------------------------
// Test 3b: repeated_sync_does_not_re_push_same_docs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn repeated_sync_does_not_re_push_same_docs() {
    let jira_server = MockServer::start().await;
    let azure_server = MockServer::start().await;

    // Jira returns 2 issues with specific timestamps
    let issues = vec![
        jira_dc_issue("TEST-1", "First issue", "2025-01-15T10:30:00.000+0000"),
        jira_dc_issue("TEST-2", "Second issue", "2025-01-16T11:00:00.000+0000"),
    ];

    // Mount Jira mock — will be called multiple times
    Mock::given(method("GET"))
        .and(path("/rest/api/2/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jira_dc_search_response(issues)))
        .mount(&jira_server)
        .await;

    // Azure: index exists
    Mock::given(method("GET"))
        .and(path_regex(r"^/indexes/jira-issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "jira-issues"
        })))
        .mount(&azure_server)
        .await;

    // Azure: docs/index — track how many times it's called
    Mock::given(method("POST"))
        .and(path_regex(r"^/indexes/jira-issues/docs/index"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(azure_index_response(&[
                "test-jira-TEST-1",
                "test-jira-TEST-2",
            ])),
        )
        .mount(&azure_server)
        .await;

    let (config_file, state_dir) = write_dc_config(&jira_server.uri(), &azure_server.uri());
    let config = load_config(config_file.path()).expect("load config");
    let state_path = state_dir.path().join("state.json");

    // First sync — should push 2 docs
    run_sync(
        &config,
        &state_path,
        &test_embedding(),
        IndexMode::AutoCreate,
        None,
        None,
    )
    .await
    .expect("first sync failed");

    let state = SyncState::load(&state_path, &[]).expect("load state");
    assert_eq!(
        state
            .get_source("test-jira")
            .subsources
            .get("TEST")
            .map(|s| s.documents_synced)
            .unwrap_or(0),
        2
    );

    // Second sync — Jira returns same issues (updated >= cursor is inclusive),
    // but sync should filter them out and NOT push anything
    run_sync(
        &config,
        &state_path,
        &test_embedding(),
        IndexMode::AutoCreate,
        None,
        None,
    )
    .await
    .expect("second sync failed");

    // Doc count should still be 2, not 4
    let state = SyncState::load(&state_path, &[]).expect("load state");
    assert_eq!(
        state
            .get_source("test-jira")
            .subsources
            .get("TEST")
            .map(|s| s.documents_synced)
            .unwrap_or(0),
        2,
        "should not re-count already synced docs"
    );

    // Azure docs/index should have been called only ONCE (first sync only)
    let calls = azure_server.received_requests().await.unwrap();
    let push_calls = calls
        .iter()
        .filter(|req| req.url.path().contains("docs/index"))
        .count();
    assert_eq!(
        push_calls, 1,
        "should only push docs once, not on repeated syncs"
    );
}

// ---------------------------------------------------------------------------
// Test 4: setup_indexes_creates_missing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn setup_indexes_creates_missing() {
    let azure_server = MockServer::start().await;

    // GET index returns 404
    Mock::given(method("GET"))
        .and(path_regex(r"^/indexes/jira-issues"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&azure_server)
        .await;

    // POST /indexes returns 201 (created)
    Mock::given(method("POST"))
        .and(path("/indexes"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "name": "jira-issues"
        })))
        .mount(&azure_server)
        .await;

    let (config_file, _state_dir) = write_dc_config("http://jira.example.com", &azure_server.uri());
    let config = load_config(config_file.path()).expect("load config");

    let created = setup_indexes(&config, &test_embedding(), IndexMode::AutoCreate)
        .await
        .expect("setup_indexes failed");

    assert!(
        created.contains(&"jira-issues".to_string()),
        "jira-issues should be listed as created"
    );

    // Verify the create endpoint was called
    let calls = azure_server.received_requests().await.unwrap();
    let create_calls = calls
        .iter()
        .filter(|req| req.method == wiremock::http::Method::POST && req.url.path() == "/indexes")
        .count();
    assert_eq!(create_calls, 1, "should call POST /indexes once");
}

// ---------------------------------------------------------------------------
// Test 5: setup_indexes_skips_existing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn setup_indexes_skips_existing() {
    let azure_server = MockServer::start().await;

    // GET index returns 200 (already exists)
    Mock::given(method("GET"))
        .and(path_regex(r"^/indexes/jira-issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "jira-issues"
        })))
        .mount(&azure_server)
        .await;

    let (config_file, _state_dir) = write_dc_config("http://jira.example.com", &azure_server.uri());
    let config = load_config(config_file.path()).expect("load config");

    let created = setup_indexes(&config, &test_embedding(), IndexMode::AutoCreate)
        .await
        .expect("setup_indexes failed");

    assert!(
        created.is_empty(),
        "nothing should be created when index already exists"
    );

    // Verify POST /indexes was NOT called
    let calls = azure_server.received_requests().await.unwrap();
    let create_calls = calls
        .iter()
        .filter(|req| req.method == wiremock::http::Method::POST && req.url.path() == "/indexes")
        .count();
    assert_eq!(create_calls, 0, "should not call create when index exists");
}

// ---------------------------------------------------------------------------
// Test 6: sync_with_confluence_chunking
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sync_with_confluence_chunking() {
    let conf_server = MockServer::start().await;
    let azure_server = MockServer::start().await;

    // Page body with heading-based content — produces multiple chunks
    let body_html = r#"
        <p>Introductory paragraph at the top of the page.</p>
        <h1>Section One</h1>
        <p>Content under section one. This covers the first major topic.</p>
        <h2>Section Two</h2>
        <p>Content under section two. Another important topic is covered here.</p>
        <h3>Section Three</h3>
        <p>Content under section three. Details about the third area.</p>
    "#;

    let page = confluence_page(
        "98765",
        "My Integration Guide",
        "DOCS",
        body_html,
        "2025-03-10T14:00:00.000Z",
    );

    // DC Confluence search endpoint
    Mock::given(method("GET"))
        .and(path("/rest/api/content/search"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(confluence_search_response(vec![page])),
        )
        .mount(&conf_server)
        .await;

    // Azure: index exists
    Mock::given(method("GET"))
        .and(path_regex(r"^/indexes/confluence-pages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "confluence-pages"
        })))
        .mount(&azure_server)
        .await;

    // Azure: docs/index returns 200 (capture the call)
    Mock::given(method("POST"))
        .and(path_regex(r"^/indexes/confluence-pages/docs/index"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "value": []
        })))
        .mount(&azure_server)
        .await;

    let (config_file, state_dir) = write_confluence_config(&conf_server.uri(), &azure_server.uri());
    let config = load_config(config_file.path()).expect("load config");
    let state_path = state_dir.path().join("state.json");

    run_sync(
        &config,
        &state_path,
        &test_embedding(),
        IndexMode::AutoCreate,
        None,
        None,
    )
    .await
    .expect("run_sync failed");

    // Verify state: docs pushed (should be multiple chunks from heading-based split)
    let state = SyncState::load(&state_path, &[]).expect("load state");
    let source_state = state.get_source("test-confluence");
    let sub = source_state
        .subsources
        .get("DOCS")
        .expect("DOCS subsource state");
    assert!(
        sub.documents_synced > 1,
        "heading-based chunking should produce multiple chunks; got {}",
        sub.documents_synced
    );
    assert!(sub.last_cursor.is_some(), "cursor should be set");

    // Verify Azure push was called
    let calls = azure_server.received_requests().await.unwrap();
    let push_calls = calls
        .iter()
        .filter(|req| req.url.path().contains("docs/index"))
        .count();
    assert!(push_calls >= 1, "should have pushed docs to Azure");
}
