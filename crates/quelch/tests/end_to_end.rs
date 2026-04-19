//! Full pipeline test with mock Jira + Confluence + Azure + deterministic embeddings.
//!
//! No real network. Spins up the axum mock server on a random port, wires the
//! engine against it, and asserts the v2 state file is written correctly.

use std::net::SocketAddr;
use std::time::Duration;

async fn spawn_mock() -> String {
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, quelch::mock::build_router())
            .await
            .unwrap();
    });
    format!("http://{}", addr)
}

#[tokio::test]
async fn sync_fills_azure_index_and_writes_v2_state() {
    let base = spawn_mock().await;

    let tmp = tempfile::TempDir::new().unwrap();
    let state_path = tmp.path().join(".quelch-state.json");
    let config = quelch::config::Config {
        azure: quelch::config::AzureConfig {
            endpoint: format!("{}/azure", base),
            api_key: "ignored".into(),
        },
        sources: vec![
            quelch::config::SourceConfig::Jira(quelch::config::JiraSourceConfig {
                name: "mock-jira".into(),
                url: format!("{}/jira", base),
                auth: quelch::config::AuthConfig::DataCenter {
                    pat: "mock-pat-token".into(),
                },
                projects: vec!["QUELCH".into(), "DEMO".into()],
                index: "jira-issues".into(),
            }),
            quelch::config::SourceConfig::Confluence(quelch::config::ConfluenceSourceConfig {
                name: "mock-conf".into(),
                url: format!("{}/confluence", base),
                auth: quelch::config::AuthConfig::DataCenter {
                    pat: "mock-pat-token".into(),
                },
                spaces: vec!["QUELCH".into(), "INFRA".into()],
                index: "confluence-pages".into(),
            }),
        ],
        sync: quelch::config::SyncConfig::default(),
    };

    // Point state file into the temp dir
    let mut config = config;
    config.sync.state_file = state_path.to_string_lossy().into();

    // Dummy embedding cfg — the schema isn't exercised by the mock Azure.
    let embedding = quelch::azure::schema::EmbeddingConfig {
        dimensions: 8,
        vectorizer_json: serde_json::json!({}),
    };
    let emb = quelch::sync::embedder::DeterministicEmbedder::new(8);

    let (_tx, mut rx) = tokio::sync::mpsc::channel::<quelch::sync::UiCommand>(1);
    quelch::sync::run_sync_with(
        &config,
        &state_path,
        &embedding,
        quelch::sync::IndexMode::AutoCreate,
        Some(&emb as &dyn quelch::sync::embedder::Embedder),
        None,
        &mut rx,
        1,
    )
    .await
    .unwrap();

    // Assert v2 state shape
    let raw = std::fs::read_to_string(&state_path).unwrap();
    let state: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(state.get("version").and_then(|v| v.as_u64()), Some(2));
    let sources = state.get("sources").unwrap().as_object().unwrap();
    let jira = sources.get("mock-jira").unwrap();
    let subs = jira.get("subsources").unwrap().as_object().unwrap();
    assert!(subs.contains_key("QUELCH"));
    assert!(subs.contains_key("DEMO"));

    // Assert docs ended up in Azure
    let client = reqwest::Client::new();
    let search = client
        .post(format!(
            "{}/azure/indexes/jira-issues/docs/search?api-version=2024-07-01",
            base
        ))
        .json(&serde_json::json!({ "search": "*" }))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = search.json().await.unwrap();
    let n = body.get("value").unwrap().as_array().unwrap().len();
    assert!(n > 0, "azure index should contain mock docs");
}

#[tokio::test]
async fn migrates_v1_state_file_on_load() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state_path = tmp.path().join("state.json");
    std::fs::write(
        &state_path,
        r#"{
            "version": 1,
            "sources": {
                "mock-jira": {
                    "last_cursor": "2026-01-15T10:00:00Z",
                    "last_sync_at": "2026-01-15T10:01:00Z",
                    "documents_synced": 42,
                    "sync_count": 3
                }
            }
        }"#,
    )
    .unwrap();

    let expected = vec![(
        "mock-jira".to_string(),
        vec!["QUELCH".to_string(), "DEMO".to_string()],
    )];
    let state = quelch::sync::state::SyncState::load(&state_path, &expected).unwrap();
    assert_eq!(state.version, 2);
    let src = state.get_source("mock-jira");
    assert!(src.subsources.contains_key("QUELCH"));
    assert!(src.subsources.contains_key("DEMO"));
}

#[tokio::test]
async fn injected_fault_causes_retry_then_success() {
    let base = spawn_mock().await;
    let client = reqwest::Client::new();

    client
        .post(format!("{}/azure/_fault", base))
        .json(&serde_json::json!({ "count": 1, "status": 503 }))
        .send()
        .await
        .unwrap();

    let search = quelch::azure::SearchClient::new(&format!("{}/azure", base), "ignored");
    // First call hits fault, SearchClient's internal retry recovers.
    search
        .create_index(&quelch::azure::schema::IndexSchema {
            name: "fault-idx".into(),
            fields: vec![],
            ..Default::default()
        })
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(search.index_exists("fault-idx").await.unwrap());
}
