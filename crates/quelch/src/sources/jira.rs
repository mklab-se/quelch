use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use tracing::debug;

use super::{FetchResult, SourceConnector, SourceDocument, SyncCursor};
use crate::config::JiraSourceConfig;

pub struct JiraConnector {
    client: Client,
    config: JiraSourceConfig,
    is_cloud: bool,
}

// ---------------------------------------------------------------------------
// Data Center response types (REST API v2: /rest/api/2/search)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct DcSearchResponse {
    issues: Vec<JiraIssue>,
    total: u64,
    #[serde(rename = "startAt")]
    start_at: u64,
}

// ---------------------------------------------------------------------------
// Cloud response types (REST API v3: /rest/api/3/search/jql)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CloudSearchResponse {
    issues: Vec<JiraIssue>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
    #[serde(rename = "isLast", default)]
    is_last: bool,
}

// ---------------------------------------------------------------------------
// Shared issue types (work for both v2 and v3 responses)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct JiraIssue {
    key: String,
    fields: JiraFields,
}

#[derive(Debug, Deserialize)]
struct JiraFields {
    summary: Option<String>,
    /// In DC (v2): plain text / wiki markup string.
    /// In Cloud (v3): ADF JSON object.
    /// We deserialize as Value to handle both.
    description: Option<serde_json::Value>,
    status: Option<JiraStatus>,
    priority: Option<JiraNamedField>,
    assignee: Option<JiraUser>,
    reporter: Option<JiraUser>,
    issuetype: Option<JiraNamedField>,
    labels: Option<Vec<String>>,
    created: Option<String>,
    updated: Option<String>,
    comment: Option<JiraCommentContainer>,
    project: Option<JiraProject>,
}

#[derive(Debug, Deserialize)]
struct JiraStatus {
    name: Option<String>,
    #[serde(rename = "statusCategory")]
    status_category: Option<JiraStatusCategory>,
}

#[derive(Debug, Deserialize)]
struct JiraStatusCategory {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JiraNamedField {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JiraUser {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

/// Jira comment field is a paginated sub-object.
#[derive(Debug, Deserialize)]
struct JiraCommentContainer {
    comments: Option<Vec<JiraComment>>,
}

#[derive(Debug, Deserialize)]
struct JiraComment {
    /// In DC (v2): plain text string.
    /// In Cloud (v3): ADF JSON object.
    body: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct JiraProject {
    key: Option<String>,
}

impl JiraConnector {
    pub fn new(config: &JiraSourceConfig) -> Self {
        let is_cloud = config.auth.is_cloud();
        Self {
            client: Client::new(),
            config: config.clone(),
            is_cloud,
        }
    }

    fn build_jql(&self, cursor: Option<&SyncCursor>) -> String {
        let project_clause = self
            .config
            .projects
            .iter()
            .map(|p| format!("project = {p}"))
            .collect::<Vec<_>>()
            .join(" OR ");

        let project_jql = if self.config.projects.len() > 1 {
            format!("({project_clause})")
        } else {
            project_clause
        };

        match cursor {
            Some(c) => {
                let ts = c.last_updated.format("%Y-%m-%d %H:%M");
                format!("{project_jql} AND updated >= \"{ts}\" ORDER BY updated ASC")
            }
            None => format!("{project_jql} ORDER BY updated ASC"),
        }
    }

    /// Build the browse URL for an issue: `{base_url}/browse/{key}`
    fn browse_url(&self, issue_key: &str) -> String {
        let base = self.config.url.trim_end_matches('/');
        format!("{base}/browse/{issue_key}")
    }

    /// Parse Jira timestamp format: "2024-04-10T14:33:21.872+0000"
    fn parse_datetime(s: &str) -> Option<DateTime<Utc>> {
        DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.3f%z")
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    }

    /// Extract plain text from a field that may be a string (v2/DC) or ADF object (v3/Cloud).
    fn extract_text(value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Object(_) => Self::extract_text_from_adf(value),
            serde_json::Value::Null => String::new(),
            _ => value.to_string(),
        }
    }

    /// Recursively extract plain text from an Atlassian Document Format (ADF) object.
    fn extract_text_from_adf(node: &serde_json::Value) -> String {
        if let Some(text) = node.get("text").and_then(|t| t.as_str()) {
            return text.to_string();
        }

        if let Some(content) = node.get("content").and_then(|c| c.as_array()) {
            let parts: Vec<String> = content.iter().map(Self::extract_text_from_adf).collect();

            let node_type = node.get("type").and_then(|t| t.as_str()).unwrap_or("");
            return match node_type {
                "paragraph" | "heading" | "blockquote" | "codeBlock" | "rule" | "bulletList"
                | "orderedList" | "listItem" | "table" | "tableRow" | "tableCell"
                | "tableHeader" => {
                    let joined = parts.join("");
                    if joined.is_empty() {
                        String::new()
                    } else {
                        format!("{joined}\n")
                    }
                }
                _ => parts.join(""),
            };
        }

        String::new()
    }

    fn issue_to_document(&self, issue: &JiraIssue) -> SourceDocument {
        let fields = &issue.fields;

        let summary = fields.summary.clone().unwrap_or_default();
        let description = fields
            .description
            .as_ref()
            .map(Self::extract_text)
            .unwrap_or_default();

        let comments_text = fields
            .comment
            .as_ref()
            .and_then(|c| c.comments.as_ref())
            .map(|comments| {
                comments
                    .iter()
                    .filter_map(|c| c.body.as_ref())
                    .map(Self::extract_text)
                    .collect::<Vec<_>>()
                    .join("\n\n")
            })
            .unwrap_or_default();

        let project_key = fields
            .project
            .as_ref()
            .and_then(|p| p.key.clone())
            .unwrap_or_default();

        let status = fields
            .status
            .as_ref()
            .and_then(|s| s.name.as_ref())
            .cloned()
            .unwrap_or_default();

        let status_category = fields
            .status
            .as_ref()
            .and_then(|s| s.status_category.as_ref())
            .and_then(|sc| sc.name.as_ref())
            .cloned()
            .unwrap_or_default();

        let priority = fields
            .priority
            .as_ref()
            .and_then(|p| p.name.as_ref())
            .cloned()
            .unwrap_or_default();

        let issue_type = fields
            .issuetype
            .as_ref()
            .and_then(|t| t.name.as_ref())
            .cloned()
            .unwrap_or_default();

        let assignee = fields
            .assignee
            .as_ref()
            .and_then(|a| a.display_name.as_ref())
            .cloned()
            .unwrap_or_default();

        let reporter = fields
            .reporter
            .as_ref()
            .and_then(|r| r.display_name.as_ref())
            .cloned()
            .unwrap_or_default();

        let labels = fields.labels.clone().unwrap_or_default();

        // Build labeled content for embedding — provides context for semantic search
        let mut content_parts = Vec::new();
        content_parts.push(format!("[{issue_key}] {summary}", issue_key = issue.key));
        content_parts.push(format!("Type: {issue_type}"));
        content_parts.push(format!("Project: {project_key}"));
        content_parts.push(format!("Status: {status} ({status_category})"));
        content_parts.push(format!("Priority: {priority}"));
        content_parts.push(format!("Assignee: {assignee}"));
        content_parts.push(format!("Reporter: {reporter}"));
        if !labels.is_empty() {
            content_parts.push(format!("Labels: {}", labels.join(", ")));
        }
        if !description.is_empty() {
            content_parts.push(format!("\nDescription:\n{description}"));
        }
        if !comments_text.is_empty() {
            content_parts.push(format!("\nComments:\n{comments_text}"));
        }
        let content = content_parts.join("\n");

        let updated_at = fields
            .updated
            .as_ref()
            .and_then(|s| Self::parse_datetime(s))
            .unwrap_or_else(Utc::now);

        let created_at = fields
            .created
            .as_ref()
            .and_then(|s| Self::parse_datetime(s))
            .unwrap_or_else(Utc::now);

        let doc_id = format!("{}-{}", self.config.name, issue.key);

        let mut map = HashMap::new();
        map.insert("id".to_string(), serde_json::json!(doc_id));
        map.insert(
            "url".to_string(),
            serde_json::json!(self.browse_url(&issue.key)),
        );
        map.insert(
            "source_name".to_string(),
            serde_json::json!(self.config.name),
        );
        map.insert("source_type".to_string(), serde_json::json!("jira"));
        map.insert("project".to_string(), serde_json::json!(project_key));
        map.insert("issue_key".to_string(), serde_json::json!(issue.key));
        map.insert("issue_type".to_string(), serde_json::json!(issue_type));
        map.insert("summary".to_string(), serde_json::json!(summary));
        map.insert("description".to_string(), serde_json::json!(description));
        map.insert("status".to_string(), serde_json::json!(status));
        map.insert(
            "status_category".to_string(),
            serde_json::json!(status_category),
        );
        map.insert("priority".to_string(), serde_json::json!(priority));
        map.insert("assignee".to_string(), serde_json::json!(assignee));
        map.insert("reporter".to_string(), serde_json::json!(reporter));
        map.insert("labels".to_string(), serde_json::json!(labels));
        map.insert("comments".to_string(), serde_json::json!(comments_text));
        map.insert("content".to_string(), serde_json::json!(content));
        map.insert(
            "created_at".to_string(),
            serde_json::json!(created_at.to_rfc3339()),
        );
        map.insert(
            "updated_at".to_string(),
            serde_json::json!(updated_at.to_rfc3339()),
        );

        SourceDocument {
            id: doc_id,
            fields: map,
            updated_at,
        }
    }

    // -----------------------------------------------------------------------
    // Data Center fetch (REST API v2: offset-based pagination)
    // -----------------------------------------------------------------------

    async fn fetch_changes_dc(
        &self,
        cursor: Option<&SyncCursor>,
        batch_size: usize,
    ) -> Result<FetchResult> {
        let jql = self.build_jql(cursor);
        let url = format!(
            "{}/rest/api/2/search",
            self.config.url.trim_end_matches('/')
        );
        let auth_header = self.config.auth.authorization_header();

        debug!(
            source = self.config.name,
            jql = jql,
            "Fetching Jira issues (DC v2)"
        );

        let resp = self
            .client
            .get(&url)
            .header("Authorization", &auth_header)
            .query(&[
                ("jql", jql.as_str()),
                ("maxResults", &batch_size.to_string()),
                ("startAt", "0"),
                ("fields", "summary,description,status,priority,assignee,reporter,issuetype,labels,created,updated,comment,project"),
            ])
            .send()
            .await
            .context("failed to connect to Jira")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Jira API error ({}): {}", status, body);
        }

        let search_resp: DcSearchResponse = resp
            .json()
            .await
            .context("failed to parse Jira DC response")?;

        let documents: Vec<SourceDocument> = search_resp
            .issues
            .iter()
            .map(|issue| self.issue_to_document(issue))
            .collect();

        let new_cursor = documents
            .last()
            .map(|doc| SyncCursor {
                last_updated: doc.updated_at,
            })
            .or_else(|| cursor.cloned())
            .unwrap_or(SyncCursor {
                last_updated: Utc::now(),
            });

        let fetched = search_resp.start_at + search_resp.issues.len() as u64;
        let has_more = fetched < search_resp.total;

        debug!(
            source = self.config.name,
            count = documents.len(),
            total = search_resp.total,
            has_more = has_more,
            "Fetched Jira issues (DC)"
        );

        Ok(FetchResult {
            documents,
            cursor: new_cursor,
            has_more,
        })
    }

    // -----------------------------------------------------------------------
    // Cloud fetch (REST API v3: /rest/api/3/search/jql, cursor-based)
    // -----------------------------------------------------------------------

    async fn fetch_changes_cloud(
        &self,
        cursor: Option<&SyncCursor>,
        batch_size: usize,
    ) -> Result<FetchResult> {
        let jql = self.build_jql(cursor);
        let url = format!(
            "{}/rest/api/3/search/jql",
            self.config.url.trim_end_matches('/')
        );
        let auth_header = self.config.auth.authorization_header();

        debug!(
            source = self.config.name,
            jql = jql,
            "Fetching Jira issues (Cloud v3)"
        );

        let resp = self
            .client
            .get(&url)
            .header("Authorization", &auth_header)
            .query(&[
                ("jql", jql.as_str()),
                ("maxResults", &batch_size.to_string()),
                ("fields", "summary,description,status,priority,assignee,reporter,issuetype,labels,created,updated,comment,project"),
            ])
            .send()
            .await
            .context("failed to connect to Jira Cloud")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Jira Cloud API error ({}): {}", status, body);
        }

        let search_resp: CloudSearchResponse = resp
            .json()
            .await
            .context("failed to parse Jira Cloud response")?;

        let documents: Vec<SourceDocument> = search_resp
            .issues
            .iter()
            .map(|issue| self.issue_to_document(issue))
            .collect();

        let new_cursor = documents
            .last()
            .map(|doc| SyncCursor {
                last_updated: doc.updated_at,
            })
            .or_else(|| cursor.cloned())
            .unwrap_or(SyncCursor {
                last_updated: Utc::now(),
            });

        let has_more = !search_resp.is_last && search_resp.next_page_token.is_some();

        debug!(
            source = self.config.name,
            count = documents.len(),
            has_more = has_more,
            "Fetched Jira issues (Cloud)"
        );

        Ok(FetchResult {
            documents,
            cursor: new_cursor,
            has_more,
        })
    }

    // -----------------------------------------------------------------------
    // fetch_all_ids
    // -----------------------------------------------------------------------

    async fn fetch_all_ids_dc(&self) -> Result<Vec<String>> {
        let mut all_ids = Vec::new();
        let mut start_at: u64 = 0;
        let page_size: usize = 1000;
        let auth_header = self.config.auth.authorization_header();
        let jql = self.projects_jql();

        loop {
            let url = format!(
                "{}/rest/api/2/search",
                self.config.url.trim_end_matches('/')
            );

            let resp = self
                .client
                .get(&url)
                .header("Authorization", &auth_header)
                .query(&[
                    ("jql", jql.as_str()),
                    ("maxResults", &page_size.to_string()),
                    ("startAt", &start_at.to_string()),
                    ("fields", "key"),
                ])
                .send()
                .await
                .context("failed to connect to Jira for ID fetch")?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("Jira API error ({}): {}", status, body);
            }

            let search_resp: DcSearchResponse = resp.json().await?;
            let batch_len = search_resp.issues.len();

            for issue in &search_resp.issues {
                all_ids.push(format!("{}-{}", self.config.name, issue.key));
            }

            if batch_len == 0 || (start_at + batch_len as u64) >= search_resp.total {
                break;
            }
            start_at += batch_len as u64;
        }

        Ok(all_ids)
    }

    async fn fetch_all_ids_cloud(&self) -> Result<Vec<String>> {
        let mut all_ids = Vec::new();
        let auth_header = self.config.auth.authorization_header();
        let jql = self.projects_jql();
        let mut next_page_token: Option<String> = None;

        loop {
            let url = format!(
                "{}/rest/api/3/search/jql",
                self.config.url.trim_end_matches('/')
            );

            let page_size = "100"; // Cloud caps at 100
            let mut query: Vec<(&str, &str)> = vec![
                ("jql", jql.as_str()),
                ("maxResults", page_size),
                ("fields", "key"),
            ];

            let token_string;
            if let Some(ref token) = next_page_token {
                token_string = token.clone();
                query.push(("nextPageToken", &token_string));
            }

            let resp = self
                .client
                .get(&url)
                .header("Authorization", &auth_header)
                .query(&query)
                .send()
                .await
                .context("failed to connect to Jira Cloud for ID fetch")?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("Jira Cloud API error ({}): {}", status, body);
            }

            let search_resp: CloudSearchResponse = resp.json().await?;

            for issue in &search_resp.issues {
                all_ids.push(format!("{}-{}", self.config.name, issue.key));
            }

            if search_resp.is_last || search_resp.next_page_token.is_none() {
                break;
            }
            next_page_token = search_resp.next_page_token;
        }

        Ok(all_ids)
    }

    fn projects_jql(&self) -> String {
        let project_clause = self
            .config
            .projects
            .iter()
            .map(|p| format!("project = {p}"))
            .collect::<Vec<_>>()
            .join(" OR ");

        if self.config.projects.len() > 1 {
            format!("({project_clause})")
        } else {
            project_clause
        }
    }
}

impl SourceConnector for JiraConnector {
    fn source_type(&self) -> &str {
        "jira"
    }

    fn source_name(&self) -> &str {
        &self.config.name
    }

    fn index_name(&self) -> &str {
        &self.config.index
    }

    async fn fetch_changes(
        &self,
        cursor: Option<&SyncCursor>,
        batch_size: usize,
    ) -> Result<FetchResult> {
        if self.is_cloud {
            self.fetch_changes_cloud(cursor, batch_size).await
        } else {
            self.fetch_changes_dc(cursor, batch_size).await
        }
    }

    async fn fetch_all_ids(&self) -> Result<Vec<String>> {
        if self.is_cloud {
            self.fetch_all_ids_cloud().await
        } else {
            self.fetch_all_ids_dc().await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthConfig;

    fn dc_config() -> JiraSourceConfig {
        JiraSourceConfig {
            name: "test-jira".to_string(),
            url: "https://jira.example.com".to_string(),
            auth: AuthConfig::DataCenter {
                pat: "fake-pat".to_string(),
            },
            projects: vec!["DO".to_string()],
            index: "jira-issues".to_string(),
        }
    }

    fn cloud_config() -> JiraSourceConfig {
        JiraSourceConfig {
            name: "test-cloud".to_string(),
            url: "https://mycompany.atlassian.net".to_string(),
            auth: AuthConfig::Cloud {
                email: "user@example.com".to_string(),
                api_token: "token".to_string(),
            },
            projects: vec!["PROJ".to_string()],
            index: "jira-issues".to_string(),
        }
    }

    #[test]
    fn detects_cloud_vs_dc() {
        let dc = JiraConnector::new(&dc_config());
        assert!(!dc.is_cloud);

        let cloud = JiraConnector::new(&cloud_config());
        assert!(cloud.is_cloud);
    }

    #[test]
    fn builds_jql_without_cursor() {
        let connector = JiraConnector::new(&dc_config());
        let jql = connector.build_jql(None);
        assert_eq!(jql, "project = DO ORDER BY updated ASC");
    }

    #[test]
    fn builds_jql_with_cursor() {
        let connector = JiraConnector::new(&dc_config());
        let cursor = SyncCursor {
            last_updated: DateTime::parse_from_rfc3339("2025-01-15T10:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
        };
        let jql = connector.build_jql(Some(&cursor));
        assert!(jql.contains("updated >= \"2025-01-15 10:30\""));
        assert!(jql.contains("project = DO"));
    }

    #[test]
    fn builds_jql_multiple_projects() {
        let mut config = dc_config();
        config.projects = vec!["DO".to_string(), "HR".to_string()];
        let connector = JiraConnector::new(&config);
        let jql = connector.build_jql(None);
        assert_eq!(jql, "(project = DO OR project = HR) ORDER BY updated ASC");
    }

    #[test]
    fn browse_url_dc() {
        let connector = JiraConnector::new(&dc_config());
        assert_eq!(
            connector.browse_url("DO-42"),
            "https://jira.example.com/browse/DO-42"
        );
    }

    #[test]
    fn browse_url_cloud() {
        let connector = JiraConnector::new(&cloud_config());
        assert_eq!(
            connector.browse_url("PROJ-1"),
            "https://mycompany.atlassian.net/browse/PROJ-1"
        );
    }

    #[test]
    fn parses_jira_datetime() {
        let dt = JiraConnector::parse_datetime("2025-01-15T10:30:00.000+0000").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-15T10:30:00+00:00");
    }

    #[test]
    fn parses_jira_datetime_with_offset() {
        use chrono::Timelike;
        let dt = JiraConnector::parse_datetime("2024-03-15T09:12:44.000+1100").unwrap();
        assert_eq!(dt.hour(), 22);
    }

    #[test]
    fn extract_text_from_plain_string() {
        let val = serde_json::json!("Hello world");
        assert_eq!(JiraConnector::extract_text(&val), "Hello world");
    }

    #[test]
    fn extract_text_from_adf() {
        let adf = serde_json::json!({
            "version": 1,
            "type": "doc",
            "content": [
                {
                    "type": "paragraph",
                    "content": [
                        { "type": "text", "text": "Hello " },
                        { "type": "text", "text": "world" }
                    ]
                },
                {
                    "type": "paragraph",
                    "content": [
                        { "type": "text", "text": "Second paragraph" }
                    ]
                }
            ]
        });
        let text = JiraConnector::extract_text(&adf);
        assert!(text.contains("Hello world"));
        assert!(text.contains("Second paragraph"));
    }

    #[test]
    fn extract_text_from_null() {
        let val = serde_json::Value::Null;
        assert_eq!(JiraConnector::extract_text(&val), "");
    }

    #[test]
    fn converts_dc_issue_to_document() {
        let connector = JiraConnector::new(&dc_config());
        let issue = JiraIssue {
            key: "DO-42".to_string(),
            fields: JiraFields {
                summary: Some("Fix the bug".to_string()),
                description: Some(serde_json::json!("It's broken")),
                status: Some(JiraStatus {
                    name: Some("Open".to_string()),
                    status_category: Some(JiraStatusCategory {
                        name: Some("To Do".to_string()),
                    }),
                }),
                priority: Some(JiraNamedField {
                    name: Some("High".to_string()),
                }),
                assignee: Some(JiraUser {
                    display_name: Some("Alice".to_string()),
                }),
                reporter: Some(JiraUser {
                    display_name: Some("Bob".to_string()),
                }),
                issuetype: Some(JiraNamedField {
                    name: Some("Bug".to_string()),
                }),
                labels: Some(vec!["backend".to_string()]),
                created: Some("2025-01-10T08:00:00.000+0000".to_string()),
                updated: Some("2025-01-15T10:30:00.000+0000".to_string()),
                comment: Some(JiraCommentContainer {
                    comments: Some(vec![JiraComment {
                        body: Some(serde_json::json!("Looking into it")),
                    }]),
                }),
                project: Some(JiraProject {
                    key: Some("DO".to_string()),
                }),
            },
        };

        let doc = connector.issue_to_document(&issue);
        assert_eq!(doc.id, "test-jira-DO-42");
        assert_eq!(doc.fields["url"], "https://jira.example.com/browse/DO-42");
        assert_eq!(doc.fields["source_type"], "jira");
        assert_eq!(doc.fields["issue_key"], "DO-42");
        assert_eq!(doc.fields["status"], "Open");
        assert_eq!(doc.fields["status_category"], "To Do");
        assert_eq!(doc.fields["assignee"], "Alice");
        assert!(
            doc.fields["content"]
                .as_str()
                .unwrap()
                .contains("Fix the bug")
        );
        assert!(
            doc.fields["content"]
                .as_str()
                .unwrap()
                .contains("Looking into it")
        );
    }

    #[test]
    fn converts_cloud_adf_issue_to_document() {
        let connector = JiraConnector::new(&cloud_config());
        let issue = JiraIssue {
            key: "PROJ-7".to_string(),
            fields: JiraFields {
                summary: Some("Cloud issue".to_string()),
                description: Some(serde_json::json!({
                    "version": 1,
                    "type": "doc",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "ADF description text" }]
                    }]
                })),
                status: Some(JiraStatus {
                    name: Some("Done".to_string()),
                    status_category: Some(JiraStatusCategory {
                        name: Some("Done".to_string()),
                    }),
                }),
                priority: None,
                assignee: None,
                reporter: None,
                issuetype: None,
                labels: None,
                created: Some("2025-03-01T12:00:00.000+0000".to_string()),
                updated: Some("2025-03-02T14:00:00.000+0000".to_string()),
                comment: Some(JiraCommentContainer {
                    comments: Some(vec![JiraComment {
                        body: Some(serde_json::json!({
                            "version": 1,
                            "type": "doc",
                            "content": [{
                                "type": "paragraph",
                                "content": [{ "type": "text", "text": "ADF comment" }]
                            }]
                        })),
                    }]),
                }),
                project: Some(JiraProject {
                    key: Some("PROJ".to_string()),
                }),
            },
        };

        let doc = connector.issue_to_document(&issue);
        assert_eq!(doc.id, "test-cloud-PROJ-7");
        assert_eq!(
            doc.fields["url"],
            "https://mycompany.atlassian.net/browse/PROJ-7"
        );
        let content = doc.fields["content"].as_str().unwrap();
        assert!(content.contains("ADF description text"));
        assert!(content.contains("ADF comment"));
    }

    #[test]
    fn handles_null_fields_gracefully() {
        let connector = JiraConnector::new(&dc_config());
        let issue = JiraIssue {
            key: "DO-1".to_string(),
            fields: JiraFields {
                summary: None,
                description: None,
                status: None,
                priority: None,
                assignee: None,
                reporter: None,
                issuetype: None,
                labels: None,
                created: None,
                updated: None,
                comment: None,
                project: None,
            },
        };

        let doc = connector.issue_to_document(&issue);
        assert_eq!(doc.id, "test-jira-DO-1");
        assert_eq!(doc.fields["status"], "");
        assert_eq!(doc.fields["assignee"], "");
        assert_eq!(doc.fields["url"], "https://jira.example.com/browse/DO-1");
    }
}
