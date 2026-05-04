//! Jira source connector — v2 implementation.
//!
//! Implements the [`SourceConnector`] trait for both Jira Cloud and Data Center.
//! Uses the Jira REST API v2 (`/rest/api/2/`) for issues and Jira Agile API
//! (`/rest/agile/1.0/`) for sprint data.
//!
//! See `docs/sync.md` for the sync algorithm and JQL format requirements.
//! See `docs/architecture.md` "Jira issue (`jira_issues`)" for the canonical document shape.

use std::collections::HashMap;

use anyhow::{Context, anyhow};
use chrono::{DateTime, Utc};
use reqwest_middleware::ClientWithMiddleware;
use serde_json::{Value, json};
use tracing::debug;

use super::{BackfillCheckpoint, Companions, FetchPage, SourceConnector, SourceDocument};
use crate::config::JiraSourceConfig;

// ---------------------------------------------------------------------------
// Connector struct
// ---------------------------------------------------------------------------

/// Jira source connector.
///
/// Implements [`SourceConnector`] for both Jira Cloud and Data Center.
/// Constructed via [`JiraConnector::new`]; the HTTP client is injected
/// (built once by the ingest worker with rate-limit middleware).
#[derive(Clone)]
pub struct JiraConnector {
    /// Source name from config — used as the stable identifier in Cosmos IDs.
    source_name: String,
    /// Base URL, e.g. `https://example.atlassian.net` or `https://jira.internal.example`.
    base_url: String,
    /// `Authorization` header value, computed once from config at construction.
    auth_header: String,
    /// Project keys to ingest (subsources).
    projects: Vec<String>,
    /// Custom field mapping: friendly name → Jira field id (e.g. `story_points → customfield_10016`).
    custom_fields: HashMap<String, String>,
    /// Rate-limit-aware HTTP client (shared with other connectors / workers).
    client: ClientWithMiddleware,
    /// Primary container for issues (from config override or default).
    container: String,
}

impl JiraConnector {
    /// Create a new `JiraConnector`.
    ///
    /// # Arguments
    ///
    /// * `config` — Jira source config from `quelch.yaml`.
    /// * `client` — pre-built `reqwest_middleware::ClientWithMiddleware` (injected by worker).
    pub fn new(config: &JiraSourceConfig, client: ClientWithMiddleware) -> anyhow::Result<Self> {
        let base_url = config.url.trim_end_matches('/').to_owned();
        let auth_header = config.auth.authorization_header();
        let container = config
            .container
            .clone()
            .unwrap_or_else(|| "jira-issues".to_string());

        // Invert the fields map: friendly name → customfield id
        let custom_fields = config.fields.clone();

        Ok(Self {
            source_name: config.name.clone(),
            base_url,
            auth_header,
            projects: config.projects.clone(),
            custom_fields,
            client,
            container,
        })
    }

    // -----------------------------------------------------------------------
    // HTTP helpers
    // -----------------------------------------------------------------------

    /// Issue a POST to `/rest/api/2/search` with the given body.
    async fn search(&self, body: Value) -> anyhow::Result<Value> {
        let url = format!("{}/rest/api/2/search", self.base_url);
        let body_bytes = serde_json::to_vec(&body).context("serialize Jira search body")?;
        let resp = self
            .client
            .post(&url)
            .header("Authorization", &self.auth_header)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .body(body_bytes)
            .send()
            .await
            .context("POST /rest/api/2/search")?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Jira search returned {status}: {text}"));
        }

        let bytes = resp
            .bytes()
            .await
            .context("read Jira search response body")?;
        serde_json::from_slice(&bytes).context("deserialize Jira search response")
    }

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

    /// Execute a paginated `/rest/api/2/search` call and return a [`FetchPage`].
    ///
    /// `start_at` defaults to 0 when `page_token` is `None`.
    async fn fetch_issues_page(
        &self,
        jql: &str,
        start_at: usize,
        batch_size: usize,
    ) -> anyhow::Result<FetchPage> {
        let body = json!({
            "jql": jql,
            "startAt": start_at,
            "maxResults": batch_size,
            "fields": ["*all"],
            "expand": ["renderedFields"]
        });

        let resp = self.search(body).await?;

        let total = resp["total"].as_u64().unwrap_or(0) as usize;
        let issues = resp["issues"].as_array().cloned().unwrap_or_default();
        let issue_count = issues.len();

        let documents: Vec<SourceDocument> = issues
            .iter()
            .map(|issue| {
                parse_issue(
                    issue,
                    &self.source_name,
                    &self.base_url,
                    &self.custom_fields,
                )
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        // Determine last_seen from the last document in this page (for backfill).
        let last_seen = documents.last().and_then(|doc| {
            let key = doc.fields.get("key")?.as_str()?.to_owned();
            Some(BackfillCheckpoint {
                updated: doc.updated_at,
                key,
            })
        });

        // next_page_token: Some if there are more pages.
        // Guard against empty pages (issue_count == 0) even when total suggests more —
        // this can happen when the source returns 0 results on the last page.
        let next_start = start_at + issue_count;
        let next_page_token = if issue_count > 0 && next_start < total {
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

    // -----------------------------------------------------------------------
    // Companion helpers
    // -----------------------------------------------------------------------

    /// Fetch all sprints for `project_key` by enumerating boards, deduplicating sprint ids.
    async fn fetch_sprints_for_project(
        &self,
        project_key: &str,
    ) -> anyhow::Result<Vec<SourceDocument>> {
        // Step 1: list boards for this project
        let boards_path =
            format!("/rest/agile/1.0/board?projectKeyOrId={project_key}&maxResults=50");
        let boards_resp = match self.get(&boards_path).await {
            Ok(v) => v,
            Err(e) => {
                debug!(
                    project_key,
                    error = %e,
                    "agile /board endpoint unavailable — skipping sprints"
                );
                return Ok(vec![]);
            }
        };

        let boards = boards_resp["values"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut sprint_docs: Vec<SourceDocument> = Vec::new();
        let mut seen_sprint_ids: std::collections::HashSet<u64> = std::collections::HashSet::new();

        for board in &boards {
            let board_id = match board["id"].as_u64() {
                Some(id) => id,
                None => continue,
            };

            let mut start_at: usize = 0;
            loop {
                let sprints_path = format!(
                    "/rest/agile/1.0/board/{board_id}/sprint?startAt={start_at}&maxResults=50"
                );
                let sprints_resp = match self.get(&sprints_path).await {
                    Ok(v) => v,
                    Err(e) => {
                        debug!(
                            board_id,
                            project_key,
                            error = %e,
                            "failed to list sprints for board — skipping"
                        );
                        break;
                    }
                };

                let sprints = sprints_resp["values"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                if sprints.is_empty() {
                    break;
                }

                let fetched = sprints.len();
                let total = sprints_resp["total"].as_u64().unwrap_or(0) as usize;

                for sprint in &sprints {
                    let sprint_id = match sprint["id"].as_u64() {
                        Some(id) => id,
                        None => continue,
                    };
                    if !seen_sprint_ids.insert(sprint_id) {
                        continue; // already processed from another board
                    }

                    let doc = parse_sprint(
                        sprint,
                        &self.source_name,
                        &self.base_url,
                        project_key,
                        board_id,
                    );
                    sprint_docs.push(doc);
                }

                start_at += fetched;
                if start_at >= total {
                    break;
                }
            }
        }

        Ok(sprint_docs)
    }

    /// Fetch fix versions for `project_key`.
    async fn fetch_fix_versions_for_project(
        &self,
        project_key: &str,
    ) -> anyhow::Result<Vec<SourceDocument>> {
        let path = format!("/rest/api/2/project/{project_key}/versions");
        let resp = match self.get(&path).await {
            Ok(v) => v,
            Err(e) => {
                debug!(
                    project_key,
                    error = %e,
                    "failed to fetch fix versions — skipping"
                );
                return Ok(vec![]);
            }
        };

        let versions = match resp.as_array() {
            Some(a) => a.clone(),
            None => {
                debug!(
                    project_key,
                    "fix versions response was not an array — skipping"
                );
                return Ok(vec![]);
            }
        };

        let docs = versions
            .iter()
            .map(|v| parse_fix_version(v, &self.source_name, &self.base_url, project_key))
            .collect();

        Ok(docs)
    }

    /// Fetch project metadata for `project_key`.
    async fn fetch_project(&self, project_key: &str) -> anyhow::Result<Option<SourceDocument>> {
        let path = format!("/rest/api/2/project/{project_key}");
        let resp = match self.get(&path).await {
            Ok(v) => v,
            Err(e) => {
                debug!(
                    project_key,
                    error = %e,
                    "failed to fetch project metadata — skipping"
                );
                return Ok(None);
            }
        };

        Ok(Some(parse_project(
            &resp,
            &self.source_name,
            &self.base_url,
        )))
    }
}

// ---------------------------------------------------------------------------
// SourceConnector impl
// ---------------------------------------------------------------------------

impl SourceConnector for JiraConnector {
    fn source_type(&self) -> &str {
        "jira"
    }

    fn source_name(&self) -> &str {
        &self.source_name
    }

    fn subsources(&self) -> &[String] {
        &self.projects
    }

    fn primary_container(&self) -> &str {
        &self.container
    }

    /// Fetch a closed minute-resolution window of issues updated in `[window_start, window_end]`.
    ///
    /// JQL format per `docs/sync.md`:
    /// ```text
    /// project = "{subsource}"
    /// AND updated >= "yyyy/MM/dd HH:mm"
    /// AND updated <= "yyyy/MM/dd HH:mm"
    /// ORDER BY updated ASC, key ASC
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

        let jql = format!(
            r#"project = "{subsource}" AND updated >= "{start_str}" AND updated <= "{end_str}" ORDER BY updated ASC, key ASC"#
        );

        let start_at: usize = page_token.and_then(|t| t.parse().ok()).unwrap_or(0);

        self.fetch_issues_page(&jql, start_at, batch_size).await
    }

    /// Fetch one page of backfill, resuming after `last_seen`.
    ///
    /// JQL format per `docs/sync.md` — "Initial backfill":
    /// ```text
    /// project = "{subsource}"
    /// AND updated <= "yyyy/MM/dd HH:mm"
    /// [AND ((updated > "{last_seen.updated ISO-8601}")
    ///       OR (updated = "{last_seen.updated ISO-8601}" AND key > "{last_seen.key}"))]
    /// ORDER BY updated ASC, key ASC
    /// ```
    async fn fetch_backfill_page(
        &self,
        subsource: &str,
        backfill_target: DateTime<Utc>,
        last_seen: Option<&BackfillCheckpoint>,
        batch_size: usize,
    ) -> anyhow::Result<FetchPage> {
        let target_str = backfill_target.format("%Y/%m/%d %H:%M").to_string();

        let jql = if let Some(checkpoint) = last_seen {
            // ISO 8601 with second precision for the resume clause.
            let updated_str = checkpoint
                .updated
                .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                .to_string();
            let key = &checkpoint.key;
            format!(
                r#"project = "{subsource}" AND updated <= "{target_str}" AND ((updated > "{updated_str}") OR (updated = "{updated_str}" AND key > "{key}")) ORDER BY updated ASC, key ASC"#
            )
        } else {
            format!(
                r#"project = "{subsource}" AND updated <= "{target_str}" ORDER BY updated ASC, key ASC"#
            )
        };

        self.fetch_issues_page(&jql, 0, batch_size).await
    }

    /// List all issue keys for `subsource`, returning them as `"{source_name}-{key}"`.
    ///
    /// Uses `fields=["key"]` only for efficiency.
    async fn list_all_ids(&self, subsource: &str) -> anyhow::Result<Vec<String>> {
        let jql = format!(r#"project = "{subsource}" ORDER BY key ASC"#);
        let mut all_ids: Vec<String> = Vec::new();
        let mut start_at: usize = 0;
        let batch_size: usize = 100;

        loop {
            let body = json!({
                "jql": jql,
                "startAt": start_at,
                "maxResults": batch_size,
                "fields": ["key"]
            });

            let resp = self.search(body).await?;
            let total = resp["total"].as_u64().unwrap_or(0) as usize;
            let issues = resp["issues"].as_array().cloned().unwrap_or_default();
            let count = issues.len();

            for issue in &issues {
                if let Some(key) = issue["key"].as_str() {
                    all_ids.push(format!("{}-{}", self.source_name, key));
                }
            }

            start_at += count;
            if start_at >= total || count == 0 {
                break;
            }
        }

        Ok(all_ids)
    }

    /// Fetch companion documents (sprints, fix versions, project metadata) for `subsource`.
    ///
    /// On 404 from the agile or versions endpoints (e.g. server install without Agile),
    /// logs at `debug!` and returns empty for that companion type — does not fail.
    async fn fetch_companions(&self, subsource: &str) -> anyhow::Result<Companions> {
        let sprints = self.fetch_sprints_for_project(subsource).await?;
        let fix_versions = self.fetch_fix_versions_for_project(subsource).await?;
        let projects = match self.fetch_project(subsource).await? {
            Some(doc) => vec![doc],
            None => vec![],
        };

        Ok(Companions {
            sprints,
            fix_versions,
            projects,
            spaces: vec![],
        })
    }
}

// ---------------------------------------------------------------------------
// Date/time helpers
// ---------------------------------------------------------------------------

/// Parse a Jira timestamp string to `DateTime<Utc>`.
///
/// Jira returns timestamps in two common formats:
/// - With milliseconds and no-colon offset: `"2026-04-28T14:02:11.000+0000"`
/// - With colon offset (RFC3339): `"2026-04-28T14:02:11.000+00:00"` or `"2026-04-28T14:02:11Z"`
///
/// We try all three formats in order.
fn parse_jira_datetime(s: &str) -> anyhow::Result<DateTime<Utc>> {
    // 1. Standard RFC3339 / ISO 8601 with colon in offset
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    // 2. Jira's "+0000" format with milliseconds
    if let Ok(dt) = DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.3f%z") {
        return Ok(dt.with_timezone(&Utc));
    }
    // 3. Without milliseconds
    if let Ok(dt) = DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%z") {
        return Ok(dt.with_timezone(&Utc));
    }
    Err(anyhow!("cannot parse Jira datetime: {s:?}"))
}

// ---------------------------------------------------------------------------
// Document parsers
// ---------------------------------------------------------------------------

/// Map a raw Jira issue JSON value to a [`SourceDocument`].
///
/// Covers every canonical field from `docs/architecture.md` "Jira issue (`jira_issues`)".
pub fn parse_issue(
    issue: &Value,
    source_name: &str,
    base_url: &str,
    custom_fields: &HashMap<String, String>,
) -> anyhow::Result<SourceDocument> {
    let key = issue["key"]
        .as_str()
        .ok_or_else(|| anyhow!("Jira issue missing 'key' field"))?;

    let fields = &issue["fields"];
    let rendered = &issue["renderedFields"];

    // updated_at: parse from fields.updated
    // Jira returns timestamps as e.g. "2026-04-28T14:02:11.000+0000" (no colon in offset),
    // which is not strict RFC3339. We try both with-ms and without-ms variants.
    let updated_str = fields["updated"]
        .as_str()
        .ok_or_else(|| anyhow!("Jira issue {key} missing fields.updated"))?;
    let updated_at =
        parse_jira_datetime(updated_str).with_context(|| format!("parse updated for {key}"))?;

    // project_key: from fields.project.key
    let project_key = fields["project"]["key"]
        .as_str()
        .unwrap_or_else(|| {
            // Fallback: extract project key from the issue key (everything before the last `-\d+`)
            key.rsplit_once('-').map(|(proj, _)| proj).unwrap_or("")
        })
        .to_owned();

    let id = format!("{source_name}-{key}");
    let source_link = format!("{base_url}/browse/{key}");

    // --- Build fields map ---
    let mut map: HashMap<String, Value> = HashMap::new();

    map.insert("key".into(), json!(key));
    map.insert("project_key".into(), json!(&project_key));
    map.insert("source_name".into(), json!(source_name));
    map.insert("source_link".into(), json!(&source_link));

    // type
    map.insert(
        "type".into(),
        json!(fields["issuetype"]["name"].as_str().unwrap_or("")),
    );

    // status / status_category
    map.insert(
        "status".into(),
        json!(fields["status"]["name"].as_str().unwrap_or("")),
    );
    map.insert(
        "status_category".into(),
        json!(
            fields["status"]["statusCategory"]["name"]
                .as_str()
                .unwrap_or("")
        ),
    );

    // priority
    map.insert(
        "priority".into(),
        json!(fields["priority"]["name"].as_str()),
    );

    // resolution / resolved
    map.insert(
        "resolution".into(),
        json!(fields["resolution"]["name"].as_str()),
    );
    map.insert("resolved".into(), json!(fields["resolutiondate"].as_str()));

    // summary
    map.insert(
        "summary".into(),
        json!(fields["summary"].as_str().unwrap_or("")),
    );

    // description: prefer renderedFields.description (HTML), fall back to fields.description (ADF or plain)
    let description = if rendered["description"].is_string() {
        rendered["description"].clone()
    } else if !fields["description"].is_null() {
        fields["description"].clone()
    } else {
        Value::Null
    };
    map.insert("description".into(), description);

    // assignee / reporter
    map.insert("assignee".into(), parse_user(&fields["assignee"]));
    map.insert("reporter".into(), parse_user(&fields["reporter"]));

    // timestamps
    map.insert("created".into(), json!(fields["created"].as_str()));
    map.insert("updated".into(), json!(fields["updated"].as_str()));
    map.insert("due_date".into(), json!(fields["duedate"].as_str()));

    // labels
    map.insert(
        "labels".into(),
        json!(fields["labels"].as_array().cloned().unwrap_or_default()),
    );

    // components
    let components: Vec<Value> = fields["components"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|c| json!(c["name"].as_str().unwrap_or("")))
        .collect();
    map.insert("components".into(), json!(components));

    // fix_versions
    let fix_versions: Vec<Value> = fields["fixVersions"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|v| json!({ "id": v["id"].as_str(), "name": v["name"].as_str() }))
        .collect();
    map.insert("fix_versions".into(), json!(fix_versions));

    // affects_versions
    let affects_versions: Vec<Value> = fields["versions"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|v| json!({ "id": v["id"].as_str(), "name": v["name"].as_str() }))
        .collect();
    map.insert("affects_versions".into(), json!(affects_versions));

    // sprint: historically customfield_10020; take the most recent active or future one.
    // Sprint field may be a direct object (Data Center) or array of objects.
    let sprint = extract_sprint(fields);
    map.insert("sprint".into(), sprint);

    // parent
    let parent = if !fields["parent"].is_null() && fields["parent"].is_object() {
        json!({
            "id": fields["parent"]["id"].as_str(),
            "key": fields["parent"]["key"].as_str(),
            "type": fields["parent"]["fields"]["issuetype"]["name"].as_str()
        })
    } else {
        Value::Null
    };
    map.insert("parent".into(), parent);

    // epic_link: legacy customfield_10014
    map.insert(
        "epic_link".into(),
        json!(fields["customfield_10014"].as_str()),
    );

    // issuelinks
    let issuelinks: Vec<Value> = fields["issuelinks"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(parse_issuelink)
        .collect();
    map.insert("issuelinks".into(), json!(issuelinks));

    // comments
    let comments: Vec<Value> = fields["comment"]["comments"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(parse_comment)
        .collect();
    map.insert("comments".into(), json!(comments));

    // custom fields: friendly name → value from issue's customfield_XXXXX
    for (friendly_name, customfield_id) in custom_fields {
        let value = fields[customfield_id.as_str()].clone();
        map.insert(friendly_name.clone(), value);
    }

    // Quelch internals
    map.insert("_partition_key".into(), json!(&project_key));
    map.insert("_deleted".into(), json!(false));
    map.insert("_deleted_at".into(), Value::Null);

    Ok(SourceDocument {
        id,
        partition_key: project_key,
        fields: map,
        updated_at,
        source_link,
    })
}

/// Extract the active or future sprint from the issue's sprint fields.
///
/// Jira historically uses `customfield_10020` for sprints. It can be an array
/// of sprint objects (Cloud) or a raw object (DC). We take the most recent
/// active/future sprint, or the last closed sprint if none is active.
fn extract_sprint(fields: &Value) -> Value {
    // Try customfield_10020 first (most installations)
    let raw = &fields["customfield_10020"];

    let sprints: Vec<&Value> = if let Some(arr) = raw.as_array() {
        arr.iter().collect()
    } else if raw.is_object() {
        vec![raw]
    } else {
        return Value::Null;
    };

    if sprints.is_empty() {
        return Value::Null;
    }

    // Priority: active > future > closed (last)
    let best = sprints.iter().min_by_key(|s: &&&Value| -> u8 {
        match s["state"].as_str() {
            Some("active") => 0,
            Some("future") => 1,
            _ => 2,
        }
    });
    match best {
        Some(s) => json!({
            "id": s["id"],
            "name": s["name"].as_str(),
            "state": s["state"].as_str(),
            "start_date": s["startDate"].as_str(),
            "end_date": s["endDate"].as_str(),
            "goal": s["goal"].as_str()
        }),
        None => Value::Null,
    }
}

/// Map a Jira user object to `{ id, name, email }`.
fn parse_user(user: &Value) -> Value {
    if user.is_null() {
        return Value::Null;
    }
    json!({
        "id": user["accountId"].as_str().or_else(|| user["name"].as_str()),
        "name": user["displayName"].as_str(),
        "email": user["emailAddress"].as_str()
    })
}

/// Map a Jira `issuelinks` entry to the canonical shape.
fn parse_issuelink(link: &Value) -> Value {
    let link_type = &link["type"];

    if let Some(inward_issue) = link["inwardIssue"].as_object() {
        let target_key = inward_issue
            .get("key")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let target_summary = inward_issue
            .get("fields")
            .and_then(|f| f["summary"].as_str())
            .unwrap_or("");
        json!({
            "type": link_type["inward"].as_str().unwrap_or(""),
            "direction": "inward",
            "target_key": target_key,
            "target_summary": target_summary
        })
    } else if let Some(outward_issue) = link["outwardIssue"].as_object() {
        let target_key = outward_issue
            .get("key")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let target_summary = outward_issue
            .get("fields")
            .and_then(|f| f["summary"].as_str())
            .unwrap_or("");
        json!({
            "type": link_type["outward"].as_str().unwrap_or(""),
            "direction": "outward",
            "target_key": target_key,
            "target_summary": target_summary
        })
    } else {
        json!({
            "type": "",
            "direction": "unknown",
            "target_key": "",
            "target_summary": ""
        })
    }
}

/// Map a Jira comment entry to the canonical shape.
fn parse_comment(comment: &Value) -> Value {
    json!({
        "id": comment["id"].as_str(),
        "author": parse_user(&comment["author"]),
        "body": comment["body"].as_str().unwrap_or(""),
        "created": comment["created"].as_str(),
        "updated": comment["updated"].as_str()
    })
}

/// Map a Jira Agile sprint object to a [`SourceDocument`] for the `jira-sprints` container.
fn parse_sprint(
    sprint: &Value,
    source_name: &str,
    base_url: &str,
    project_key: &str,
    board_id: u64,
) -> SourceDocument {
    let sprint_id = sprint["id"]
        .as_u64()
        .map(|n| n.to_string())
        .unwrap_or_default();

    let id = format!("{source_name}-sprint-{sprint_id}");
    let source_link = format!("{base_url}/rest/agile/1.0/sprint/{sprint_id}");
    let partition_key = project_key.to_owned();

    let now = Utc::now().to_rfc3339();
    let updated_at = sprint["endDate"]
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let mut fields: HashMap<String, Value> = HashMap::new();
    fields.insert("id".into(), json!(&id));
    fields.insert("source_name".into(), json!(source_name));
    fields.insert("source_link".into(), json!(&source_link));
    fields.insert("key".into(), json!(&sprint_id));
    fields.insert("name".into(), json!(sprint["name"].as_str().unwrap_or("")));
    fields.insert(
        "state".into(),
        json!(sprint["state"].as_str().unwrap_or("")),
    );
    fields.insert("start_date".into(), json!(sprint["startDate"].as_str()));
    fields.insert("end_date".into(), json!(sprint["endDate"].as_str()));
    fields.insert(
        "complete_date".into(),
        json!(sprint["completeDate"].as_str()),
    );
    fields.insert("goal".into(), json!(sprint["goal"].as_str()));
    fields.insert("project_keys".into(), json!([project_key]));
    fields.insert("board_id".into(), json!(board_id.to_string()));
    fields.insert("created".into(), json!(&now));
    fields.insert("updated".into(), json!(&now));
    fields.insert("_partition_key".into(), json!(project_key));
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

/// Map a Jira version object to a [`SourceDocument`] for the `jira-fix-versions` container.
fn parse_fix_version(
    version: &Value,
    source_name: &str,
    base_url: &str,
    project_key: &str,
) -> SourceDocument {
    let version_id = version["id"].as_str().unwrap_or("").to_owned();
    let version_name = version["name"].as_str().unwrap_or("").to_owned();

    let id = format!("{source_name}-fixversion-{version_id}");
    let source_link = format!("{base_url}/rest/api/2/version/{version_id}");
    let partition_key = project_key.to_owned();

    let now = Utc::now().to_rfc3339();
    let updated_at = version["releaseDate"]
        .as_str()
        .and_then(|s| {
            DateTime::parse_from_str(&format!("{s}T00:00:00Z"), "%Y-%m-%dT%H:%M:%SZ").ok()
        })
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let mut fields: HashMap<String, Value> = HashMap::new();
    fields.insert("id".into(), json!(&id));
    fields.insert("source_name".into(), json!(source_name));
    fields.insert("source_link".into(), json!(&source_link));
    fields.insert("name".into(), json!(&version_name));
    fields.insert("description".into(), json!(version["description"].as_str()));
    fields.insert(
        "released".into(),
        json!(version["released"].as_bool().unwrap_or(false)),
    );
    fields.insert(
        "release_date".into(),
        json!(version["releaseDate"].as_str()),
    );
    fields.insert(
        "archived".into(),
        json!(version["archived"].as_bool().unwrap_or(false)),
    );
    fields.insert("project_key".into(), json!(project_key));
    fields.insert("created".into(), json!(&now));
    fields.insert("updated".into(), json!(&now));
    fields.insert("_partition_key".into(), json!(project_key));
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

/// Map a Jira project object to a [`SourceDocument`] for the `jira-projects` container.
fn parse_project(project: &Value, source_name: &str, base_url: &str) -> SourceDocument {
    let key = project["key"].as_str().unwrap_or("").to_owned();
    let id = format!("{source_name}-{key}");
    let source_link = format!("{base_url}/projects/{key}");
    let partition_key = key.clone();

    let now = Utc::now().to_rfc3339();
    let updated_at = Utc::now();

    let mut fields: HashMap<String, Value> = HashMap::new();
    fields.insert("id".into(), json!(&id));
    fields.insert("source_name".into(), json!(source_name));
    fields.insert("source_link".into(), json!(&source_link));
    fields.insert("key".into(), json!(&key));
    fields.insert("name".into(), json!(project["name"].as_str().unwrap_or("")));
    fields.insert("description".into(), json!(project["description"].as_str()));
    fields.insert("lead".into(), parse_user(&project["lead"]));
    fields.insert(
        "project_type_key".into(),
        json!(project["projectTypeKey"].as_str()),
    );
    fields.insert(
        "category".into(),
        if project["projectCategory"].is_object() {
            json!({
                "id": project["projectCategory"]["id"].as_str(),
                "name": project["projectCategory"]["name"].as_str()
            })
        } else {
            Value::Null
        },
    );
    fields.insert("created".into(), json!(&now));
    fields.insert("updated".into(), json!(&now));
    fields.insert("_partition_key".into(), json!(&key));
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use reqwest_middleware::ClientBuilder;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::config::AuthConfig;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    /// Build a [`JiraConnector`] pointing at the mock server URI.
    fn build_connector(server_uri: &str, source_name: &str, auth: AuthConfig) -> JiraConnector {
        let base_client = reqwest::Client::new();
        let client = ClientBuilder::new(base_client).build();

        let config = JiraSourceConfig {
            name: source_name.to_string(),
            url: server_uri.to_string(),
            auth,
            projects: vec!["DO".to_string()],
            container: None,
            companion_containers: Default::default(),
            fields: HashMap::new(),
        };

        JiraConnector::new(&config, client).expect("connector construction should not fail")
    }

    /// Minimal valid search response (empty result set).
    fn empty_search_response() -> Value {
        json!({
            "issues": [],
            "startAt": 0,
            "maxResults": 100,
            "total": 0
        })
    }

    /// A single full-featured Jira issue fixture exercising every canonical field.
    fn full_issue_fixture() -> Value {
        json!({
            "key": "DO-1234",
            "fields": {
                "project": { "key": "DO", "name": "DataOps" },
                "issuetype": { "name": "Story" },
                "status": {
                    "name": "In Progress",
                    "statusCategory": { "name": "In Progress" }
                },
                "priority": { "name": "High" },
                "resolution": null,
                "resolutiondate": null,
                "summary": "Camera disconnects intermittently on WiFi",
                "description": null,
                "assignee": {
                    "accountId": "user-001",
                    "displayName": "Kristofer Liljeblad",
                    "emailAddress": "kristofer@example.com"
                },
                "reporter": {
                    "accountId": "user-002",
                    "displayName": "Alice",
                    "emailAddress": "alice@example.com"
                },
                "created": "2026-04-12T10:21:00.000+0000",
                "updated": "2026-04-28T14:02:11.000+0000",
                "duedate": "2026-05-15",
                "labels": ["wifi", "regression"],
                "components": [
                    { "name": "camera" },
                    { "name": "firmware" }
                ],
                "fixVersions": [
                    { "id": "10001", "name": "iXX-2.7.0" }
                ],
                "versions": [
                    { "id": "10000", "name": "iXX-2.6.3" }
                ],
                "customfield_10020": [
                    {
                        "id": 204,
                        "name": "DO Sprint 42",
                        "state": "active",
                        "startDate": "2026-04-15T00:00:00.000Z",
                        "endDate": "2026-04-29T00:00:00.000Z",
                        "goal": "Stabilise firmware"
                    }
                ],
                "parent": {
                    "id": "10100",
                    "key": "DO-1100",
                    "fields": { "issuetype": { "name": "Epic" } }
                },
                "customfield_10014": "DO-1100",
                "issuelinks": [
                    {
                        "type": { "outward": "blocks", "inward": "is blocked by" },
                        "outwardIssue": {
                            "key": "DO-1180",
                            "fields": { "summary": "Target issue" }
                        }
                    },
                    {
                        "type": { "outward": "blocks", "inward": "is blocked by" },
                        "inwardIssue": {
                            "key": "DO-1170",
                            "fields": { "summary": "Blocking issue" }
                        }
                    }
                ],
                "comment": {
                    "comments": [
                        {
                            "id": "c1",
                            "author": {
                                "accountId": "user-003",
                                "displayName": "Bob",
                                "emailAddress": "bob@example.com"
                            },
                            "body": "Confirmed on device rev B",
                            "created": "2026-04-13T09:00:00.000+0000",
                            "updated": "2026-04-13T09:00:00.000+0000"
                        }
                    ]
                }
            },
            "renderedFields": {
                "description": "<p>Rendered HTML description</p>"
            }
        })
    }

    // -----------------------------------------------------------------------
    // Test: fetch_window emits correct JQL
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn fetch_window_emits_correct_jql() {
        let server = MockServer::start().await;

        // body_string_contains checks raw bytes — JSON serializes quotes as \" so we match
        // against the escaped form as it appears in the serialized request body.
        Mock::given(method("POST"))
            .and(path("/rest/api/2/search"))
            .and(body_string_contains(r#"updated >= \"2026/04/30 14:23\""#))
            .and(body_string_contains(r#"updated <= \"2026/04/30 14:25\""#))
            .respond_with(ResponseTemplate::new(200).set_body_json(empty_search_response()))
            .mount(&server)
            .await;

        let connector = build_connector(
            &server.uri(),
            "test",
            AuthConfig::DataCenter { pat: "x".into() },
        );

        let start: DateTime<Utc> = "2026-04-30T14:23:00Z".parse().unwrap();
        let end: DateTime<Utc> = "2026-04-30T14:25:00Z".parse().unwrap();

        let page = connector
            .fetch_window("DO", start, end, 100, None)
            .await
            .expect("fetch_window should succeed");

        assert!(page.documents.is_empty());
        assert!(page.next_page_token.is_none());
    }

    // -----------------------------------------------------------------------
    // Test: authentication header is sent on every request
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn auth_header_sent_on_every_request() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/rest/api/2/search"))
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

        // Should not panic — the mock only matches requests with the correct Authorization header.
        connector
            .fetch_window("DO", start, end, 100, None)
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

        Mock::given(method("POST"))
            .and(path("/rest/api/2/search"))
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
            .fetch_window("DO", start, end, 100, None)
            .await
            .expect("cloud auth header should be sent correctly");
    }

    // -----------------------------------------------------------------------
    // Test: parses full canonical issue
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn parses_full_canonical_issue() {
        let issue = full_issue_fixture();
        let custom_fields: HashMap<String, String> = HashMap::new();

        let doc = parse_issue(
            &issue,
            "jira-internal",
            "https://jira.example.com",
            &custom_fields,
        )
        .expect("parse_issue should succeed");

        // id follows "{source_name}-{key}" convention
        assert_eq!(doc.id, "jira-internal-DO-1234");
        assert_eq!(doc.partition_key, "DO");
        assert_eq!(doc.source_link, "https://jira.example.com/browse/DO-1234");

        let f = &doc.fields;

        // Core identity fields
        assert_eq!(f["key"].as_str().unwrap(), "DO-1234");
        assert_eq!(f["project_key"].as_str().unwrap(), "DO");
        assert_eq!(f["source_name"].as_str().unwrap(), "jira-internal");

        // Issue metadata
        assert_eq!(f["type"].as_str().unwrap(), "Story");
        assert_eq!(f["status"].as_str().unwrap(), "In Progress");
        assert_eq!(f["status_category"].as_str().unwrap(), "In Progress");
        assert_eq!(f["priority"].as_str().unwrap(), "High");
        assert!(f["resolution"].is_null());
        assert!(f["resolved"].is_null());

        // Content
        assert!(f["summary"].as_str().unwrap().contains("Camera"));
        // renderedFields.description takes priority over fields.description
        assert_eq!(
            f["description"].as_str().unwrap(),
            "<p>Rendered HTML description</p>"
        );

        // People
        assert_eq!(
            f["assignee"]["name"].as_str().unwrap(),
            "Kristofer Liljeblad"
        );
        assert_eq!(f["reporter"]["name"].as_str().unwrap(), "Alice");

        // Dates
        assert!(f["created"].as_str().unwrap().contains("2026-04-12"));
        assert!(f["updated"].as_str().unwrap().contains("2026-04-28"));
        assert_eq!(f["due_date"].as_str().unwrap(), "2026-05-15");

        // Collections
        let labels = f["labels"].as_array().unwrap();
        assert_eq!(labels.len(), 2);
        assert!(labels.iter().any(|l| l.as_str() == Some("wifi")));

        let components = f["components"].as_array().unwrap();
        assert_eq!(components.len(), 2);

        let fix_versions = f["fix_versions"].as_array().unwrap();
        assert_eq!(fix_versions.len(), 1);
        assert_eq!(fix_versions[0]["name"].as_str().unwrap(), "iXX-2.7.0");

        let affects_versions = f["affects_versions"].as_array().unwrap();
        assert_eq!(affects_versions.len(), 1);
        assert_eq!(affects_versions[0]["name"].as_str().unwrap(), "iXX-2.6.3");

        // Sprint
        let sprint = &f["sprint"];
        assert!(!sprint.is_null(), "sprint should be present");
        assert_eq!(sprint["name"].as_str().unwrap(), "DO Sprint 42");
        assert_eq!(sprint["state"].as_str().unwrap(), "active");

        // Parent / epic
        assert_eq!(f["parent"]["key"].as_str().unwrap(), "DO-1100");
        assert_eq!(f["epic_link"].as_str().unwrap(), "DO-1100");

        // Issuelinks
        let issuelinks = f["issuelinks"].as_array().unwrap();
        assert_eq!(issuelinks.len(), 2);
        let outward = issuelinks
            .iter()
            .find(|l| l["direction"].as_str() == Some("outward"))
            .unwrap();
        assert_eq!(outward["type"].as_str().unwrap(), "blocks");
        assert_eq!(outward["target_key"].as_str().unwrap(), "DO-1180");
        let inward = issuelinks
            .iter()
            .find(|l| l["direction"].as_str() == Some("inward"))
            .unwrap();
        assert_eq!(inward["type"].as_str().unwrap(), "is blocked by");

        // Comments
        let comments = f["comments"].as_array().unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0]["id"].as_str().unwrap(), "c1");
        assert_eq!(comments[0]["author"]["name"].as_str().unwrap(), "Bob");

        // Quelch internals
        assert_eq!(f["_partition_key"].as_str().unwrap(), "DO");
        assert!(!f["_deleted"].as_bool().unwrap());
        assert!(f["_deleted_at"].is_null());
    }

    // -----------------------------------------------------------------------
    // Test: custom fields are applied
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn custom_fields_are_applied() {
        let mut issue = full_issue_fixture();
        issue["fields"]["customfield_10016"] = json!(5);

        let mut custom_fields: HashMap<String, String> = HashMap::new();
        custom_fields.insert("story_points".into(), "customfield_10016".into());

        let doc = parse_issue(
            &issue,
            "jira-internal",
            "https://jira.example.com",
            &custom_fields,
        )
        .expect("parse_issue with custom fields should succeed");

        assert_eq!(doc.fields["story_points"].as_u64().unwrap(), 5);
    }

    // -----------------------------------------------------------------------
    // Test: pagination via startAt
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn paginates_via_start_at() {
        let server = MockServer::start().await;

        // First page: total=250, startAt=0, 100 issues
        let issues_page1: Vec<Value> = (0..100)
            .map(|i| {
                json!({
                    "key": format!("DO-{i}"),
                    "fields": {
                        "project": { "key": "DO" },
                        "issuetype": { "name": "Task" },
                        "status": { "name": "Open", "statusCategory": { "name": "To Do" } },
                        "priority": { "name": "Medium" },
                        "summary": format!("Issue {i}"),
                        "description": null,
                        "created": "2026-01-01T00:00:00.000+0000",
                        "updated": "2026-04-01T00:00:00.000+0000",
                    }
                })
            })
            .collect();

        Mock::given(method("POST"))
            .and(path("/rest/api/2/search"))
            .and(body_string_contains(r#""startAt":0"#))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issues": issues_page1,
                "startAt": 0,
                "maxResults": 100,
                "total": 250
            })))
            .mount(&server)
            .await;

        // Second page request (startAt=100)
        Mock::given(method("POST"))
            .and(path("/rest/api/2/search"))
            .and(body_string_contains(r#""startAt":100"#))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issues": [],
                "startAt": 100,
                "maxResults": 100,
                "total": 250
            })))
            .mount(&server)
            .await;

        let connector = build_connector(
            &server.uri(),
            "test",
            AuthConfig::DataCenter { pat: "x".into() },
        );

        let start: DateTime<Utc> = "2026-04-01T00:00:00Z".parse().unwrap();
        let end: DateTime<Utc> = "2026-04-01T01:00:00Z".parse().unwrap();

        // First page should have next_page_token = Some("100")
        let page1 = connector
            .fetch_window("DO", start, end, 100, None)
            .await
            .expect("first page should succeed");

        assert_eq!(page1.documents.len(), 100);
        assert_eq!(page1.next_page_token, Some("100".to_string()));

        // Second call with the token
        let page2 = connector
            .fetch_window("DO", start, end, 100, page1.next_page_token.as_deref())
            .await
            .expect("second page should succeed");

        assert_eq!(page2.documents.len(), 0);
        // total=250, startAt=100, issues returned=0 → 100 < 250 but page is empty
        // next_page_token should be None since issue count 0 means we're done
        assert!(page2.next_page_token.is_none());
    }

    // -----------------------------------------------------------------------
    // Test: fetch_backfill_page with resume clause
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn fetch_backfill_page_with_resume_clause() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/rest/api/2/search"))
            .and(body_string_contains("updated > "))
            .and(body_string_contains("key > "))
            .and(body_string_contains("DO-100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(empty_search_response()))
            .mount(&server)
            .await;

        let connector = build_connector(
            &server.uri(),
            "test",
            AuthConfig::DataCenter { pat: "x".into() },
        );

        let target: DateTime<Utc> = "2026-04-30T14:25:00Z".parse().unwrap();
        let last_seen = BackfillCheckpoint {
            updated: "2026-04-28T10:00:00Z".parse().unwrap(),
            key: "DO-100".to_string(),
        };

        let page = connector
            .fetch_backfill_page("DO", target, Some(&last_seen), 100)
            .await
            .expect("backfill with resume clause should succeed");

        assert!(page.documents.is_empty());
    }

    // -----------------------------------------------------------------------
    // Test: fetch_backfill_page without resume (first page)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn fetch_backfill_page_first_page() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/rest/api/2/search"))
            .and(body_string_contains(r#"updated <= \"2026/04/30 14:25\""#))
            .respond_with(ResponseTemplate::new(200).set_body_json(empty_search_response()))
            .mount(&server)
            .await;

        let connector = build_connector(
            &server.uri(),
            "test",
            AuthConfig::DataCenter { pat: "x".into() },
        );

        let target: DateTime<Utc> = "2026-04-30T14:25:00Z".parse().unwrap();

        connector
            .fetch_backfill_page("DO", target, None, 100)
            .await
            .expect("backfill first page (no resume) should succeed");
    }

    // -----------------------------------------------------------------------
    // Test: list_all_ids paginates
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_all_ids_paginates() {
        let server = MockServer::start().await;
        let call_count = Arc::new(AtomicUsize::new(0));

        // Page 1: 2 issues, total=3
        let page1_clone = call_count.clone();
        Mock::given(method("POST"))
            .and(path("/rest/api/2/search"))
            .and(body_string_contains(r#""startAt":0"#))
            .respond_with(move |_req: &wiremock::Request| {
                page1_clone.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_json(json!({
                    "issues": [
                        { "key": "DO-1" },
                        { "key": "DO-2" }
                    ],
                    "startAt": 0,
                    "maxResults": 100,
                    "total": 3
                }))
            })
            .mount(&server)
            .await;

        // Page 2: 1 issue
        Mock::given(method("POST"))
            .and(path("/rest/api/2/search"))
            .and(body_string_contains(r#""startAt":2"#))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issues": [
                    { "key": "DO-3" }
                ],
                "startAt": 2,
                "maxResults": 100,
                "total": 3
            })))
            .mount(&server)
            .await;

        let connector = build_connector(
            &server.uri(),
            "test",
            AuthConfig::DataCenter { pat: "x".into() },
        );

        let ids = connector
            .list_all_ids("DO")
            .await
            .expect("list_all_ids should succeed");

        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&"test-DO-1".to_string()));
        assert!(ids.contains(&"test-DO-2".to_string()));
        assert!(ids.contains(&"test-DO-3".to_string()));
    }

    // -----------------------------------------------------------------------
    // Test: fetch_companions handles missing agile API (404)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn fetch_companions_handles_missing_agile_api() {
        let server = MockServer::start().await;

        // Board endpoint returns 404
        Mock::given(method("GET"))
            .and(path("/rest/agile/1.0/board"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        // Versions endpoint returns empty array
        Mock::given(method("GET"))
            .and(path("/rest/api/2/project/DO/versions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        // Project endpoint returns a project
        Mock::given(method("GET"))
            .and(path("/rest/api/2/project/DO"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "10000",
                "key": "DO",
                "name": "DataOps",
                "description": "Main project",
                "projectTypeKey": "software",
                "lead": {
                    "accountId": "user-001",
                    "displayName": "Kristofer",
                    "emailAddress": "k@example.com"
                }
            })))
            .mount(&server)
            .await;

        let connector = build_connector(
            &server.uri(),
            "jira-internal",
            AuthConfig::DataCenter { pat: "x".into() },
        );

        let companions = connector
            .fetch_companions("DO")
            .await
            .expect("fetch_companions should not error on 404 from agile API");

        // Sprints should be empty (agile API was unavailable)
        assert!(
            companions.sprints.is_empty(),
            "sprints should be empty when agile API 404s"
        );

        // Fix versions should be empty (empty array returned)
        assert!(companions.fix_versions.is_empty());

        // Project should be present
        assert_eq!(companions.projects.len(), 1);
        assert_eq!(companions.projects[0].id, "jira-internal-DO");
        assert_eq!(
            companions.projects[0].fields["name"].as_str().unwrap(),
            "DataOps"
        );
    }

    // -----------------------------------------------------------------------
    // Test: sprint selection prefers active over future over closed
    // -----------------------------------------------------------------------

    #[test]
    fn sprint_prefers_active_over_future() {
        let fields = json!({
            "customfield_10020": [
                { "id": 1, "name": "Future Sprint", "state": "future" },
                { "id": 2, "name": "Active Sprint", "state": "active" },
                { "id": 3, "name": "Old Sprint", "state": "closed" }
            ]
        });

        let sprint = extract_sprint(&fields);
        assert_eq!(sprint["name"].as_str().unwrap(), "Active Sprint");
    }

    // -----------------------------------------------------------------------
    // Test: backfill last_seen is populated from page documents
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn backfill_last_seen_populated() {
        let server = MockServer::start().await;

        let issue = json!({
            "key": "DO-99",
            "fields": {
                "project": { "key": "DO" },
                "issuetype": { "name": "Task" },
                "status": { "name": "Open", "statusCategory": { "name": "To Do" } },
                "priority": null,
                "summary": "Test issue",
                "description": null,
                "created": "2026-01-01T00:00:00.000+0000",
                "updated": "2026-04-28T14:02:11.000+0000",
            }
        });

        Mock::given(method("POST"))
            .and(path("/rest/api/2/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issues": [issue],
                "startAt": 0,
                "maxResults": 100,
                "total": 1
            })))
            .mount(&server)
            .await;

        let connector = build_connector(
            &server.uri(),
            "test",
            AuthConfig::DataCenter { pat: "x".into() },
        );

        let target: DateTime<Utc> = "2026-04-30T14:25:00Z".parse().unwrap();
        let page = connector
            .fetch_backfill_page("DO", target, None, 100)
            .await
            .expect("backfill page should succeed");

        let last_seen = page.last_seen.expect("last_seen should be populated");
        assert_eq!(last_seen.key, "DO-99");
        assert_eq!(
            last_seen.updated,
            "2026-04-28T14:02:11Z".parse::<DateTime<Utc>>().unwrap()
        );
    }
}
