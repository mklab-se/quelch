//! `SearchApiAdapter` trait and production `AzureSearchAdapter`.
//!
//! The trait decouples the `search::run` business logic from the Azure
//! REST transport, enabling unit tests to use a mock without any network
//! calls.
//!
//! # Production adapter
//!
//! [`AzureSearchAdapter`] makes raw HTTP calls to:
//!
//! - `POST {service}/indexes/{indexName}/docs/search?api-version=...`
//!   for direct hybrid search (`search_index`).
//! - `POST {service}/knowledgebases/{kbName}/retrieve?api-version=...`
//!   for Knowledge Base agentic retrieval (`search_knowledge_base`).
//!
//! Auth is obtained via `rigg-client`'s [`rigg_client::auth::get_auth_provider`].
//!
//! TODO(phase-11): These methods could migrate into `rigg_client::AzureSearchClient`
//! for consistency. For now they live here to keep the transport self-contained.

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Value, json};

use crate::mcp::error::McpError;

// ---------------------------------------------------------------------------
// Raw response types
// ---------------------------------------------------------------------------

/// A single search hit, as returned by the underlying adapter.
#[derive(Debug)]
pub struct RawHit {
    /// Document ID.
    pub id: String,
    /// Relevance / ranking score.
    pub score: f64,
    /// All non-system fields from the Azure index document.
    pub fields: Value,
    /// Highlight / snippet (populated when include_full_body = false).
    pub snippet: Option<String>,
    /// Full document body (populated when include_full_body = true).
    pub body: Option<String>,
}

/// The raw response returned by both search paths.
#[derive(Debug)]
pub struct RawSearchResponse {
    pub hits: Vec<RawHit>,
    /// Synthesised answer from KB agentic retrieval (only for KB path with
    /// `include_synthesis = true`).
    pub answer: Option<String>,
    /// Source citations for the synthesised answer.
    pub citations: Option<Vec<Value>>,
    /// Pagination continuation token (base64-encoded for single-index; per-index
    /// map for multi-index fan-out).
    pub next_cursor: Option<String>,
    /// Estimated total matches across all indexes.
    pub total_estimate: u64,
}

// ---------------------------------------------------------------------------
// Adapter trait
// ---------------------------------------------------------------------------

/// Abstraction over the Azure AI Search REST API for the `search` tool.
///
/// The production implementation wraps the Azure REST API directly via
/// [`AzureSearchAdapter`].  Tests inject [`MockSearchApi`].
#[async_trait]
#[allow(clippy::too_many_arguments)]
pub trait SearchApiAdapter: Send + Sync {
    /// Run a Knowledge Base agentic-retrieval query.
    ///
    /// `include_synthesis: true` requests the synthesised `answer` + citations
    /// in the response (corresponds to MCP `include_content: "agentic_answer"`).
    /// `include_full_body: true` requests full document bodies in hits.
    async fn search_knowledge_base(
        &self,
        knowledge_base_name: &str,
        query: &str,
        odata_filter: Option<&str>,
        top: usize,
        cursor: Option<&str>,
        include_synthesis: bool,
        include_full_body: bool,
    ) -> Result<RawSearchResponse, McpError>;

    /// Direct hybrid search against a single Azure AI Search index.
    ///
    /// Used as the fallback when `disable_agentic` is set, or when
    /// `include_content` is `snippet` / `full`.
    async fn search_index(
        &self,
        index_name: &str,
        query: &str,
        odata_filter: Option<&str>,
        top: usize,
        cursor: Option<&str>,
        include_full_body: bool,
    ) -> Result<RawSearchResponse, McpError>;
}

// ---------------------------------------------------------------------------
// Production adapter (raw reqwest calls)
// ---------------------------------------------------------------------------

/// Production adapter that talks directly to the Azure AI Search REST API.
///
/// Auth is obtained via [`rigg_client::auth::get_auth_provider`] (Azure CLI
/// or service-principal env vars).
pub struct AzureSearchAdapter {
    http: Client,
    service_url: String,
    api_version: String,
    token: String,
}

impl AzureSearchAdapter {
    /// Create an adapter for the given Azure AI Search service URL.
    ///
    /// The `service_url` is typically `https://{name}.search.windows.net`.
    /// The `api_version` should be the preview version used by rigg-client
    /// (e.g. `"2025-11-01-preview"`).
    pub fn new(service_url: String, api_version: String) -> Result<Self, McpError> {
        let auth = rigg_client::auth::get_auth_provider()
            .map_err(|e| McpError::Unauthenticated(format!("auth: {e}")))?;
        let token = auth
            .get_token()
            .map_err(|e| McpError::Unauthenticated(format!("token: {e}")))?;
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| McpError::Internal(format!("http client: {e}")))?;

        Ok(Self {
            http,
            service_url,
            api_version,
            token,
        })
    }

    /// Bearer-token header value.
    fn bearer(&self) -> String {
        format!("Bearer {}", self.token)
    }
}

#[async_trait]
impl SearchApiAdapter for AzureSearchAdapter {
    async fn search_knowledge_base(
        &self,
        knowledge_base_name: &str,
        query: &str,
        odata_filter: Option<&str>,
        top: usize,
        cursor: Option<&str>,
        include_synthesis: bool,
        include_full_body: bool,
    ) -> Result<RawSearchResponse, McpError> {
        // POST {service}/knowledgebases/{name}/retrieve?api-version=...
        // This is the Azure AI Search Agentic Retrieval (Knowledge Base search) endpoint.
        // Ref: https://learn.microsoft.com/azure/search/knowledge-store-concept-intro (preview)
        // TODO(phase-11): verify exact endpoint path and body schema against GA docs.
        let url = format!(
            "{}/knowledgebases/{}/retrieve?api-version={}",
            self.service_url, knowledge_base_name, self.api_version,
        );

        let mut body = json!({
            "search": query,
            "top": top,
            "includeSynthesis": include_synthesis,
            "includeFullBody": include_full_body,
        });

        if let Some(filter) = odata_filter {
            body["filter"] = json!(filter);
        }
        if let Some(c) = cursor {
            body["continuationToken"] = json!(c);
        }

        let resp = self
            .http
            .post(&url)
            .header("Authorization", self.bearer())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| McpError::Unavailable(format!("KB search request failed: {e}")))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| McpError::Internal(format!("read KB response: {e}")))?;

        if !status.is_success() {
            return Err(McpError::Unavailable(format!(
                "KB search returned {status}: {text}"
            )));
        }

        let val: Value = serde_json::from_str(&text)
            .map_err(|e| McpError::Internal(format!("parse KB response: {e}")))?;

        parse_kb_response(val)
    }

    async fn search_index(
        &self,
        index_name: &str,
        query: &str,
        odata_filter: Option<&str>,
        top: usize,
        cursor: Option<&str>,
        include_full_body: bool,
    ) -> Result<RawSearchResponse, McpError> {
        // POST {service}/indexes/{name}/docs/search?api-version=...
        // Ref: https://learn.microsoft.com/azure/search/search-query-rest-api
        let url = format!(
            "{}/indexes/{}/docs/search?api-version={}",
            self.service_url, index_name, self.api_version,
        );

        let mut body = json!({
            "search": query,
            "top": top,
            "select": "*",
            "count": true,
            "queryType": "full",
            "searchMode": "any",
            "vectorQueries": [],
        });

        if let Some(filter) = odata_filter {
            body["filter"] = json!(filter);
        }
        if let Some(c) = cursor {
            // Azure passes the continuation in the search body as @search.nextPageParameters
            // but the token itself is provided as a POST body replacement.
            // For simplicity, treat it as an opaque continuation token embedded in query.
            body["continuationToken"] = json!(c);
        }
        if !include_full_body {
            body["highlight"] = json!("body");
            body["highlightPreTag"] = json!("<mark>");
            body["highlightPostTag"] = json!("</mark>");
        }

        let resp = self
            .http
            .post(&url)
            .header("Authorization", self.bearer())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| McpError::Unavailable(format!("index search request failed: {e}")))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| McpError::Internal(format!("read index response: {e}")))?;

        if !status.is_success() {
            return Err(McpError::Unavailable(format!(
                "index search returned {status}: {text}"
            )));
        }

        let val: Value = serde_json::from_str(&text)
            .map_err(|e| McpError::Internal(format!("parse index response: {e}")))?;

        parse_index_response(val, include_full_body)
    }
}

// ---------------------------------------------------------------------------
// Response parsers
// ---------------------------------------------------------------------------

/// Parse a Knowledge Base `/retrieve` response into [`RawSearchResponse`].
fn parse_kb_response(val: Value) -> Result<RawSearchResponse, McpError> {
    // Expected shape (preview, approximate):
    // {
    //   "value": [ { "@search.score": 0.9, "id": "...", ... } ],
    //   "synthesis": { "answer": "...", "citations": [...] },
    //   "@search.nextPageParameters": { "continuationToken": "..." }
    // }
    let hits = parse_hits(val.get("value"), true)?;
    let answer = val
        .pointer("/synthesis/answer")
        .and_then(Value::as_str)
        .map(String::from);
    let citations = val
        .pointer("/synthesis/citations")
        .and_then(Value::as_array)
        .cloned();
    let next_cursor = extract_continuation(&val);
    let total_estimate = hits.len() as u64;

    Ok(RawSearchResponse {
        hits,
        answer,
        citations,
        next_cursor,
        total_estimate,
    })
}

/// Parse an index `/docs/search` response into [`RawSearchResponse`].
fn parse_index_response(
    val: Value,
    include_full_body: bool,
) -> Result<RawSearchResponse, McpError> {
    // Azure index search response:
    // {
    //   "@odata.count": 42,
    //   "value": [ { "@search.score": 0.9, "@search.highlights": {...}, "id": "...", ... } ],
    //   "@search.nextPageParameters": { ... }
    // }
    let total_estimate = val.get("@odata.count").and_then(Value::as_u64).unwrap_or(0);
    let hits = parse_hits(val.get("value"), include_full_body)?;
    let next_cursor = extract_continuation(&val);

    Ok(RawSearchResponse {
        hits,
        answer: None,
        citations: None,
        next_cursor,
        total_estimate,
    })
}

/// Parse the `"value"` array into [`RawHit`]s.
fn parse_hits(value: Option<&Value>, include_full_body: bool) -> Result<Vec<RawHit>, McpError> {
    let arr = match value.and_then(Value::as_array) {
        Some(a) => a,
        None => return Ok(vec![]),
    };

    let mut hits = Vec::with_capacity(arr.len());
    for item in arr {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        let score = item
            .get("@search.score")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);

        // Extract snippet from highlights.
        let snippet = item
            .pointer("/@search.highlights/body/0")
            .and_then(Value::as_str)
            .map(String::from);

        // Full body field from document.
        let body = if include_full_body {
            item.get("body").and_then(Value::as_str).map(String::from)
        } else {
            None
        };

        // Build the `fields` object: everything except Azure system fields.
        let fields = strip_system_fields(item);

        hits.push(RawHit {
            id,
            score,
            fields,
            snippet,
            body,
        });
    }

    Ok(hits)
}

/// Extract the continuation token from the response, if any.
fn extract_continuation(val: &Value) -> Option<String> {
    // Index search: token is in @search.nextPageParameters.
    if let Some(npp) = val.get("@search.nextPageParameters")
        && let Ok(encoded) = serde_json::to_string(npp)
    {
        use base64::Engine;
        return Some(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(encoded));
    }
    // KB retrieve: may use continuationToken directly.
    val.get("continuationToken")
        .and_then(Value::as_str)
        .map(String::from)
}

/// Remove Azure system fields (prefixed with `@search.`) from a hit object.
fn strip_system_fields(item: &Value) -> Value {
    match item.as_object() {
        None => item.clone(),
        Some(map) => {
            let filtered: serde_json::Map<String, Value> = map
                .iter()
                .filter(|(k, _)| !k.starts_with('@'))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            Value::Object(filtered)
        }
    }
}

// ---------------------------------------------------------------------------
// No-op adapter (for `quelch dev` and tests that don't care about search)
// ---------------------------------------------------------------------------

/// A [`SearchApiAdapter`] that always returns empty results.
///
/// Used by [`crate::mcp::run_server_in_memory`] so `quelch dev` can serve
/// MCP tool calls without any Azure credentials.  The `query`, `get`, and
/// `aggregate` tools still work normally via the Cosmos backend; only the
/// `search` tool will return an empty result set.
pub struct NoOpSearch;

#[async_trait]
impl SearchApiAdapter for NoOpSearch {
    async fn search_knowledge_base(
        &self,
        _knowledge_base_name: &str,
        _query: &str,
        _odata_filter: Option<&str>,
        _top: usize,
        _cursor: Option<&str>,
        _include_synthesis: bool,
        _include_full_body: bool,
    ) -> Result<RawSearchResponse, McpError> {
        Ok(RawSearchResponse {
            hits: vec![],
            answer: None,
            citations: None,
            next_cursor: None,
            total_estimate: 0,
        })
    }

    async fn search_index(
        &self,
        _index_name: &str,
        _query: &str,
        _odata_filter: Option<&str>,
        _top: usize,
        _cursor: Option<&str>,
        _include_full_body: bool,
    ) -> Result<RawSearchResponse, McpError> {
        Ok(RawSearchResponse {
            hits: vec![],
            answer: None,
            citations: None,
            next_cursor: None,
            total_estimate: 0,
        })
    }
}

// ---------------------------------------------------------------------------
// Mock (for unit tests)
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Call record for inspecting which adapter method was invoked.
    #[derive(Debug, Clone)]
    pub enum SearchCall {
        KnowledgeBase {
            knowledge_base_name: String,
            query: String,
            odata_filter: Option<String>,
            top: usize,
            cursor: Option<String>,
            include_synthesis: bool,
            include_full_body: bool,
        },
        Index {
            index_name: String,
            query: String,
            odata_filter: Option<String>,
            top: usize,
            cursor: Option<String>,
            include_full_body: bool,
        },
    }

    /// A mock [`SearchApiAdapter`] that records calls and returns canned responses.
    #[derive(Default, Clone)]
    pub struct MockSearchApi {
        /// Recorded calls (push-only from inside the adapter).
        pub calls: Arc<Mutex<Vec<SearchCall>>>,
        /// If set, KB searches return this response; otherwise a default.
        pub kb_response: Option<Arc<dyn Fn() -> RawSearchResponse + Send + Sync>>,
        /// If set, index searches return this response; otherwise a default.
        pub index_response: Option<Arc<dyn Fn() -> RawSearchResponse + Send + Sync>>,
    }

    impl MockSearchApi {
        pub fn new() -> Self {
            Self::default()
        }

        /// Configure the KB response factory.
        pub fn with_kb_response<F>(mut self, f: F) -> Self
        where
            F: Fn() -> RawSearchResponse + Send + Sync + 'static,
        {
            self.kb_response = Some(Arc::new(f));
            self
        }

        /// Configure the index response factory.
        pub fn with_index_response<F>(mut self, f: F) -> Self
        where
            F: Fn() -> RawSearchResponse + Send + Sync + 'static,
        {
            self.index_response = Some(Arc::new(f));
            self
        }

        /// Return a reference to the recorded calls.
        pub fn calls_snapshot(&self) -> Vec<SearchCall> {
            self.calls.lock().unwrap().clone()
        }

        /// Build a minimal canned `RawSearchResponse` with one hit.
        pub fn default_response() -> RawSearchResponse {
            RawSearchResponse {
                hits: vec![RawHit {
                    id: "hit-1".to_string(),
                    score: 0.9,
                    fields: serde_json::json!({
                        "source_link": "https://example.com/issue/1",
                        "summary": "Test hit",
                    }),
                    snippet: Some("…relevant snippet…".to_string()),
                    body: None,
                }],
                answer: None,
                citations: None,
                next_cursor: None,
                total_estimate: 1,
            }
        }

        /// Build a canned response with an agentic answer populated.
        pub fn agentic_response() -> RawSearchResponse {
            RawSearchResponse {
                hits: vec![RawHit {
                    id: "hit-a".to_string(),
                    score: 0.95,
                    fields: serde_json::json!({
                        "source_link": "https://example.com/page/1",
                        "title": "Relevant Page",
                    }),
                    snippet: Some("…agentic snippet…".to_string()),
                    body: None,
                }],
                answer: Some("The synthesised answer is: 42.".to_string()),
                citations: Some(vec![
                    serde_json::json!({"url": "https://example.com/page/1"}),
                ]),
                next_cursor: None,
                total_estimate: 1,
            }
        }

        /// Build a canned response with body content.
        pub fn full_body_response() -> RawSearchResponse {
            RawSearchResponse {
                hits: vec![RawHit {
                    id: "hit-b".to_string(),
                    score: 0.85,
                    fields: serde_json::json!({
                        "source_link": "https://example.com/issue/2",
                        "summary": "Full body test",
                    }),
                    snippet: Some("…snippet…".to_string()),
                    body: Some("Full body content here.".to_string()),
                }],
                answer: None,
                citations: None,
                next_cursor: None,
                total_estimate: 1,
            }
        }
    }

    #[async_trait]
    impl SearchApiAdapter for MockSearchApi {
        async fn search_knowledge_base(
            &self,
            knowledge_base_name: &str,
            query: &str,
            odata_filter: Option<&str>,
            top: usize,
            cursor: Option<&str>,
            include_synthesis: bool,
            include_full_body: bool,
        ) -> Result<RawSearchResponse, McpError> {
            self.calls.lock().unwrap().push(SearchCall::KnowledgeBase {
                knowledge_base_name: knowledge_base_name.to_string(),
                query: query.to_string(),
                odata_filter: odata_filter.map(String::from),
                top,
                cursor: cursor.map(String::from),
                include_synthesis,
                include_full_body,
            });

            Ok(if let Some(f) = &self.kb_response {
                f()
            } else {
                MockSearchApi::default_response()
            })
        }

        async fn search_index(
            &self,
            index_name: &str,
            query: &str,
            odata_filter: Option<&str>,
            top: usize,
            cursor: Option<&str>,
            include_full_body: bool,
        ) -> Result<RawSearchResponse, McpError> {
            self.calls.lock().unwrap().push(SearchCall::Index {
                index_name: index_name.to_string(),
                query: query.to_string(),
                odata_filter: odata_filter.map(String::from),
                top,
                cursor: cursor.map(String::from),
                include_full_body,
            });

            Ok(if let Some(f) = &self.index_response {
                f()
            } else {
                MockSearchApi::default_response()
            })
        }
    }
}
