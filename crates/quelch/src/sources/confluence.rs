//! Confluence source connector — v2 implementation.
//!
//! Implements the [`SourceConnector`] trait for both Confluence Cloud and Server/Data Center.
//! Uses the Confluence REST API (`/rest/api/content/search`) for CQL-based page queries.
//!
//! See `docs/sync.md` for the sync algorithm and CQL format requirements.
//! See `docs/architecture.md` "Confluence page (`confluence_pages`)" for the canonical document shape.

use std::collections::HashMap;

use anyhow::{Context, anyhow};
use chrono::{DateTime, Utc};
use reqwest_middleware::ClientWithMiddleware;
use serde_json::{Value, json};
use tracing::debug;

use super::{BackfillCheckpoint, Companions, FetchPage, SourceConnector, SourceDocument};
use crate::config::ConfluenceSourceConfig;

// ---------------------------------------------------------------------------
// Connector struct
// ---------------------------------------------------------------------------

/// Confluence source connector.
///
/// Implements [`SourceConnector`] for both Confluence Cloud and Data Center.
/// Constructed via [`ConfluenceConnector::new`]; the HTTP client is injected
/// (built once by the ingest worker with rate-limit middleware).
#[derive(Clone)]
pub struct ConfluenceConnector {
    /// Source name from config — used as the stable identifier in Cosmos IDs.
    source_name: String,
    /// Base URL, e.g. `https://example.atlassian.net/wiki` (Cloud) or
    /// `https://confluence.internal.example` (Data Center).
    base_url: String,
    /// `Authorization` header value, computed once from config at construction.
    auth_header: String,
    /// Space keys to ingest (subsources).
    spaces: Vec<String>,
    /// Rate-limit-aware HTTP client (shared with other connectors / workers).
    client: ClientWithMiddleware,
    /// Primary container for pages (from config override or default).
    container: String,
}

impl ConfluenceConnector {
    /// Create a new `ConfluenceConnector`.
    ///
    /// # Arguments
    ///
    /// * `config` — Confluence source config from `quelch.yaml`.
    /// * `client` — pre-built `reqwest_middleware::ClientWithMiddleware` (injected by worker).
    pub fn new(
        config: &ConfluenceSourceConfig,
        client: ClientWithMiddleware,
    ) -> anyhow::Result<Self> {
        let base_url = config.url.trim_end_matches('/').to_owned();
        let auth_header = config.auth.authorization_header();
        let container = config
            .container
            .clone()
            .unwrap_or_else(|| "confluence-pages".to_string());

        Ok(Self {
            source_name: config.name.clone(),
            base_url,
            auth_header,
            spaces: config.spaces.clone(),
            client,
            container,
        })
    }

    // -----------------------------------------------------------------------
    // HTTP helpers
    // -----------------------------------------------------------------------

    /// GET `{base_url}{path}` with auth headers.
    async fn get(&self, path: &str) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .send()
            .await
            .with_context(|| format!("GET {path}"))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp
                .bytes()
                .await
                .map(|b| String::from_utf8_lossy(&b).to_string())
                .unwrap_or_default();
            return Err(anyhow!("GET {path} returned {status}: {text}"));
        }

        let bytes = resp
            .bytes()
            .await
            .with_context(|| format!("read response body from {path}"))?;
        serde_json::from_slice(&bytes).with_context(|| format!("deserialize response from {path}"))
    }

    // -----------------------------------------------------------------------
    // Shared pagination helpers
    // -----------------------------------------------------------------------

    /// Execute a paginated CQL search and return a [`FetchPage`].
    ///
    /// `start` is the offset-based pagination start (0-indexed).
    async fn fetch_pages_page(
        &self,
        cql: &str,
        start: usize,
        batch_size: usize,
    ) -> anyhow::Result<FetchPage> {
        // expand: body.storage, version, ancestors, metadata.labels, history
        let expand = "body.storage,version,ancestors,metadata.labels,history";
        let path = format!(
            "/rest/api/content/search?cql={}&start={}&limit={}&expand={}",
            urlencoding::encode(cql),
            start,
            batch_size,
            urlencoding::encode(expand),
        );

        let resp = self.get(&path).await?;

        let results = resp["results"].as_array().cloned().unwrap_or_default();
        let result_count = results.len();

        let documents: Vec<SourceDocument> = results
            .iter()
            .map(|page| parse_page(page, &self.source_name, &self.base_url))
            .collect::<anyhow::Result<Vec<_>>>()?;

        // Determine last_seen from the last document in this page (for backfill).
        // The checkpoint key is the page_id (numeric), not the composite Cosmos id.
        let last_seen = documents.last().and_then(|doc| {
            let page_id = doc.fields.get("page_id")?.as_str()?.to_owned();
            Some(BackfillCheckpoint {
                updated: doc.updated_at,
                key: page_id,
            })
        });

        // Confluence uses offset-based pagination; `_links.next` indicates more pages.
        let has_next = resp["_links"]["next"].is_string();
        let next_start = start + result_count;
        let next_page_token = if result_count > 0 && has_next {
            Some(next_start.to_string())
        } else {
            None
        };

        Ok(FetchPage {
            documents,
            next_page_token,
            last_seen,
        })
    }
}

// ---------------------------------------------------------------------------
// SourceConnector impl
// ---------------------------------------------------------------------------

impl SourceConnector for ConfluenceConnector {
    fn source_type(&self) -> &str {
        "confluence"
    }

    fn source_name(&self) -> &str {
        &self.source_name
    }

    fn subsources(&self) -> &[String] {
        &self.spaces
    }

    fn primary_container(&self) -> &str {
        &self.container
    }

    /// Fetch a closed minute-resolution window of pages modified in `[window_start, window_end]`.
    ///
    /// CQL format per `docs/sync.md`:
    /// ```text
    /// space = "{subsource}" AND type = "page"
    /// AND lastmodified >= "yyyy/MM/dd HH:mm"
    /// AND lastmodified <= "yyyy/MM/dd HH:mm"
    /// ORDER BY lastmodified ASC, id ASC
    /// ```
    async fn fetch_window(
        &self,
        subsource: &str,
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
        batch_size: usize,
        page_token: Option<&str>,
    ) -> anyhow::Result<FetchPage> {
        let start_str = window_start.format("%Y/%m/%d %H:%M").to_string();
        let end_str = window_end.format("%Y/%m/%d %H:%M").to_string();

        let cql = format!(
            r#"space = "{subsource}" AND type = "page" AND lastmodified >= "{start_str}" AND lastmodified <= "{end_str}" ORDER BY lastmodified ASC, id ASC"#
        );

        let start: usize = page_token.and_then(|t| t.parse().ok()).unwrap_or(0);

        self.fetch_pages_page(&cql, start, batch_size).await
    }

    /// Fetch one page of backfill, resuming after `last_seen`.
    ///
    /// CQL format per `docs/sync.md` — "Initial backfill":
    /// ```text
    /// space = "{subsource}" AND type = "page"
    /// AND lastmodified <= "yyyy/MM/dd HH:mm"
    /// [AND ((lastmodified > "{last_seen.updated ISO-8601}")
    ///       OR (lastmodified = "{last_seen.updated ISO-8601}" AND id > "{last_seen.key}"))]
    /// ORDER BY lastmodified ASC, id ASC
    /// ```
    ///
    /// Note: `last_seen.key` is the numeric page id, NOT the composite Cosmos id.
    async fn fetch_backfill_page(
        &self,
        subsource: &str,
        backfill_target: DateTime<Utc>,
        last_seen: Option<&BackfillCheckpoint>,
        batch_size: usize,
    ) -> anyhow::Result<FetchPage> {
        let target_str = backfill_target.format("%Y/%m/%d %H:%M").to_string();

        let cql = if let Some(checkpoint) = last_seen {
            // ISO 8601 with second precision for the resume clause.
            let updated_str = checkpoint
                .updated
                .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                .to_string();
            let key = &checkpoint.key;
            format!(
                r#"space = "{subsource}" AND type = "page" AND lastmodified <= "{target_str}" AND ((lastmodified > "{updated_str}") OR (lastmodified = "{updated_str}" AND id > "{key}")) ORDER BY lastmodified ASC, id ASC"#
            )
        } else {
            format!(
                r#"space = "{subsource}" AND type = "page" AND lastmodified <= "{target_str}" ORDER BY lastmodified ASC, id ASC"#
            )
        };

        self.fetch_pages_page(&cql, 0, batch_size).await
    }

    /// List all page ids for `subsource`, returning them as composite ids.
    ///
    /// Returns `"{source_name}-{space_key}-{page_id}"` for each page.
    async fn list_all_ids(&self, subsource: &str) -> anyhow::Result<Vec<String>> {
        let cql = format!(r#"space = "{subsource}" AND type = "page""#);
        let mut all_ids: Vec<String> = Vec::new();
        let mut start: usize = 0;
        let batch_size: usize = 100;

        loop {
            let path = format!(
                "/rest/api/content/search?cql={}&start={}&limit={}",
                urlencoding::encode(&cql),
                start,
                batch_size,
            );

            let resp = self.get(&path).await?;
            let results = resp["results"].as_array().cloned().unwrap_or_default();
            let count = results.len();

            for page in &results {
                let page_id = page["id"].as_str().unwrap_or("").to_owned();
                // Use space key from the response — canonical per architecture.md.
                let space_key = page["space"]["key"]
                    .as_str()
                    .unwrap_or(subsource)
                    .to_owned();
                all_ids.push(format!("{}-{}-{}", self.source_name, space_key, page_id));
            }

            let has_next = resp["_links"]["next"].is_string();
            start += count;
            if count == 0 || !has_next {
                break;
            }
        }

        Ok(all_ids)
    }

    /// Fetch companion documents (space metadata) for `subsource`.
    ///
    /// GET `/rest/api/space/{subsource}?expand=description,homepage`.
    /// On 404, logs at `debug!` and returns empty `Companions` (does not fail).
    async fn fetch_companions(&self, subsource: &str) -> anyhow::Result<Companions> {
        let path = format!("/rest/api/space/{subsource}?expand=description,homepage");

        let resp = match self.get(&path).await {
            Ok(v) => v,
            Err(e) => {
                debug!(
                    space_key = subsource,
                    error = %e,
                    "Confluence space endpoint unavailable — skipping space companion"
                );
                return Ok(Companions::default());
            }
        };

        let space_doc = parse_space(&resp, &self.source_name, &self.base_url);
        Ok(Companions {
            spaces: vec![space_doc],
            ..Companions::default()
        })
    }
}

// ---------------------------------------------------------------------------
// Date/time helpers
// ---------------------------------------------------------------------------

/// Parse a Confluence timestamp string to `DateTime<Utc>`.
///
/// Confluence returns timestamps in ISO 8601 format, typically:
/// - `"2026-04-28T14:02:11.000Z"` (Cloud)
/// - `"2026-04-28T14:02:11.000+0000"` (Data Center)
fn parse_confluence_datetime(s: &str) -> anyhow::Result<DateTime<Utc>> {
    // 1. Standard RFC3339 / ISO 8601 with colon in offset (or Z suffix)
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    // 2. Confluence's "+0000" format with milliseconds
    if let Ok(dt) = DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.3f%z") {
        return Ok(dt.with_timezone(&Utc));
    }
    // 3. Without milliseconds
    if let Ok(dt) = DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%z") {
        return Ok(dt.with_timezone(&Utc));
    }
    Err(anyhow!("cannot parse Confluence datetime: {s:?}"))
}

// ---------------------------------------------------------------------------
// Document parsers
// ---------------------------------------------------------------------------

/// Map a raw Confluence page JSON value to a [`SourceDocument`].
///
/// Covers every canonical field from `docs/architecture.md` "Confluence page (`confluence_pages`)".
///
/// The `id` is `"{source_name}-{space_key}-{page_id}"` — a three-part composite.
/// `space_key` is read from the page response (`space.key`), not from the connector's
/// subsource parameter, so page-move-between-spaces semantics are handled correctly.
pub fn parse_page(
    page: &Value,
    source_name: &str,
    base_url: &str,
) -> anyhow::Result<SourceDocument> {
    let page_id = page["id"]
        .as_str()
        .ok_or_else(|| anyhow!("Confluence page missing 'id' field"))?;

    // Read space_key from the response — canonical source of truth.
    let space_key = page["space"]["key"].as_str().unwrap_or("").to_owned();

    let title = page["title"].as_str().unwrap_or("").to_owned();

    // updated_at: prefer version.when, fall back to history.lastUpdated.when
    let updated_str = page["version"]["when"]
        .as_str()
        .or_else(|| page["history"]["lastUpdated"]["when"].as_str())
        .ok_or_else(|| anyhow!("Confluence page {page_id} missing version.when"))?;
    let updated_at = parse_confluence_datetime(updated_str)
        .with_context(|| format!("parse updated_at for page {page_id}"))?;

    // source_link: use _links.webui if present, otherwise fallback
    let source_link = if let Some(webui) = page["_links"]["webui"].as_str() {
        format!("{base_url}{webui}")
    } else {
        format!("{base_url}/spaces/{space_key}/pages/{page_id}")
    };

    // 3-part composite id: {source_name}-{space_key}-{page_id}
    // source_name already contains the "confluence-" prefix by convention
    // (e.g. "confluence-internal"), so this produces "confluence-internal-ENG-12345".
    let id = format!("{source_name}-{space_key}-{page_id}");

    // --- Build fields map ---
    let mut map: HashMap<String, Value> = HashMap::new();

    map.insert("space_key".into(), json!(&space_key));
    map.insert("page_id".into(), json!(page_id));
    map.insert("title".into(), json!(&title));
    map.insert("source_name".into(), json!(source_name));
    map.insert("source_link".into(), json!(&source_link));

    // body: from body.storage.value
    map.insert(
        "body".into(),
        json!(page["body"]["storage"]["value"].as_str().unwrap_or("")),
    );

    // version: { number, when, by: {id, name, email} }
    let version_by = parse_confluence_user(&page["version"]["by"]);
    map.insert(
        "version".into(),
        json!({
            "number": page["version"]["number"],
            "when": page["version"]["when"].as_str(),
            "by": version_by
        }),
    );

    // ancestors: array of { id, title }
    let ancestors: Vec<Value> = page["ancestors"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|a| {
            json!({
                "id": a["id"].as_str(),
                "title": a["title"].as_str().unwrap_or("")
            })
        })
        .collect();
    map.insert("ancestors".into(), json!(ancestors));

    // created / created_by: from history.createdDate and history.createdBy
    map.insert(
        "created".into(),
        json!(page["history"]["createdDate"].as_str()),
    );
    map.insert(
        "created_by".into(),
        parse_confluence_user(&page["history"]["createdBy"]),
    );

    // updated / updated_by: updated matches updated_at; by from version.by
    map.insert("updated".into(), json!(updated_str));
    map.insert(
        "updated_by".into(),
        parse_confluence_user(&page["version"]["by"]),
    );

    // labels: from metadata.labels.results[].name
    let labels: Vec<Value> = page["metadata"]["labels"]["results"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|l| json!(l["name"].as_str().unwrap_or("")))
        .collect();
    map.insert("labels".into(), json!(labels));

    // Quelch internals
    map.insert("_partition_key".into(), json!(&space_key));
    map.insert("_deleted".into(), json!(false));
    map.insert("_deleted_at".into(), Value::Null);

    Ok(SourceDocument {
        id,
        partition_key: space_key,
        fields: map,
        updated_at,
        source_link,
    })
}

/// Map a Confluence user object to `{ id, name, email }`.
fn parse_confluence_user(user: &Value) -> Value {
    if user.is_null() || user.is_object() && user.as_object().map(|o| o.is_empty()).unwrap_or(true)
    {
        return Value::Null;
    }
    json!({
        "id": user["accountId"].as_str().or_else(|| user["userKey"].as_str()).or_else(|| user["username"].as_str()),
        "name": user["displayName"].as_str(),
        "email": user["email"].as_str().or_else(|| user["emailAddress"].as_str())
    })
}

/// Map a Confluence space object to a [`SourceDocument`] for the `confluence-spaces` container.
fn parse_space(space: &Value, source_name: &str, base_url: &str) -> SourceDocument {
    let space_key = space["key"].as_str().unwrap_or("").to_owned();
    let id = format!("{source_name}-space-{space_key}");

    // source_link: use _links.webui if available, otherwise construct
    let source_link = if let Some(webui) = space["_links"]["webui"].as_str() {
        format!("{base_url}{webui}")
    } else {
        format!("{base_url}/display/{space_key}/")
    };

    let partition_key = space_key.clone();
    let now = Utc::now().to_rfc3339();
    let updated_at = Utc::now();

    let homepage_id = space["homepage"]["id"].as_str().map(|s| s.to_owned());

    let mut fields: HashMap<String, Value> = HashMap::new();
    fields.insert("id".into(), json!(&id));
    fields.insert("source_name".into(), json!(source_name));
    fields.insert("source_link".into(), json!(&source_link));
    fields.insert("key".into(), json!(&space_key));
    fields.insert("name".into(), json!(space["name"].as_str().unwrap_or("")));
    // description: from description.plain.value or description.view.value
    let description = space["description"]["plain"]["value"]
        .as_str()
        .or_else(|| space["description"]["view"]["value"].as_str())
        .or_else(|| space["description"].as_str());
    fields.insert("description".into(), json!(description));
    fields.insert(
        "type".into(),
        json!(space["type"].as_str().unwrap_or("global")),
    );
    fields.insert("homepage_id".into(), json!(homepage_id));
    fields.insert("created".into(), json!(&now));
    fields.insert("updated".into(), json!(&now));
    fields.insert("_partition_key".into(), json!(&space_key));
    fields.insert("_deleted".into(), json!(false));
    fields.insert("_deleted_at".into(), Value::Null);

    SourceDocument {
        id,
        partition_key,
        fields,
        updated_at,
        source_link,
    }
}

// ---------------------------------------------------------------------------
// URL encoding helper (inline to avoid extra dep)
// ---------------------------------------------------------------------------

mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for byte in s.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(byte as char)
                }
                b' ' => out.push('+'),
                b => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use reqwest_middleware::ClientBuilder;
    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param_contains};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::config::AuthConfig;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    /// Build a [`ConfluenceConnector`] pointing at the mock server URI.
    fn build_connector(
        server_uri: &str,
        source_name: &str,
        auth: AuthConfig,
    ) -> ConfluenceConnector {
        let base_client = reqwest::Client::new();
        let client = ClientBuilder::new(base_client).build();

        let config = ConfluenceSourceConfig {
            name: source_name.to_string(),
            url: server_uri.to_string(),
            auth,
            spaces: vec!["ENG".to_string()],
            container: None,
            companion_containers: Default::default(),
        };

        ConfluenceConnector::new(&config, client).expect("connector construction should not fail")
    }

    /// Minimal valid search response (empty result set).
    fn empty_search_response() -> Value {
        json!({
            "results": [],
            "start": 0,
            "limit": 100,
            "size": 0,
            "_links": {}
        })
    }

    /// A single full-featured Confluence page fixture exercising every canonical field.
    fn full_page_fixture() -> Value {
        json!({
            "id": "12345",
            "type": "page",
            "title": "Camera Connectivity Pipeline",
            "space": { "key": "ENG", "name": "Engineering" },
            "body": {
                "storage": {
                    "value": "<p>Camera connectivity pipeline documentation</p>",
                    "representation": "storage"
                }
            },
            "version": {
                "number": 7,
                "when": "2026-04-28T14:02:11.000Z",
                "by": {
                    "accountId": "user-001",
                    "displayName": "Kristofer Liljeblad",
                    "emailAddress": "kristofer@example.com"
                }
            },
            "ancestors": [
                { "id": "1000", "title": "Architecture" },
                { "id": "1001", "title": "Components" }
            ],
            "metadata": {
                "labels": {
                    "results": [
                        { "name": "camera" },
                        { "name": "architecture" }
                    ]
                }
            },
            "history": {
                "createdDate": "2026-01-12T10:00:00.000Z",
                "createdBy": {
                    "accountId": "user-002",
                    "displayName": "Alice",
                    "emailAddress": "alice@example.com"
                },
                "lastUpdated": {
                    "when": "2026-04-28T14:02:11.000Z"
                }
            },
            "_links": {
                "webui": "/display/ENG/Camera+Connectivity+Pipeline"
            }
        })
    }

    // -----------------------------------------------------------------------
    // Test: fetch_window emits correct CQL
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn fetch_window_emits_correct_cql() {
        let server = MockServer::start().await;

        // The CQL is URL-decoded by wiremock's query_param_contains before matching.
        Mock::given(method("GET"))
            .and(path("/rest/api/content/search"))
            .and(query_param_contains(
                "cql",
                r#"lastmodified >= "2026/04/30 14:23""#,
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(empty_search_response()))
            .mount(&server)
            .await;

        let connector = build_connector(
            &server.uri(),
            "confluence-test",
            AuthConfig::DataCenter { pat: "x".into() },
        );

        let start: DateTime<Utc> = "2026-04-30T14:23:00Z".parse().unwrap();
        let end: DateTime<Utc> = "2026-04-30T14:25:00Z".parse().unwrap();

        let page = connector
            .fetch_window("ENG", start, end, 100, None)
            .await
            .expect("fetch_window should succeed");

        assert!(page.documents.is_empty());
        assert!(page.next_page_token.is_none());
    }

    // -----------------------------------------------------------------------
    // Test: parses full canonical page
    // -----------------------------------------------------------------------

    #[test]
    fn parses_full_canonical_page() {
        let fixture = full_page_fixture();
        let doc = parse_page(
            &fixture,
            "confluence-internal",
            "https://confluence.example.com",
        )
        .expect("parse_page should succeed");

        // 3-part composite id: {source_name}-{space_key}-{page_id}
        // source_name "confluence-internal" already contains the "confluence-" prefix
        assert_eq!(doc.id, "confluence-internal-ENG-12345");
        assert_eq!(doc.partition_key, "ENG");
        assert_eq!(
            doc.source_link,
            "https://confluence.example.com/display/ENG/Camera+Connectivity+Pipeline"
        );

        let f = &doc.fields;

        // Core identity
        assert_eq!(f["space_key"].as_str().unwrap(), "ENG");
        assert_eq!(f["page_id"].as_str().unwrap(), "12345");
        assert_eq!(f["title"].as_str().unwrap(), "Camera Connectivity Pipeline");
        assert_eq!(f["source_name"].as_str().unwrap(), "confluence-internal");

        // Body
        assert!(
            f["body"]
                .as_str()
                .unwrap()
                .contains("Camera connectivity pipeline")
        );

        // Version
        let version = &f["version"];
        assert_eq!(version["number"].as_u64().unwrap(), 7);
        assert!(version["when"].as_str().unwrap().contains("2026-04-28"));
        assert_eq!(
            version["by"]["name"].as_str().unwrap(),
            "Kristofer Liljeblad"
        );

        // Ancestors
        let ancestors = f["ancestors"].as_array().unwrap();
        assert_eq!(ancestors.len(), 2);
        assert_eq!(ancestors[0]["title"].as_str().unwrap(), "Architecture");

        // Created / created_by
        assert!(f["created"].as_str().unwrap().contains("2026-01-12"));
        assert_eq!(f["created_by"]["name"].as_str().unwrap(), "Alice");

        // Updated / updated_by
        assert!(f["updated"].as_str().unwrap().contains("2026-04-28"));
        assert_eq!(
            f["updated_by"]["name"].as_str().unwrap(),
            "Kristofer Liljeblad"
        );

        // Labels
        let labels = f["labels"].as_array().unwrap();
        assert_eq!(labels.len(), 2);
        assert!(labels.iter().any(|l| l.as_str() == Some("camera")));
        assert!(labels.iter().any(|l| l.as_str() == Some("architecture")));

        // Quelch internals
        assert_eq!(f["_partition_key"].as_str().unwrap(), "ENG");
        assert!(!f["_deleted"].as_bool().unwrap());
        assert!(f["_deleted_at"].is_null());
    }

    // -----------------------------------------------------------------------
    // Test: page id includes space_key (3-part composite)
    // -----------------------------------------------------------------------

    #[test]
    fn id_includes_space_key() {
        let fixture = full_page_fixture();
        let doc = parse_page(&fixture, "my-source", "https://confluence.example.com")
            .expect("parse_page should succeed");

        // Must be: {source_name}-{space_key}-{page_id}
        // source_name "my-source" is used as-is; by convention Confluence source names
        // start with "confluence-" but the id format itself is just {source_name}-{space_key}-{page_id}.
        assert_eq!(doc.id, "my-source-ENG-12345");

        // Must NOT be a 2-part id without space key (old format was source_name-page_id)
        assert_ne!(doc.id, "my-source-12345");
        assert!(doc.id.contains("ENG"), "id must include space_key");
        assert!(doc.id.contains("12345"), "id must include page_id");
    }

    // -----------------------------------------------------------------------
    // Test: paginates via start offset
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn paginates_via_start_offset() {
        let server = MockServer::start().await;

        let page1_results: Vec<Value> = (0..5)
            .map(|i| {
                json!({
                    "id": format!("{i}"),
                    "type": "page",
                    "title": format!("Page {i}"),
                    "space": { "key": "ENG" },
                    "body": { "storage": { "value": "content" } },
                    "version": {
                        "number": 1,
                        "when": "2026-04-01T00:00:00.000Z",
                        "by": { "accountId": "u1", "displayName": "User" }
                    },
                    "ancestors": [],
                    "metadata": { "labels": { "results": [] } },
                    "history": { "createdDate": "2026-01-01T00:00:00.000Z", "createdBy": {} },
                    "_links": { "webui": format!("/display/ENG/Page+{i}") }
                })
            })
            .collect();

        // First page response: has _links.next to indicate more
        Mock::given(method("GET"))
            .and(path("/rest/api/content/search"))
            .and(query_param_contains("start", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "results": page1_results,
                "start": 0,
                "limit": 5,
                "size": 5,
                "_links": {
                    "next": "/rest/api/content/search?start=5&limit=5"
                }
            })))
            .mount(&server)
            .await;

        // Second page: empty, no next link
        Mock::given(method("GET"))
            .and(path("/rest/api/content/search"))
            .and(query_param_contains("start", "5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "results": [],
                "start": 5,
                "limit": 5,
                "size": 0,
                "_links": {}
            })))
            .mount(&server)
            .await;

        let connector = build_connector(
            &server.uri(),
            "test",
            AuthConfig::DataCenter { pat: "x".into() },
        );

        let start: DateTime<Utc> = "2026-04-01T00:00:00Z".parse().unwrap();
        let end: DateTime<Utc> = "2026-04-02T00:00:00Z".parse().unwrap();

        let page1 = connector
            .fetch_window("ENG", start, end, 5, None)
            .await
            .expect("first page should succeed");

        assert_eq!(page1.documents.len(), 5);
        // next_page_token should be "5" (start offset of next page)
        assert_eq!(page1.next_page_token, Some("5".to_string()));

        let page2 = connector
            .fetch_window("ENG", start, end, 5, page1.next_page_token.as_deref())
            .await
            .expect("second page should succeed");

        assert_eq!(page2.documents.len(), 0);
        assert!(page2.next_page_token.is_none());
    }

    // -----------------------------------------------------------------------
    // Test: fetch_backfill_page with resume clause
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn fetch_backfill_page_with_resume_clause() {
        let server = MockServer::start().await;

        // We verify the OR-clause is present in the decoded CQL value.
        // wiremock's query_param_contains matches against the URL-decoded parameter value.
        Mock::given(method("GET"))
            .and(path("/rest/api/content/search"))
            .and(query_param_contains("cql", "lastmodified > "))
            .and(query_param_contains("cql", r#"id > "12345""#))
            .respond_with(ResponseTemplate::new(200).set_body_json(empty_search_response()))
            .mount(&server)
            .await;

        let connector = build_connector(
            &server.uri(),
            "test",
            AuthConfig::DataCenter { pat: "x".into() },
        );

        let target: DateTime<Utc> = "2026-04-30T14:25:00Z".parse().unwrap();
        // last_seen.key is the numeric page_id, NOT the composite id
        let last_seen = BackfillCheckpoint {
            updated: "2026-04-28T10:00:00Z".parse().unwrap(),
            key: "12345".to_string(), // numeric page id
        };

        let page = connector
            .fetch_backfill_page("ENG", target, Some(&last_seen), 100)
            .await
            .expect("backfill with resume clause should succeed");

        assert!(page.documents.is_empty());
    }

    // -----------------------------------------------------------------------
    // Test: last_seen.key is the page_id (numeric), not the composite id
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn backfill_last_seen_key_is_page_id() {
        let server = MockServer::start().await;

        let page_fixture = json!({
            "id": "99999",
            "type": "page",
            "title": "Test Page",
            "space": { "key": "ENG" },
            "body": { "storage": { "value": "content" } },
            "version": {
                "number": 1,
                "when": "2026-04-28T14:02:11.000Z",
                "by": { "accountId": "u1", "displayName": "User" }
            },
            "ancestors": [],
            "metadata": { "labels": { "results": [] } },
            "history": { "createdDate": "2026-01-01T00:00:00.000Z", "createdBy": {} },
            "_links": { "webui": "/display/ENG/Test+Page" }
        });

        Mock::given(method("GET"))
            .and(path("/rest/api/content/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "results": [page_fixture],
                "start": 0,
                "limit": 100,
                "size": 1,
                "_links": {}
            })))
            .mount(&server)
            .await;

        let connector = build_connector(
            &server.uri(),
            "test",
            AuthConfig::DataCenter { pat: "x".into() },
        );

        let target: DateTime<Utc> = "2026-04-30T14:25:00Z".parse().unwrap();
        let fetch_page = connector
            .fetch_backfill_page("ENG", target, None, 100)
            .await
            .expect("backfill page should succeed");

        let last_seen = fetch_page.last_seen.expect("last_seen should be populated");

        // Key must be the page_id "99999", NOT the composite "confluence-test-ENG-99999"
        assert_eq!(last_seen.key, "99999");
        assert!(
            !last_seen.key.starts_with("confluence-"),
            "last_seen.key must be the numeric page_id, not the composite id"
        );
    }

    // -----------------------------------------------------------------------
    // Test: list_all_ids returns composite ids
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_all_ids_returns_composite_ids() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/rest/api/content/search"))
            .and(query_param_contains("start", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "results": [
                    { "id": "111", "space": { "key": "ENG" } },
                    { "id": "222", "space": { "key": "ENG" } },
                    { "id": "333", "space": { "key": "ENG" } }
                ],
                "start": 0,
                "limit": 100,
                "size": 3,
                "_links": {}
            })))
            .mount(&server)
            .await;

        let connector = build_connector(
            &server.uri(),
            "confluence-prod",
            AuthConfig::DataCenter { pat: "x".into() },
        );

        let ids = connector
            .list_all_ids("ENG")
            .await
            .expect("list_all_ids should succeed");

        assert_eq!(ids.len(), 3);
        // Each id must be {source_name}-{space_key}-{page_id}
        // source_name = "confluence-prod", space_key = "ENG"
        assert!(ids.contains(&"confluence-prod-ENG-111".to_string()));
        assert!(ids.contains(&"confluence-prod-ENG-222".to_string()));
        assert!(ids.contains(&"confluence-prod-ENG-333".to_string()));

        // Must include the space_key — NOT just source_name-page_id
        for id in &ids {
            assert!(
                id.starts_with("confluence-prod-ENG-"),
                "id {id} does not follow expected composite format (source_name-space_key-page_id)"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Test: fetch_companions handles 404 gracefully
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn fetch_companions_handles_404() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/rest/api/space/ENG"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let connector = build_connector(
            &server.uri(),
            "confluence-internal",
            AuthConfig::DataCenter { pat: "x".into() },
        );

        let companions = connector
            .fetch_companions("ENG")
            .await
            .expect("fetch_companions should not error on 404");

        assert!(
            companions.spaces.is_empty(),
            "spaces should be empty when space endpoint 404s"
        );
        assert!(companions.sprints.is_empty());
        assert!(companions.fix_versions.is_empty());
        assert!(companions.projects.is_empty());
    }

    // -----------------------------------------------------------------------
    // Test: fetch_companions populates space doc correctly
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn fetch_companions_populates_space() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/rest/api/space/ENG"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "key": "ENG",
                "name": "Engineering",
                "type": "global",
                "description": {
                    "plain": {
                        "value": "Engineering team space",
                        "representation": "plain"
                    }
                },
                "homepage": { "id": "10001" },
                "_links": { "webui": "/display/ENG/" }
            })))
            .mount(&server)
            .await;

        let connector = build_connector(
            &server.uri(),
            "confluence-internal",
            AuthConfig::DataCenter { pat: "x".into() },
        );

        let companions = connector
            .fetch_companions("ENG")
            .await
            .expect("fetch_companions should succeed");

        assert_eq!(companions.spaces.len(), 1);
        let space = &companions.spaces[0];
        assert_eq!(space.id, "confluence-internal-space-ENG");
        assert_eq!(space.partition_key, "ENG");
        assert_eq!(space.fields["key"].as_str().unwrap(), "ENG");
        assert_eq!(space.fields["name"].as_str().unwrap(), "Engineering");
        assert_eq!(space.fields["type"].as_str().unwrap(), "global");
        assert_eq!(
            space.fields["description"].as_str().unwrap(),
            "Engineering team space"
        );
        assert_eq!(space.fields["homepage_id"].as_str().unwrap(), "10001");
    }

    // -----------------------------------------------------------------------
    // Test: auth header is sent on every request
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn auth_header_sent_on_every_request() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/rest/api/content/search"))
            .and(header("Authorization", "Bearer my-pat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(empty_search_response()))
            .mount(&server)
            .await;

        let connector = build_connector(
            &server.uri(),
            "test",
            AuthConfig::DataCenter {
                pat: "my-pat".into(),
            },
        );

        let start: DateTime<Utc> = "2026-04-30T14:23:00Z".parse().unwrap();
        let end: DateTime<Utc> = "2026-04-30T14:25:00Z".parse().unwrap();

        connector
            .fetch_window("ENG", start, end, 100, None)
            .await
            .expect("request with correct auth header should succeed");
    }

    // -----------------------------------------------------------------------
    // Test: cloud Basic auth header
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn cloud_basic_auth_header_is_set() {
        use base64::Engine;
        let credentials = "user@example.com:my-api-token";
        let expected = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(credentials)
        );

        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/rest/api/content/search"))
            .and(header("Authorization", expected.as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(empty_search_response()))
            .mount(&server)
            .await;

        let connector = build_connector(
            &server.uri(),
            "test",
            AuthConfig::Cloud {
                email: "user@example.com".into(),
                api_token: "my-api-token".into(),
            },
        );

        let start: DateTime<Utc> = "2026-04-30T14:23:00Z".parse().unwrap();
        let end: DateTime<Utc> = "2026-04-30T14:25:00Z".parse().unwrap();

        connector
            .fetch_window("ENG", start, end, 100, None)
            .await
            .expect("cloud auth header should be sent correctly");
    }
}
