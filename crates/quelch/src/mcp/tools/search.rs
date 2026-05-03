//! MCP `search` tool — semantic / hybrid search via Azure AI Search.
//!
//! # Routing
//!
//! By default, queries route through the Azure AI Search **Knowledge Base**
//! (Agentic Retrieval), which groups all exposed indexes and produces
//! synthesised answers.  When `config.disable_agentic` is true, or when
//! `include_content` is `snippet` or `full`, the tool falls back to direct
//! per-index hybrid search.
//!
//! # `include_content`
//!
//! | Value            | Routing         | `snippet` | `body` | `answer` + `citations` |
//! |-----------------|----------------|-----------|--------|------------------------|
//! | `snippet`       | direct index   | yes       | no     | no                     |
//! | `full`          | direct index   | yes       | yes    | no                     |
//! | `agentic_answer`| Knowledge Base | yes       | no     | yes                    |
//!
//! Requesting `agentic_answer` when `disable_agentic: true` returns
//! `McpError::InvalidArgument`.
//!
//! # Soft-delete
//!
//! The tool always appends `_deleted ne true` to the OData filter unless
//! `include_deleted: true` is passed.
//!
//! # Multi-source fan-out
//!
//! When `data_sources` is `None`, the tool queries all searchable exposed
//! sources.  For the direct-index path, each index is queried sequentially
//! and results are merged/sorted by score.  For the Knowledge Base path,
//! a single KB call is made (the KB handles fan-out server-side).
//!
//! TODO(v2-follow-up): parallel fan-out with merged cursor for multi-index.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::mcp::error::McpError;
use crate::mcp::expose::ExposeResolver;
use crate::mcp::filter::{odata, parse};

use super::search_api::{RawHit, SearchApiAdapter};

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

/// How much content to return in search results.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IncludeContent {
    /// Return a snippet / highlight extracted by Azure (default).
    #[default]
    Snippet,
    /// Return the full document body.
    Full,
    /// Route through the Knowledge Base and return a synthesised answer with
    /// citations in addition to the document hits.
    AgenticAnswer,
}

/// Request parameters for the `search` tool.
#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    /// Free-text search query.
    pub query: String,
    /// Logical data-source names to search.  When absent, all exposed
    /// **searchable** data sources are searched.
    pub data_sources: Option<Vec<String>>,
    /// Optional structured filter (JSON filter grammar).
    #[serde(rename = "where")]
    pub r#where: Option<Value>,
    /// Maximum hits per page (default: 25).
    #[serde(default = "default_top")]
    pub top: usize,
    /// Pagination cursor from a prior response.
    pub cursor: Option<String>,
    /// When `true`, include soft-deleted documents.
    #[serde(default)]
    pub include_deleted: bool,
    /// Controls what content is returned and which backend is used.
    #[serde(default)]
    pub include_content: IncludeContent,
}

fn default_top() -> usize {
    25
}

/// Configuration for the `search` tool runtime.
#[derive(Debug, Clone)]
pub struct SearchToolConfig {
    /// When `true`, only direct hybrid search is used (no Knowledge Base).
    pub disable_agentic: bool,
    /// Name of the Azure AI Search Knowledge Base to query.
    pub knowledge_base_name: String,
    /// Default page size (mirrors `mcp.default_top`).
    pub default_top: usize,
    /// Maximum page size allowed (mirrors `mcp.max_top`).
    pub max_top: usize,
}

impl Default for SearchToolConfig {
    fn default() -> Self {
        Self {
            disable_agentic: false,
            knowledge_base_name: "quelch-kb".to_string(),
            default_top: 25,
            max_top: 100,
        }
    }
}

/// Response from the `search` tool.
#[derive(Debug, Serialize)]
pub struct SearchResponse {
    /// Matching documents.
    pub items: Vec<SearchItem>,
    /// Synthesised answer (only when `include_content: agentic_answer`).
    pub answer: Option<String>,
    /// Source citations for the answer (only when `include_content: agentic_answer`).
    pub citations: Option<Vec<Value>>,
    /// Pagination cursor for the next page, or `None` when exhausted.
    pub next_cursor: Option<String>,
    /// Estimated total number of matching documents across all queried sources.
    pub total_estimate: u64,
}

/// A single search hit in the response.
#[derive(Debug, Serialize)]
pub struct SearchItem {
    /// Document ID.
    pub id: String,
    /// Relevance score (higher is better).
    pub score: f64,
    /// Logical data-source name this hit came from.
    pub data_source: String,
    /// URL to the source document (e.g. Jira issue, Confluence page).
    pub source_link: String,
    /// Snippet / highlight (present when `include_content: snippet` or `full`).
    pub snippet: Option<String>,
    /// Full document body (present only when `include_content: full`).
    pub body: Option<String>,
    /// All remaining document fields.
    pub fields: Value,
}

// ---------------------------------------------------------------------------
// Tool entry point
// ---------------------------------------------------------------------------

/// Execute the `search` tool.
///
/// # Arguments
///
/// * `api` — the search API adapter (production or mock).
/// * `expose` — the exposure resolver enforcing the deployment's `expose:` list.
/// * `schema` — the schema catalog (used to filter searchable sources).
/// * `config` — runtime config for this tool.
/// * `req` — the request parameters.
pub async fn run(
    api: &dyn SearchApiAdapter,
    expose: &ExposeResolver,
    schema: &crate::mcp::schema::SchemaCatalog,
    config: &SearchToolConfig,
    req: SearchRequest,
) -> Result<SearchResponse, McpError> {
    // ── 1. Validate incompatible flags ──────────────────────────────────────
    if config.disable_agentic && req.include_content == IncludeContent::AgenticAnswer {
        return Err(McpError::InvalidArgument(
            "agentic_answer is unavailable when disable_agentic is set".to_string(),
        ));
    }

    // ── 2. Clamp top ────────────────────────────────────────────────────────
    let top = req.top.min(config.max_top);

    // ── 3. Resolve data sources ─────────────────────────────────────────────
    let sources: Vec<(String, String)> = match &req.data_sources {
        Some(names) => {
            // Explicit list — check each is exposed.
            let mut resolved = Vec::new();
            for name in names {
                let ds = expose.resolve(name)?;
                resolved.push((name.clone(), ds.kind.clone()));
            }
            resolved
        }
        None => {
            // All exposed, searchable sources.
            expose
                .list_all()
                .iter()
                .filter(|(_, ds)| {
                    schema
                        .lookup(&ds.kind)
                        .map(|k| k.searchable)
                        .unwrap_or(false)
                })
                .map(|(name, ds)| (name.clone(), ds.kind.clone()))
                .collect()
        }
    };

    if sources.is_empty() {
        return Ok(SearchResponse {
            items: vec![],
            answer: None,
            citations: None,
            next_cursor: None,
            total_estimate: 0,
        });
    }

    // ── 4. Build OData filter ────────────────────────────────────────────────
    let odata_filter: Option<String> = match &req.r#where {
        Some(v) => {
            let ast = parse(v)?;
            Some(odata::build(&ast, req.include_deleted)?)
        }
        None => {
            if req.include_deleted {
                None
            } else {
                Some("_deleted ne true".to_string())
            }
        }
    };

    // ── 5. Decide route ──────────────────────────────────────────────────────
    let use_knowledge_base =
        !config.disable_agentic && req.include_content == IncludeContent::AgenticAnswer;

    let include_full_body = matches!(req.include_content, IncludeContent::Full);
    let include_synthesis = matches!(req.include_content, IncludeContent::AgenticAnswer);

    // ── 6. Execute search ────────────────────────────────────────────────────
    if use_knowledge_base {
        // Single KB call — the KB groups indexes internally.
        let raw = api
            .search_knowledge_base(
                &config.knowledge_base_name,
                &req.query,
                odata_filter.as_deref(),
                top,
                req.cursor.as_deref(),
                include_synthesis,
                include_full_body,
            )
            .await?;

        // KB hits don't carry a per-hit data_source; use the first resolved source.
        // TODO(v2-follow-up): KB responses may include an index name per hit.
        let data_source_name = sources
            .first()
            .map(|(n, _)| n.as_str())
            .unwrap_or("unknown");
        let items = map_hits(raw.hits, data_source_name, req.include_content);

        return Ok(SearchResponse {
            total_estimate: raw.total_estimate,
            next_cursor: raw.next_cursor,
            answer: raw.answer,
            citations: raw.citations,
            items,
        });
    }

    // Direct per-index fan-out.
    // TODO(v2-follow-up): run in parallel; for now sequential is correct and simple.
    let mut all_hits: Vec<(String, RawHit)> = Vec::new();
    let mut total_estimate: u64 = 0;
    let mut last_cursor: Option<String> = None;

    // For single-source, pass the cursor directly (it's an opaque token from the index).
    // For multi-source, decode the per-source cursor map.
    let cursor_map = if sources.len() > 1 {
        decode_cursor_map(req.cursor.as_deref())
    } else {
        None
    };
    let single_cursor = if sources.len() == 1 {
        req.cursor.as_deref()
    } else {
        None
    };

    for (source_name, _kind) in &sources {
        let per_source_cursor = if sources.len() == 1 {
            single_cursor
        } else {
            cursor_map
                .as_ref()
                .and_then(|m| m.get(source_name.as_str()))
                .and_then(Value::as_str)
        };

        let raw = api
            .search_index(
                source_name,
                &req.query,
                odata_filter.as_deref(),
                top,
                per_source_cursor,
                include_full_body,
            )
            .await?;

        total_estimate = total_estimate.saturating_add(raw.total_estimate);

        if let Some(token) = raw.next_cursor {
            // Store per-source continuation for cursor encoding.
            last_cursor = Some(token);
        }

        for hit in raw.hits {
            all_hits.push((source_name.clone(), hit));
        }
    }

    // Sort merged hits by score descending, truncate to top.
    all_hits.sort_by(|(_, a), (_, b)| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    all_hits.truncate(top);

    let items: Vec<SearchItem> = all_hits
        .into_iter()
        .map(|(source, hit)| map_single_hit(hit, &source, req.include_content))
        .collect();

    // TODO(v2-follow-up): encode per-index cursors into a single cursor token.
    // For v1, just pass through the last single-source cursor if there was only
    // one source.
    let next_cursor = if sources.len() == 1 {
        last_cursor
    } else {
        // Multi-source: cursor is not yet implemented; omit.
        // TODO(v2-follow-up): encode per-index map.
        None
    };

    Ok(SearchResponse {
        items,
        answer: None,
        citations: None,
        next_cursor,
        total_estimate,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Decode a base64-encoded JSON cursor map (`{"source_name": "token", ...}`).
fn decode_cursor_map(cursor: Option<&str>) -> Option<serde_json::Map<String, Value>> {
    use base64::Engine;
    let encoded = cursor?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .ok()?;
    let val: Value = serde_json::from_slice(&bytes).ok()?;
    val.as_object().cloned()
}

/// Map a slice of [`RawHit`]s to [`SearchItem`]s.
fn map_hits(hits: Vec<RawHit>, data_source: &str, content: IncludeContent) -> Vec<SearchItem> {
    hits.into_iter()
        .map(|h| map_single_hit(h, data_source, content))
        .collect()
}

/// Map a single [`RawHit`] to a [`SearchItem`].
fn map_single_hit(hit: RawHit, data_source: &str, content: IncludeContent) -> SearchItem {
    // Extract source_link from fields; fall back to a synthetic value.
    let source_link = hit
        .fields
        .get("source_link")
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| {
            // TODO(v2-follow-up): synthesise a real URL from the index name + id.
            format!("urn:quelch:{}:{}", data_source, hit.id)
        });

    // Only return snippet / body depending on include_content mode.
    let snippet = match content {
        IncludeContent::Snippet | IncludeContent::Full | IncludeContent::AgenticAnswer => {
            hit.snippet.clone()
        }
    };
    let body = match content {
        IncludeContent::Full => hit.body.clone(),
        _ => None,
    };

    SearchItem {
        id: hit.id,
        score: hit.score,
        data_source: data_source.to_string(),
        source_link,
        snippet,
        body,
        fields: hit.fields,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::schema::SchemaCatalog;
    use crate::mcp::tools::search_api::mock::{MockSearchApi, SearchCall};
    use crate::mcp::tools::test_helpers::build_expose;

    fn build_expose_searchable() -> ExposeResolver {
        build_expose(&[
            ("jira_issues", "jira_issue", "jira-issues"),
            ("confluence_pages", "confluence_page", "confluence-pages"),
        ])
    }

    fn build_expose_single() -> ExposeResolver {
        build_expose(&[("jira_issues", "jira_issue", "jira-issues")])
    }

    fn build_expose_with_non_searchable() -> ExposeResolver {
        build_expose(&[
            ("jira_issues", "jira_issue", "jira-issues"),
            ("jira_sprints", "jira_sprint", "jira-sprints"),
        ])
    }

    fn default_config() -> SearchToolConfig {
        SearchToolConfig {
            disable_agentic: false,
            knowledge_base_name: "test-kb".to_string(),
            default_top: 25,
            max_top: 100,
        }
    }

    fn agentic_req(query: &str) -> SearchRequest {
        SearchRequest {
            query: query.to_string(),
            data_sources: Some(vec!["jira_issues".to_string()]),
            r#where: None,
            top: 10,
            cursor: None,
            include_deleted: false,
            include_content: IncludeContent::AgenticAnswer,
        }
    }

    fn snippet_req(query: &str) -> SearchRequest {
        SearchRequest {
            query: query.to_string(),
            data_sources: Some(vec!["jira_issues".to_string()]),
            r#where: None,
            top: 10,
            cursor: None,
            include_deleted: false,
            include_content: IncludeContent::Snippet,
        }
    }

    // ── Routing ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn search_routes_through_knowledge_base() {
        let api = MockSearchApi::new().with_kb_response(MockSearchApi::agentic_response);
        let expose = build_expose_single();
        let schema = SchemaCatalog::new();
        let config = default_config(); // disable_agentic = false

        let resp = run(&api, &expose, &schema, &config, agentic_req("test query"))
            .await
            .unwrap();

        let calls = api.calls_snapshot();
        assert_eq!(calls.len(), 1, "expected exactly one API call");
        assert!(
            matches!(&calls[0], SearchCall::KnowledgeBase { .. }),
            "expected KB call, got: {:?}",
            calls[0]
        );
        // KB response has an answer field
        assert!(resp.answer.is_some(), "answer should be populated");
    }

    #[tokio::test]
    async fn search_disable_agentic_routes_through_index() {
        let api = MockSearchApi::new();
        let expose = build_expose_single();
        let schema = SchemaCatalog::new();
        let config = SearchToolConfig {
            disable_agentic: true,
            ..default_config()
        };

        let resp = run(&api, &expose, &schema, &config, snippet_req("find bugs"))
            .await
            .unwrap();

        let calls = api.calls_snapshot();
        assert_eq!(calls.len(), 1);
        assert!(
            matches!(&calls[0], SearchCall::Index { .. }),
            "expected index call when disable_agentic=true, got: {:?}",
            calls[0]
        );
        assert!(resp.answer.is_none());
    }

    #[tokio::test]
    async fn search_snippet_routes_through_index_not_kb() {
        // Even with disable_agentic=false, snippet mode should NOT use KB.
        let api = MockSearchApi::new();
        let expose = build_expose_single();
        let schema = SchemaCatalog::new();
        let config = default_config(); // disable_agentic = false

        run(&api, &expose, &schema, &config, snippet_req("text"))
            .await
            .unwrap();

        let calls = api.calls_snapshot();
        assert!(
            matches!(&calls[0], SearchCall::Index { .. }),
            "snippet mode should use direct index search"
        );
    }

    // ── include_content modes ────────────────────────────────────────────────

    #[tokio::test]
    async fn search_include_content_full_returns_body() {
        let api = MockSearchApi::new().with_index_response(MockSearchApi::full_body_response);
        let expose = build_expose_single();
        let schema = SchemaCatalog::new();
        let config = SearchToolConfig {
            disable_agentic: true,
            ..default_config()
        };
        let req = SearchRequest {
            query: "test".to_string(),
            data_sources: Some(vec!["jira_issues".to_string()]),
            r#where: None,
            top: 10,
            cursor: None,
            include_deleted: false,
            include_content: IncludeContent::Full,
        };

        let resp = run(&api, &expose, &schema, &config, req).await.unwrap();

        let calls = api.calls_snapshot();
        assert!(
            matches!(
                &calls[0],
                SearchCall::Index {
                    include_full_body: true,
                    ..
                }
            ),
            "expected include_full_body=true in index call"
        );
        assert!(
            resp.items[0].body.is_some(),
            "items should have body populated for include_content=full"
        );
    }

    #[tokio::test]
    async fn search_include_content_agentic_answer_returns_answer_field() {
        let api = MockSearchApi::new().with_kb_response(MockSearchApi::agentic_response);
        let expose = build_expose_single();
        let schema = SchemaCatalog::new();
        let config = default_config();

        let resp = run(
            &api,
            &expose,
            &schema,
            &config,
            agentic_req("what is done?"),
        )
        .await
        .unwrap();

        assert!(
            resp.answer.is_some(),
            "answer should be present for agentic_answer"
        );
        assert_eq!(resp.answer.unwrap(), "The synthesised answer is: 42.");
        assert!(resp.citations.is_some(), "citations should be present");
    }

    // ── Soft-delete ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn search_excludes_soft_deleted_by_default() {
        let api = MockSearchApi::new();
        let expose = build_expose_single();
        let schema = SchemaCatalog::new();
        let config = SearchToolConfig {
            disable_agentic: true,
            ..default_config()
        };
        let req = SearchRequest {
            query: "bugs".to_string(),
            data_sources: Some(vec!["jira_issues".to_string()]),
            r#where: None,
            top: 10,
            cursor: None,
            include_deleted: false, // default
            include_content: IncludeContent::Snippet,
        };

        run(&api, &expose, &schema, &config, req).await.unwrap();

        let calls = api.calls_snapshot();
        if let SearchCall::Index { odata_filter, .. } = &calls[0] {
            let filter = odata_filter.as_deref().unwrap_or("");
            assert!(
                filter.contains("_deleted ne true"),
                "filter should exclude soft-deleted; got: {filter}"
            );
        } else {
            panic!("expected Index call");
        }
    }

    #[tokio::test]
    async fn search_includes_soft_deleted_when_set() {
        let api = MockSearchApi::new();
        let expose = build_expose_single();
        let schema = SchemaCatalog::new();
        let config = SearchToolConfig {
            disable_agentic: true,
            ..default_config()
        };
        let req = SearchRequest {
            query: "bugs".to_string(),
            data_sources: Some(vec!["jira_issues".to_string()]),
            r#where: None,
            top: 10,
            cursor: None,
            include_deleted: true,
            include_content: IncludeContent::Snippet,
        };

        run(&api, &expose, &schema, &config, req).await.unwrap();

        let calls = api.calls_snapshot();
        if let SearchCall::Index { odata_filter, .. } = &calls[0] {
            let has_deleted_guard = odata_filter
                .as_deref()
                .map(|f| f.contains("_deleted"))
                .unwrap_or(false);
            assert!(
                !has_deleted_guard,
                "filter should NOT contain _deleted predicate when include_deleted=true; got: {odata_filter:?}"
            );
        } else {
            panic!("expected Index call");
        }
    }

    // ── Access control ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn search_forbidden_for_unexposed_data_source() {
        let api = MockSearchApi::new();
        let expose = build_expose_single(); // only jira_issues
        let schema = SchemaCatalog::new();
        let config = SearchToolConfig {
            disable_agentic: true,
            ..default_config()
        };
        let req = SearchRequest {
            query: "test".to_string(),
            data_sources: Some(vec!["confluence_pages".to_string()]),
            r#where: None,
            top: 10,
            cursor: None,
            include_deleted: false,
            include_content: IncludeContent::Snippet,
        };

        let err = run(&api, &expose, &schema, &config, req).await.unwrap_err();
        assert!(
            matches!(err, McpError::Forbidden(_)),
            "expected Forbidden for unexposed source, got: {err:?}"
        );
    }

    // ── All searchable sources ────────────────────────────────────────────────

    #[tokio::test]
    async fn search_uses_all_searchable_when_data_sources_omitted() {
        let api = MockSearchApi::new();
        let expose = build_expose_with_non_searchable();
        let schema = SchemaCatalog::new();
        let config = SearchToolConfig {
            disable_agentic: true,
            ..default_config()
        };
        let req = SearchRequest {
            query: "open issues".to_string(),
            data_sources: None, // no explicit sources
            r#where: None,
            top: 10,
            cursor: None,
            include_deleted: false,
            include_content: IncludeContent::Snippet,
        };

        run(&api, &expose, &schema, &config, req).await.unwrap();

        let calls = api.calls_snapshot();
        // Only jira_issues is searchable; jira_sprints is not.
        let index_calls: Vec<&str> = calls
            .iter()
            .filter_map(|c| {
                if let SearchCall::Index { index_name, .. } = c {
                    Some(index_name.as_str())
                } else {
                    None
                }
            })
            .collect();

        assert!(
            index_calls.contains(&"jira_issues"),
            "should call jira_issues; got: {index_calls:?}"
        );
        assert!(
            !index_calls.contains(&"jira_sprints"),
            "should NOT call jira_sprints (not searchable); got: {index_calls:?}"
        );
    }

    #[tokio::test]
    async fn search_uses_all_exposed_searchable_when_no_data_sources() {
        let api = MockSearchApi::new();
        let expose = build_expose_searchable(); // jira_issues + confluence_pages both searchable
        let schema = SchemaCatalog::new();
        let config = SearchToolConfig {
            disable_agentic: true,
            ..default_config()
        };
        let req = SearchRequest {
            query: "team".to_string(),
            data_sources: None,
            r#where: None,
            top: 10,
            cursor: None,
            include_deleted: false,
            include_content: IncludeContent::Snippet,
        };

        run(&api, &expose, &schema, &config, req).await.unwrap();

        let calls = api.calls_snapshot();
        let index_calls: Vec<&str> = calls
            .iter()
            .filter_map(|c| {
                if let SearchCall::Index { index_name, .. } = c {
                    Some(index_name.as_str())
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(index_calls.len(), 2, "should call both searchable sources");
    }

    // ── Pagination ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn search_paginates_via_cursor() {
        // First call — no cursor.
        let api = MockSearchApi::new().with_index_response(|| {
            let mut r = MockSearchApi::default_response();
            r.next_cursor = Some("next-token-1".to_string());
            r
        });
        let expose = build_expose_single();
        let schema = SchemaCatalog::new();
        let config = SearchToolConfig {
            disable_agentic: true,
            ..default_config()
        };
        let req1 = SearchRequest {
            query: "paginate".to_string(),
            data_sources: Some(vec!["jira_issues".to_string()]),
            r#where: None,
            top: 1,
            cursor: None,
            include_deleted: false,
            include_content: IncludeContent::Snippet,
        };

        let resp1 = run(&api, &expose, &schema, &config, req1).await.unwrap();
        // Single source, so cursor should be threaded through.
        assert!(
            resp1.next_cursor.is_some(),
            "first page should have next_cursor"
        );
        let cursor = resp1.next_cursor.unwrap();

        // Second call — with cursor.
        let api2 = MockSearchApi::new();
        let req2 = SearchRequest {
            query: "paginate".to_string(),
            data_sources: Some(vec!["jira_issues".to_string()]),
            r#where: None,
            top: 1,
            cursor: Some(cursor),
            include_deleted: false,
            include_content: IncludeContent::Snippet,
        };

        run(&api2, &expose, &schema, &config, req2).await.unwrap();

        let calls2 = api2.calls_snapshot();
        if let SearchCall::Index { cursor, .. } = &calls2[0] {
            assert!(
                cursor.is_some(),
                "second request should pass cursor to the API"
            );
        }
    }

    // ── Conflict: agentic_answer + disable_agentic ────────────────────────────

    #[tokio::test]
    async fn search_agentic_answer_with_disable_agentic_returns_invalid_argument() {
        let api = MockSearchApi::new();
        let expose = build_expose_single();
        let schema = SchemaCatalog::new();
        let config = SearchToolConfig {
            disable_agentic: true,
            ..default_config()
        };

        let err = run(&api, &expose, &schema, &config, agentic_req("test"))
            .await
            .unwrap_err();

        assert!(
            matches!(err, McpError::InvalidArgument(_)),
            "expected InvalidArgument, got: {err:?}"
        );
    }
}
