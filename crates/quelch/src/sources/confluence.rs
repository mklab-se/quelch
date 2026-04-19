use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use tracing::debug;

use super::{FetchResult, SourceConnector, SourceDocument, SyncCursor};
use crate::config::ConfluenceSourceConfig;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const CHUNK_SIZE: usize = 4000;
const CHUNK_OVERLAP: usize = 200;

// ---------------------------------------------------------------------------
// Response types (shared between Cloud and DC)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct ConfluenceSearchResponse {
    pub results: Vec<ConfluenceResult>,
    #[serde(rename = "_links", default)]
    pub links: ConfluenceLinks,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct ConfluenceLinks {
    pub next: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConfluenceResult {
    #[serde(default)]
    pub id: String,
    pub title: Option<String>,
    pub space: Option<ConfluenceSpace>,
    pub body: Option<ConfluenceBody>,
    pub version: Option<ConfluenceVersion>,
    pub history: Option<ConfluenceHistory>,
    pub ancestors: Option<Vec<ConfluenceAncestor>>,
    pub metadata: Option<ConfluenceMetadata>,
    #[serde(rename = "_links", default)]
    pub links: Option<ConfluenceResultLinks>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct ConfluenceResultLinks {
    pub webui: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConfluenceSpace {
    pub key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConfluenceBody {
    pub storage: Option<ConfluenceStorage>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConfluenceStorage {
    pub value: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConfluenceVersion {
    pub when: Option<String>,
    pub by: Option<ConfluenceUser>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConfluenceHistory {
    #[serde(rename = "createdDate")]
    pub created_date: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConfluenceUser {
    /// Cloud: accountId, DC: username
    #[serde(rename = "accountId")]
    pub account_id: Option<String>,
    pub username: Option<String>,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConfluenceAncestor {
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConfluenceMetadata {
    pub labels: Option<ConfluenceLabels>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConfluenceLabels {
    pub results: Option<Vec<ConfluenceLabel>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConfluenceLabel {
    pub name: Option<String>,
}

// ---------------------------------------------------------------------------
// Chunk data
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Chunk {
    pub index: usize,
    pub heading: String,
    pub body: String,
}

// ---------------------------------------------------------------------------
// HTML stripping
// ---------------------------------------------------------------------------

/// Replace `<![CDATA[...]]>` sections with their inner text content.
fn extract_cdata(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut remaining = html;

    while let Some(start) = remaining.find("<![CDATA[") {
        result.push_str(&remaining[..start]);
        let after_open = start + 9; // len("<![CDATA[")
        if let Some(end) = remaining[after_open..].find("]]>") {
            result.push_str(&remaining[after_open..after_open + end]);
            remaining = &remaining[(after_open + end + 3)..]; // skip "]]>"
        } else {
            // Unclosed CDATA, just append the rest
            result.push_str(&remaining[start..]);
            remaining = "";
        }
    }
    result.push_str(remaining);
    result
}

/// Convert Confluence storage format XHTML to plain text.
pub fn strip_html(html: &str) -> String {
    // Pre-process: extract CDATA content, replacing CDATA sections with their text
    let preprocessed = extract_cdata(html);
    let html = &preprocessed;

    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut chars = html.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            in_tag = true;
            continue;
        }
        if ch == '>' {
            in_tag = false;
            // Add a space after closing tags to prevent words from merging
            result.push(' ');
            continue;
        }
        if in_tag {
            continue;
        }
        if ch == '&' {
            let mut entity = String::new();
            for ec in chars.by_ref() {
                if ec == ';' {
                    break;
                }
                entity.push(ec);
            }
            match entity.as_str() {
                "amp" => result.push('&'),
                "lt" => result.push('<'),
                "gt" => result.push('>'),
                "quot" => result.push('"'),
                "#39" | "apos" => result.push('\''),
                "nbsp" => result.push(' '),
                _ => {
                    // Unknown entity, keep as-is
                    result.push('&');
                    result.push_str(&entity);
                    result.push(';');
                }
            }
        } else {
            result.push(ch);
        }
    }

    // Collapse whitespace: multiple spaces/newlines into single newlines
    collapse_whitespace(&result)
}

fn collapse_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut last_was_whitespace = false;
    let mut last_was_newline = false;

    for ch in s.chars() {
        if ch == '\n' {
            if !last_was_newline {
                result.push('\n');
            }
            last_was_whitespace = true;
            last_was_newline = true;
        } else if ch.is_whitespace() {
            if !last_was_whitespace {
                result.push(' ');
            }
            last_was_whitespace = true;
        } else {
            result.push(ch);
            last_was_whitespace = false;
            last_was_newline = false;
        }
    }

    result.trim().to_string()
}

// ---------------------------------------------------------------------------
// Chunking
// ---------------------------------------------------------------------------

/// Split a Confluence page XHTML body into chunks based on headings.
///
/// Primary strategy: split on `<h1>`, `<h2>`, `<h3>` tags.
/// Fallback: fixed-size splitting at ~4000 chars with ~200 char overlap.
pub fn chunk_page(html_body: &str) -> Vec<Chunk> {
    let sections = split_on_headings(html_body);

    if sections.len() <= 1 {
        // No headings found — use fixed-size chunking on the entire body
        let text = strip_html(html_body);
        if text.is_empty() {
            return vec![Chunk {
                index: 0,
                heading: String::new(),
                body: String::new(),
            }];
        }
        return fixed_size_chunks(&text);
    }

    // Heading-based chunks
    sections
        .into_iter()
        .enumerate()
        .map(|(i, (heading, body_html))| {
            let body = strip_html(&body_html);
            Chunk {
                index: i,
                heading,
                body,
            }
        })
        .collect()
}

/// Find the next case-insensitive occurrence of a heading open tag (<h1>, <h2>, <h3>)
/// starting at `from`. Returns `(position, tag_end_position)` where tag_end_position
/// is the index right after the `>` of the opening tag.
fn find_heading_open(html: &str, from: usize) -> Option<(usize, usize)> {
    let bytes = html.as_bytes();
    let mut i = from;
    while i + 3 < bytes.len() {
        if bytes[i] == b'<'
            && (bytes[i + 1] == b'h' || bytes[i + 1] == b'H')
            && (bytes[i + 2] == b'1' || bytes[i + 2] == b'2' || bytes[i + 2] == b'3')
        {
            // Found `<h[123]`, now scan for the closing `>`
            let mut j = i + 3;
            while j < bytes.len() {
                if bytes[j] == b'>' {
                    return Some((i, j + 1));
                }
                j += 1;
            }
        }
        i += 1;
    }
    None
}

/// Find the closing heading tag (</h1>, </h2>, </h3>) starting at `from`.
/// Returns the position right after the closing `>`.
fn find_heading_close(html: &str, from: usize) -> Option<usize> {
    let bytes = html.as_bytes();
    let mut i = from;
    while i + 4 < bytes.len() {
        if bytes[i] == b'<'
            && bytes[i + 1] == b'/'
            && (bytes[i + 2] == b'h' || bytes[i + 2] == b'H')
            && (bytes[i + 3] == b'1' || bytes[i + 3] == b'2' || bytes[i + 3] == b'3')
        {
            // Found `</h[123]`, scan for `>`
            let mut j = i + 4;
            while j < bytes.len() {
                if bytes[j] == b'>' {
                    return Some(j + 1);
                }
                j += 1;
            }
        }
        i += 1;
    }
    None
}

/// Split HTML on heading tags, returning (heading_text, body_html) pairs.
/// The first element has an empty heading (content before first heading).
fn split_on_headings(html: &str) -> Vec<(String, String)> {
    // Collect all heading positions: (start_of_tag, heading_text)
    let mut positions: Vec<(usize, String)> = Vec::new();
    let mut search_from = 0;

    while let Some((tag_start, after_open)) = find_heading_open(html, search_from) {
        if let Some(after_close) = find_heading_close(html, after_open) {
            let heading_inner = &html[after_open..after_close];
            let heading_text = strip_html(heading_inner);
            positions.push((tag_start, heading_text));
            search_from = after_close;
        } else {
            break;
        }
    }

    if positions.is_empty() {
        return vec![(String::new(), html.to_string())];
    }

    let mut sections = Vec::new();

    // Content before first heading (chunk 0)
    let pre_heading = &html[..positions[0].0];
    sections.push((String::new(), pre_heading.to_string()));

    // Each heading section
    for (i, (pos, heading)) in positions.iter().enumerate() {
        let body_start = *pos;
        let body_end = if i + 1 < positions.len() {
            positions[i + 1].0
        } else {
            html.len()
        };
        sections.push((heading.clone(), html[body_start..body_end].to_string()));
    }

    sections
}

/// Fixed-size chunking with overlap for pages without headings.
fn fixed_size_chunks(text: &str) -> Vec<Chunk> {
    if text.len() <= CHUNK_SIZE {
        return vec![Chunk {
            index: 0,
            heading: String::new(),
            body: text.to_string(),
        }];
    }

    let mut chunks = Vec::new();
    let mut start = 0;

    while start < text.len() {
        let end = (start + CHUNK_SIZE).min(text.len());
        let chunk_text = &text[start..end];
        chunks.push(Chunk {
            index: chunks.len(),
            heading: String::new(),
            body: chunk_text.to_string(),
        });

        if end >= text.len() {
            break;
        }

        // Next chunk starts with overlap
        start = end - CHUNK_OVERLAP;
    }

    chunks
}

// ---------------------------------------------------------------------------
// Connector
// ---------------------------------------------------------------------------

pub struct ConfluenceConnector {
    client: Client,
    config: ConfluenceSourceConfig,
    is_cloud: bool,
}

impl ConfluenceConnector {
    pub fn new(config: &ConfluenceSourceConfig) -> Self {
        let is_cloud = config.auth.is_cloud();
        Self {
            client: Client::new(),
            config: config.clone(),
            is_cloud,
        }
    }

    /// Build the CQL query string for a specific subsource (space key).
    pub(crate) fn build_cql_for(&self, subsource: &str, cursor: Option<&SyncCursor>) -> String {
        let space_clause = format!("space = {subsource}");

        match cursor {
            Some(c) => {
                let ts = c.last_updated.format("%Y-%m-%d");
                format!(
                    "{space_clause} AND type = page AND lastmodified >= \"{ts}\" ORDER BY lastmodified ASC"
                )
            }
            None => {
                format!("{space_clause} AND type = page ORDER BY lastmodified ASC")
            }
        }
    }

    /// Build the browse URL for a page.
    pub(crate) fn browse_url(&self, page: &ConfluenceResult) -> String {
        let base = self.config.url.trim_end_matches('/');
        if self.is_cloud {
            let space_key = page
                .space
                .as_ref()
                .and_then(|s| s.key.as_deref())
                .unwrap_or("UNKNOWN");
            format!("{base}/spaces/{space_key}/pages/{}", page.id)
        } else {
            // DC: use _links.webui if available
            if let Some(links) = &page.links
                && let Some(webui) = &links.webui
            {
                return format!("{base}{webui}");
            }
            format!("{base}/pages/viewpage.action?pageId={}", page.id)
        }
    }

    /// Extract the author identifier from a Confluence result.
    fn extract_author(&self, result: &ConfluenceResult) -> String {
        result
            .version
            .as_ref()
            .and_then(|v| v.by.as_ref())
            .map(|user| {
                user.display_name
                    .as_deref()
                    .or(user.account_id.as_deref())
                    .or(user.username.as_deref())
                    .unwrap_or("")
                    .to_string()
            })
            .unwrap_or_default()
    }

    /// Extract labels from metadata.
    fn extract_labels(result: &ConfluenceResult) -> Vec<String> {
        result
            .metadata
            .as_ref()
            .and_then(|m| m.labels.as_ref())
            .and_then(|l| l.results.as_ref())
            .map(|labels| {
                labels
                    .iter()
                    .filter_map(|l| l.name.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    /// Build ancestors string like "Parent > Grandparent > ...".
    fn build_ancestors(result: &ConfluenceResult) -> String {
        result
            .ancestors
            .as_ref()
            .map(|ancestors| {
                ancestors
                    .iter()
                    .filter_map(|a| a.title.clone())
                    .collect::<Vec<_>>()
                    .join(" > ")
            })
            .unwrap_or_default()
    }

    /// Parse Confluence datetime formats.
    fn parse_datetime(s: &str) -> Option<DateTime<Utc>> {
        // Try ISO 8601 with timezone offset
        DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
            .or_else(|| {
                DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.3f%z")
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            })
    }

    /// Convert a single Confluence page result into one or more SourceDocuments (chunks).
    fn page_to_documents(&self, result: &ConfluenceResult) -> Vec<SourceDocument> {
        let title = result.title.clone().unwrap_or_default();
        let space_key = result
            .space
            .as_ref()
            .and_then(|s| s.key.clone())
            .unwrap_or_default();
        let page_id = result.id.clone();
        let url = self.browse_url(result);
        let author = self.extract_author(result);
        let labels = Self::extract_labels(result);
        let ancestors = Self::build_ancestors(result);

        let html_body = result
            .body
            .as_ref()
            .and_then(|b| b.storage.as_ref())
            .and_then(|s| s.value.clone())
            .unwrap_or_default();

        let updated_at = result
            .version
            .as_ref()
            .and_then(|v| v.when.as_deref())
            .and_then(Self::parse_datetime)
            .unwrap_or_else(Utc::now);

        let created_at = result
            .history
            .as_ref()
            .and_then(|h| h.created_date.as_deref())
            .and_then(Self::parse_datetime)
            .unwrap_or(updated_at);

        let chunks = chunk_page(&html_body);

        chunks
            .into_iter()
            .map(|chunk| {
                // Build labeled content for embedding
                let heading_label = if chunk.heading.is_empty() {
                    String::new()
                } else {
                    format!("\nSection: {}", chunk.heading)
                };
                let content = format!(
                    "Page: {title}{heading_label}\nSpace: {space_key}\nAuthor: {author}\nLabels: {label_str}\n\n{body}",
                    label_str = labels.join(", "),
                    body = chunk.body,
                );

                let doc_id = format!("{}-{}-{}", self.config.name, page_id, chunk.index);

                let mut fields = HashMap::new();
                fields.insert("id".to_string(), serde_json::json!(doc_id));
                fields.insert("url".to_string(), serde_json::json!(url));
                fields.insert(
                    "source_name".to_string(),
                    serde_json::json!(self.config.name),
                );
                fields.insert("source_type".to_string(), serde_json::json!("confluence"));
                fields.insert("space_key".to_string(), serde_json::json!(space_key));
                fields.insert("page_id".to_string(), serde_json::json!(page_id));
                fields.insert("page_title".to_string(), serde_json::json!(title));
                fields.insert(
                    "chunk_index".to_string(),
                    serde_json::json!(chunk.index as i32),
                );
                fields.insert(
                    "chunk_heading".to_string(),
                    serde_json::json!(chunk.heading),
                );
                fields.insert("body".to_string(), serde_json::json!(chunk.body));
                fields.insert("labels".to_string(), serde_json::json!(labels));
                fields.insert("author".to_string(), serde_json::json!(author));
                fields.insert("ancestors".to_string(), serde_json::json!(ancestors));
                fields.insert("content".to_string(), serde_json::json!(content));
                fields.insert(
                    "created_at".to_string(),
                    serde_json::json!(created_at.to_rfc3339()),
                );
                fields.insert(
                    "updated_at".to_string(),
                    serde_json::json!(updated_at.to_rfc3339()),
                );

                SourceDocument {
                    id: doc_id,
                    fields,
                    updated_at,
                }
            })
            .collect()
    }

    /// Search endpoint path differs between Cloud and DC.
    fn search_path(&self) -> &str {
        if self.is_cloud {
            "/rest/api/search"
        } else {
            "/rest/api/content/search"
        }
    }

    // -----------------------------------------------------------------------
    // Fetch changes
    // -----------------------------------------------------------------------

    async fn fetch_changes_impl(
        &self,
        subsource: &str,
        cursor: Option<&SyncCursor>,
        batch_size: usize,
    ) -> Result<FetchResult> {
        let cql = self.build_cql_for(subsource, cursor);
        let base = self.config.url.trim_end_matches('/');
        let url = format!("{base}{}", self.search_path());
        let auth_header = self.config.auth.authorization_header();

        debug!(
            source = self.config.name,
            cql = cql,
            cloud = self.is_cloud,
            "Fetching Confluence pages"
        );

        let resp = self
            .client
            .get(&url)
            .header("Authorization", &auth_header)
            .query(&[
                ("cql", cql.as_str()),
                ("limit", &batch_size.to_string()),
                ("start", "0"),
                (
                    "expand",
                    "body.storage,version,ancestors,metadata.labels,space,history",
                ),
            ])
            .send()
            .await
            .context("failed to connect to Confluence")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Confluence API error ({}): {}", status, body);
        }

        let search_resp: ConfluenceSearchResponse = resp
            .json()
            .await
            .context("failed to parse Confluence response")?;

        let has_more = search_resp.links.next.is_some();

        let mut documents = Vec::new();
        for result in &search_resp.results {
            let docs = self.page_to_documents(result);
            documents.extend(docs);
        }

        let new_cursor = documents
            .last()
            .map(|doc| SyncCursor {
                last_updated: doc.updated_at,
            })
            .or_else(|| cursor.cloned())
            .unwrap_or(SyncCursor {
                last_updated: Utc::now(),
            });

        debug!(
            source = self.config.name,
            pages = search_resp.results.len(),
            chunks = documents.len(),
            has_more = has_more,
            "Fetched Confluence pages"
        );

        Ok(FetchResult {
            documents,
            cursor: new_cursor,
            has_more,
        })
    }

    // -----------------------------------------------------------------------
    // Fetch all IDs
    // -----------------------------------------------------------------------

    async fn fetch_all_ids_impl(&self, subsource: &str) -> Result<Vec<String>> {
        let mut all_ids = Vec::new();
        let mut start: usize = 0;
        let page_size: usize = 25;
        let auth_header = self.config.auth.authorization_header();
        let cql = self.build_cql_for(subsource, None);
        let base = self.config.url.trim_end_matches('/');

        loop {
            let url = format!("{base}{}", self.search_path());

            let resp = self
                .client
                .get(&url)
                .header("Authorization", &auth_header)
                .query(&[
                    ("cql", cql.as_str()),
                    ("limit", &page_size.to_string()),
                    ("start", &start.to_string()),
                    ("expand", ""),
                ])
                .send()
                .await
                .context("failed to connect to Confluence for ID fetch")?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("Confluence API error ({}): {}", status, body);
            }

            let search_resp: ConfluenceSearchResponse = resp.json().await?;
            let batch_len = search_resp.results.len();

            for result in &search_resp.results {
                // We return page-level IDs; the caller must handle chunk IDs
                all_ids.push(format!("{}-{}", self.config.name, result.id));
            }

            if batch_len == 0 || search_resp.links.next.is_none() {
                break;
            }
            start += batch_len;
        }

        Ok(all_ids)
    }
}

impl SourceConnector for ConfluenceConnector {
    fn source_type(&self) -> &str {
        "confluence"
    }

    fn source_name(&self) -> &str {
        &self.config.name
    }

    fn index_name(&self) -> &str {
        &self.config.index
    }

    fn subsources(&self) -> &[String] {
        &self.config.spaces
    }

    async fn fetch_changes(
        &self,
        subsource: &str,
        cursor: Option<&SyncCursor>,
        batch_size: usize,
    ) -> Result<FetchResult> {
        self.fetch_changes_impl(subsource, cursor, batch_size).await
    }

    async fn fetch_all_ids(&self, subsource: &str) -> Result<Vec<String>> {
        self.fetch_all_ids_impl(subsource).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthConfig;

    fn dc_config() -> ConfluenceSourceConfig {
        ConfluenceSourceConfig {
            name: "test-confluence".to_string(),
            url: "https://confluence.example.com".to_string(),
            auth: AuthConfig::DataCenter {
                pat: "fake-pat".to_string(),
            },
            spaces: vec!["ENG".to_string()],
            index: "confluence-pages".to_string(),
        }
    }

    fn cloud_config() -> ConfluenceSourceConfig {
        ConfluenceSourceConfig {
            name: "test-cloud".to_string(),
            url: "https://mycompany.atlassian.net/wiki".to_string(),
            auth: AuthConfig::Cloud {
                email: "user@example.com".to_string(),
                api_token: "token".to_string(),
            },
            spaces: vec!["PROJ".to_string()],
            index: "confluence-pages".to_string(),
        }
    }

    // -----------------------------------------------------------------------
    // strip_html tests
    // -----------------------------------------------------------------------

    #[test]
    fn strip_html_basic_tags() {
        let html = "<p>Hello <strong>world</strong></p>";
        let text = strip_html(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("world"));
        assert!(!text.contains("<p>"));
        assert!(!text.contains("<strong>"));
    }

    #[test]
    fn strip_html_entities() {
        let html = "5 &gt; 3 &amp; 2 &lt; 4 &quot;quoted&quot; &#39;apos&#39;";
        let text = strip_html(html);
        assert!(text.contains("5 > 3 & 2 < 4 \"quoted\" 'apos'"));
    }

    #[test]
    fn strip_html_nbsp() {
        let html = "hello&nbsp;world";
        let text = strip_html(html);
        assert!(text.contains("hello world"));
    }

    #[test]
    fn strip_html_nested_elements() {
        let html = "<div><p>Outer <span>inner <em>deep</em></span></p></div>";
        let text = strip_html(html);
        assert!(text.contains("Outer"));
        assert!(text.contains("inner"));
        assert!(text.contains("deep"));
    }

    #[test]
    fn strip_html_confluence_macros() {
        let html = r#"<ac:structured-macro ac:name="code"><ac:plain-text-body><![CDATA[fn main() {}]]></ac:plain-text-body></ac:structured-macro>"#;
        let text = strip_html(html);
        // Should extract text content from macros
        assert!(text.contains("fn main()"));
    }

    #[test]
    fn strip_html_collapses_whitespace() {
        let html = "<p>  Hello  </p>\n\n\n<p>  World  </p>";
        let text = strip_html(html);
        // Should not have excessive whitespace
        assert!(!text.contains("\n\n\n"));
    }

    #[test]
    fn strip_html_empty() {
        assert_eq!(strip_html(""), "");
    }

    #[test]
    fn strip_html_plain_text() {
        assert_eq!(strip_html("just plain text"), "just plain text");
    }

    // -----------------------------------------------------------------------
    // chunk_page tests
    // -----------------------------------------------------------------------

    #[test]
    fn chunk_page_with_headings() {
        let html = r#"<p>Introduction paragraph</p><h1>First Section</h1><p>Content of first section</p><h2>Sub Section</h2><p>Content of sub section</p><h1>Second Section</h1><p>Content of second section</p>"#;

        let chunks = chunk_page(html);
        assert_eq!(chunks.len(), 4); // pre-heading + 3 heading sections

        // Chunk 0: content before first heading
        assert_eq!(chunks[0].index, 0);
        assert_eq!(chunks[0].heading, "");
        assert!(chunks[0].body.contains("Introduction"));

        // Chunk 1: first heading
        assert_eq!(chunks[1].index, 1);
        assert_eq!(chunks[1].heading, "First Section");
        assert!(chunks[1].body.contains("Content of first section"));

        // Chunk 2: sub section
        assert_eq!(chunks[2].index, 2);
        assert_eq!(chunks[2].heading, "Sub Section");
        assert!(chunks[2].body.contains("Content of sub section"));

        // Chunk 3: second section
        assert_eq!(chunks[3].index, 3);
        assert_eq!(chunks[3].heading, "Second Section");
        assert!(chunks[3].body.contains("Content of second section"));
    }

    #[test]
    fn chunk_page_no_headings_short() {
        let html = "<p>Short page with no headings</p>";
        let chunks = chunk_page(html);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].index, 0);
        assert_eq!(chunks[0].heading, "");
        assert!(chunks[0].body.contains("Short page"));
    }

    #[test]
    fn chunk_page_no_headings_long() {
        // Create a body longer than CHUNK_SIZE
        let long_text = "A ".repeat(3000); // 6000 chars
        let html = format!("<p>{long_text}</p>");
        let chunks = chunk_page(&html);

        assert!(chunks.len() > 1, "Should split into multiple chunks");

        // All chunks should have empty heading
        for chunk in &chunks {
            assert_eq!(chunk.heading, "");
        }

        // Verify indices are sequential
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.index, i);
        }
    }

    #[test]
    fn chunk_page_mixed_heading_levels() {
        let html = "<h1>Title</h1><p>Intro</p><h2>Sub</h2><p>Sub content</p><h3>Sub-sub</h3><p>Deep content</p>";

        let chunks = chunk_page(html);
        // pre-heading (empty) + h1 + h2 + h3
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[1].heading, "Title");
        assert_eq!(chunks[2].heading, "Sub");
        assert_eq!(chunks[3].heading, "Sub-sub");
    }

    #[test]
    fn chunk_page_empty_body() {
        let chunks = chunk_page("");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].index, 0);
        assert!(chunks[0].body.is_empty());
    }

    #[test]
    fn fixed_size_chunks_overlap() {
        let text = "x".repeat(CHUNK_SIZE + 1000);
        let chunks = fixed_size_chunks(&text);
        assert!(chunks.len() >= 2);

        // Verify overlap: end of chunk 0 overlaps with start of chunk 1
        let end_of_first = &chunks[0].body[chunks[0].body.len() - CHUNK_OVERLAP..];
        let start_of_second = &chunks[1].body[..CHUNK_OVERLAP];
        assert_eq!(end_of_first, start_of_second);
    }

    #[test]
    fn chunk_page_heading_with_attributes() {
        let html = r#"<h1 class="title" id="top">Styled Heading</h1><p>Some content</p>"#;
        let chunks = chunk_page(html);
        assert_eq!(chunks.len(), 2); // pre-heading + heading section
        assert_eq!(chunks[1].heading, "Styled Heading");
    }

    #[test]
    fn chunk_page_h4_not_split() {
        // h4+ should not cause splits
        let html = "<h4>Not a split point</h4><p>Content under h4</p>";
        let chunks = chunk_page(html);
        assert_eq!(chunks.len(), 1); // no split on h4
    }

    // -----------------------------------------------------------------------
    // CQL building tests
    // -----------------------------------------------------------------------

    #[test]
    fn builds_cql_without_cursor() {
        let connector = ConfluenceConnector::new(&dc_config());
        let cql = connector.build_cql_for("ENG", None);
        assert_eq!(cql, "space = ENG AND type = page ORDER BY lastmodified ASC");
    }

    #[test]
    fn builds_cql_with_cursor() {
        let connector = ConfluenceConnector::new(&dc_config());
        let cursor = SyncCursor {
            last_updated: DateTime::parse_from_rfc3339("2025-01-15T10:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
        };
        let cql = connector.build_cql_for("ENG", Some(&cursor));
        assert!(cql.contains("lastmodified >= \"2025-01-15\""));
        assert!(cql.contains("space = ENG"));
        assert!(cql.contains("type = page"));
    }

    #[test]
    fn subsources_returns_space_keys() {
        let mut config = dc_config();
        config.spaces = vec!["ENG".to_string(), "OPS".to_string()];
        let connector = ConfluenceConnector::new(&config);
        assert_eq!(
            connector.subsources(),
            &["ENG".to_string(), "OPS".to_string()]
        );
    }

    #[test]
    fn builds_cql_for_single_subsource() {
        let mut config = dc_config();
        config.spaces = vec!["ENG".to_string(), "OPS".to_string()];
        let connector = ConfluenceConnector::new(&config);
        let cql = connector.build_cql_for("OPS", None);
        assert_eq!(cql, "space = OPS AND type = page ORDER BY lastmodified ASC");
    }

    // -----------------------------------------------------------------------
    // Cloud vs DC detection
    // -----------------------------------------------------------------------

    #[test]
    fn detects_cloud_vs_dc() {
        let dc = ConfluenceConnector::new(&dc_config());
        assert!(!dc.is_cloud);

        let cloud = ConfluenceConnector::new(&cloud_config());
        assert!(cloud.is_cloud);
    }

    #[test]
    fn cloud_uses_search_endpoint() {
        let cloud = ConfluenceConnector::new(&cloud_config());
        assert_eq!(cloud.search_path(), "/rest/api/search");
    }

    #[test]
    fn dc_uses_content_search_endpoint() {
        let dc = ConfluenceConnector::new(&dc_config());
        assert_eq!(dc.search_path(), "/rest/api/content/search");
    }

    // -----------------------------------------------------------------------
    // Browse URL construction
    // -----------------------------------------------------------------------

    #[test]
    fn browse_url_cloud() {
        let connector = ConfluenceConnector::new(&cloud_config());
        let result = ConfluenceResult {
            id: "12345".to_string(),
            title: Some("Test Page".to_string()),
            space: Some(ConfluenceSpace {
                key: Some("PROJ".to_string()),
            }),
            body: None,
            version: None,
            history: None,
            ancestors: None,
            metadata: None,
            links: None,
        };
        assert_eq!(
            connector.browse_url(&result),
            "https://mycompany.atlassian.net/wiki/spaces/PROJ/pages/12345"
        );
    }

    #[test]
    fn browse_url_dc_with_webui() {
        let connector = ConfluenceConnector::new(&dc_config());
        let result = ConfluenceResult {
            id: "12345".to_string(),
            title: Some("Test Page".to_string()),
            space: Some(ConfluenceSpace {
                key: Some("ENG".to_string()),
            }),
            body: None,
            version: None,
            history: None,
            ancestors: None,
            metadata: None,
            links: Some(ConfluenceResultLinks {
                webui: Some("/display/ENG/Test+Page".to_string()),
            }),
        };
        assert_eq!(
            connector.browse_url(&result),
            "https://confluence.example.com/display/ENG/Test+Page"
        );
    }

    #[test]
    fn browse_url_dc_without_webui() {
        let connector = ConfluenceConnector::new(&dc_config());
        let result = ConfluenceResult {
            id: "12345".to_string(),
            title: None,
            space: None,
            body: None,
            version: None,
            history: None,
            ancestors: None,
            metadata: None,
            links: None,
        };
        assert_eq!(
            connector.browse_url(&result),
            "https://confluence.example.com/pages/viewpage.action?pageId=12345"
        );
    }

    // -----------------------------------------------------------------------
    // Page-to-document conversion
    // -----------------------------------------------------------------------

    #[test]
    fn page_to_documents_basic() {
        let connector = ConfluenceConnector::new(&dc_config());
        let result = ConfluenceResult {
            id: "99".to_string(),
            title: Some("Architecture Overview".to_string()),
            space: Some(ConfluenceSpace {
                key: Some("ENG".to_string()),
            }),
            body: Some(ConfluenceBody {
                storage: Some(ConfluenceStorage {
                    value: Some(
                        "<h1>Introduction</h1><p>This is the intro</p><h2>Details</h2><p>More details here</p>"
                            .to_string(),
                    ),
                }),
            }),
            version: Some(ConfluenceVersion {
                when: Some("2025-03-01T12:00:00.000+0000".to_string()),
                by: Some(ConfluenceUser {
                    account_id: Some("abc123".to_string()),
                    username: None,
                    display_name: Some("Alice".to_string()),
                }),
            }),
            history: Some(ConfluenceHistory {
                created_date: Some("2025-01-01T00:00:00.000+0000".to_string()),
            }),
            ancestors: Some(vec![
                ConfluenceAncestor {
                    title: Some("Engineering".to_string()),
                },
                ConfluenceAncestor {
                    title: Some("Docs".to_string()),
                },
            ]),
            metadata: Some(ConfluenceMetadata {
                labels: Some(ConfluenceLabels {
                    results: Some(vec![
                        ConfluenceLabel {
                            name: Some("architecture".to_string()),
                        },
                        ConfluenceLabel {
                            name: Some("backend".to_string()),
                        },
                    ]),
                }),
            }),
            links: None,
        };

        let docs = connector.page_to_documents(&result);
        assert_eq!(docs.len(), 3); // pre-heading + 2 heading sections

        // Check first chunk (pre-heading, empty in this case)
        let doc0 = &docs[0];
        assert_eq!(doc0.id, "test-confluence-99-0");
        assert_eq!(doc0.fields["source_type"], "confluence");
        assert_eq!(doc0.fields["space_key"], "ENG");
        assert_eq!(doc0.fields["page_id"], "99");
        assert_eq!(doc0.fields["page_title"], "Architecture Overview");
        assert_eq!(doc0.fields["chunk_index"], 0);
        assert_eq!(doc0.fields["author"], "Alice");
        assert_eq!(doc0.fields["ancestors"], "Engineering > Docs");
        assert_eq!(
            doc0.fields["labels"],
            serde_json::json!(["architecture", "backend"])
        );

        // Check heading chunk
        let doc1 = &docs[1];
        assert_eq!(doc1.id, "test-confluence-99-1");
        assert_eq!(doc1.fields["chunk_heading"], "Introduction");
        assert!(
            doc1.fields["body"]
                .as_str()
                .unwrap()
                .contains("This is the intro")
        );

        // Content field should have labeled format
        let content = doc1.fields["content"].as_str().unwrap();
        assert!(content.contains("Page: Architecture Overview"));
        assert!(content.contains("Section: Introduction"));
        assert!(content.contains("This is the intro"));

        // Chunk 2
        let doc2 = &docs[2];
        assert_eq!(doc2.fields["chunk_heading"], "Details");
        assert!(
            doc2.fields["body"]
                .as_str()
                .unwrap()
                .contains("More details here")
        );
    }

    #[test]
    fn page_to_documents_no_headings() {
        let connector = ConfluenceConnector::new(&dc_config());
        let result = ConfluenceResult {
            id: "50".to_string(),
            title: Some("Simple Page".to_string()),
            space: Some(ConfluenceSpace {
                key: Some("ENG".to_string()),
            }),
            body: Some(ConfluenceBody {
                storage: Some(ConfluenceStorage {
                    value: Some("<p>Just a simple paragraph</p>".to_string()),
                }),
            }),
            version: Some(ConfluenceVersion {
                when: Some("2025-06-01T09:00:00.000+0000".to_string()),
                by: None,
            }),
            history: None,
            ancestors: None,
            metadata: None,
            links: None,
        };

        let docs = connector.page_to_documents(&result);
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].id, "test-confluence-50-0");
        assert_eq!(docs[0].fields["chunk_heading"], "");

        let content = docs[0].fields["content"].as_str().unwrap();
        assert!(content.contains("Page: Simple Page"));
        assert!(content.contains("Just a simple paragraph"));
    }

    #[test]
    fn page_to_documents_url_cloud() {
        let connector = ConfluenceConnector::new(&cloud_config());
        let result = ConfluenceResult {
            id: "777".to_string(),
            title: Some("Cloud Page".to_string()),
            space: Some(ConfluenceSpace {
                key: Some("PROJ".to_string()),
            }),
            body: Some(ConfluenceBody {
                storage: Some(ConfluenceStorage {
                    value: Some("<p>Cloud content</p>".to_string()),
                }),
            }),
            version: None,
            history: None,
            ancestors: None,
            metadata: None,
            links: None,
        };

        let docs = connector.page_to_documents(&result);
        assert_eq!(
            docs[0].fields["url"],
            "https://mycompany.atlassian.net/wiki/spaces/PROJ/pages/777"
        );
    }

    #[test]
    fn page_to_documents_handles_null_fields() {
        let connector = ConfluenceConnector::new(&dc_config());
        let result = ConfluenceResult {
            id: "1".to_string(),
            title: None,
            space: None,
            body: None,
            version: None,
            history: None,
            ancestors: None,
            metadata: None,
            links: None,
        };

        let docs = connector.page_to_documents(&result);
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].fields["page_title"], "");
        assert_eq!(docs[0].fields["space_key"], "");
        assert_eq!(docs[0].fields["author"], "");
        assert_eq!(docs[0].fields["ancestors"], "");
        assert_eq!(docs[0].fields["labels"], serde_json::json!([]));
    }

    // -----------------------------------------------------------------------
    // Source connector trait impl
    // -----------------------------------------------------------------------

    #[test]
    fn source_type_is_confluence() {
        let connector = ConfluenceConnector::new(&dc_config());
        assert_eq!(connector.source_type(), "confluence");
    }

    #[test]
    fn source_name_from_config() {
        let connector = ConfluenceConnector::new(&dc_config());
        assert_eq!(connector.source_name(), "test-confluence");
    }

    #[test]
    fn index_name_from_config() {
        let connector = ConfluenceConnector::new(&dc_config());
        assert_eq!(connector.index_name(), "confluence-pages");
    }

    // -----------------------------------------------------------------------
    // Datetime parsing
    // -----------------------------------------------------------------------

    #[test]
    fn parses_rfc3339_datetime() {
        let dt = ConfluenceConnector::parse_datetime("2025-01-15T10:30:00.000Z").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-15T10:30:00+00:00");
    }

    #[test]
    fn parses_offset_datetime() {
        let dt = ConfluenceConnector::parse_datetime("2025-01-15T10:30:00.000+0000").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-15T10:30:00+00:00");
    }

    #[test]
    fn returns_none_for_invalid_datetime() {
        assert!(ConfluenceConnector::parse_datetime("not-a-date").is_none());
    }
}
