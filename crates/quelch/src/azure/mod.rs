pub mod schema;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use thiserror::Error;
use tracing::debug;

use self::schema::IndexSchema;

const API_VERSION: &str = "2024-07-01";
const MAX_RETRY_ATTEMPTS: u32 = 3;

/// Emit a structured tracing event with Azure response metrics for the TUI.
fn emit_response_event(status_u16: u16, elapsed: std::time::Duration) {
    tracing::info!(
        phase = "azure_response",
        status = status_u16 as u64,
        latency_ms = elapsed.as_millis() as u64,
        throttled = (status_u16 == 429) as u64,
        "Azure response"
    );
}

/// Unwrap a `reqwest` send result while emitting a tracing event on both
/// success and transport failure. On transport error, `status = 0`.
fn emit_azure_response(
    send_result: Result<reqwest::Response, reqwest::Error>,
    start: Instant,
) -> Result<reqwest::Response, AzureError> {
    let elapsed = start.elapsed();
    match send_result {
        Ok(resp) => {
            emit_response_event(resp.status().as_u16(), elapsed);
            Ok(resp)
        }
        Err(e) => {
            emit_response_event(0, elapsed);
            Err(AzureError::Http(e))
        }
    }
}

#[derive(Debug, Error)]
pub enum AzureError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Azure API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Client for Azure AI Search REST API.
pub struct SearchClient {
    client: Client,
    endpoint: String,
    api_key: String,
}

#[derive(Debug, Serialize)]
struct IndexBatch {
    value: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    value: Vec<serde_json::Value>,
    #[serde(rename = "@odata.nextLink")]
    #[allow(dead_code)]
    next_link: Option<String>,
}

impl SearchClient {
    pub fn new(endpoint: &str, api_key: &str) -> Self {
        let endpoint = endpoint.trim_end_matches('/').to_string();
        Self {
            client: Client::new(),
            endpoint,
            api_key: api_key.to_string(),
        }
    }

    /// Check if an index exists. Returns true if it does.
    pub async fn index_exists(&self, index_name: &str) -> Result<bool, AzureError> {
        let url = format!(
            "{}/indexes/{}?api-version={}",
            self.endpoint, index_name, API_VERSION
        );

        let start = Instant::now();
        let send_result = self
            .client
            .get(&url)
            .header("api-key", &self.api_key)
            .send()
            .await;
        let resp = emit_azure_response(send_result, start)?;

        if resp.status().is_success() {
            Ok(true)
        } else if resp.status().as_u16() == 404 {
            Ok(false)
        } else {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            Err(AzureError::Api {
                status,
                message: body,
            })
        }
    }

    /// Create an index with the given schema. Fails if index already exists (409).
    /// Retries transient 429/5xx responses using exponential backoff.
    pub async fn create_index(&self, schema: &IndexSchema) -> Result<(), AzureError> {
        let url = format!("{}/indexes?api-version={}", self.endpoint, API_VERSION);

        let resp = self
            .request_with_retry(|| {
                self.client
                    .post(&url)
                    .header("api-key", &self.api_key)
                    .header("Content-Type", "application/json")
                    .json(schema)
            })
            .await?;

        if resp.status().is_success() {
            debug!("Created index '{}'", schema.name);
            Ok(())
        } else {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            Err(AzureError::Api {
                status,
                message: body,
            })
        }
    }

    /// Delete an index. Returns Ok even if the index doesn't exist.
    pub async fn delete_index(&self, index_name: &str) -> Result<(), AzureError> {
        let url = format!(
            "{}/indexes/{}?api-version={}",
            self.endpoint, index_name, API_VERSION
        );

        let start = Instant::now();
        let send_result = self
            .client
            .delete(&url)
            .header("api-key", &self.api_key)
            .send()
            .await;
        let resp = emit_azure_response(send_result, start)?;

        if resp.status().is_success() || resp.status().as_u16() == 404 {
            debug!("Deleted index '{}'", index_name);
            Ok(())
        } else {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            Err(AzureError::Api {
                status,
                message: body,
            })
        }
    }

    /// Perform a semantic search query against an index.
    pub async fn search(
        &self,
        index_name: &str,
        query: &str,
        semantic_config: &str,
        top: usize,
    ) -> Result<serde_json::Value, AzureError> {
        let url = format!(
            "{}/indexes/{}/docs/search?api-version={}",
            self.endpoint, index_name, API_VERSION
        );

        let body = serde_json::json!({
            "search": query,
            "queryType": "semantic",
            "semanticConfiguration": semantic_config,
            "top": top,
            "count": true,
            "answers": "extractive|count-3",
            "captions": "extractive|highlight-true"
        });

        let start = Instant::now();
        let send_result = self
            .client
            .post(&url)
            .header("api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;
        let resp = emit_azure_response(send_result, start)?;

        if resp.status().is_success() {
            let result: serde_json::Value = resp.json().await?;
            Ok(result)
        } else {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            Err(AzureError::Api {
                status,
                message: body,
            })
        }
    }

    /// Check if index exists, create if missing. Auto-creates without prompting.
    pub async fn ensure_index(&self, schema: &IndexSchema) -> Result<(), AzureError> {
        if self.index_exists(&schema.name).await? {
            debug!("Index '{}' already exists", schema.name);
            return Ok(());
        }
        self.create_index(schema).await
    }

    /// Push documents to an index using merge-or-upload action.
    pub async fn push_documents(
        &self,
        index_name: &str,
        documents: Vec<serde_json::Value>,
    ) -> Result<(), AzureError> {
        if documents.is_empty() {
            return Ok(());
        }

        let url = format!(
            "{}/indexes/{}/docs/index?api-version={}",
            self.endpoint, index_name, API_VERSION
        );

        let docs_with_action: Vec<serde_json::Value> = documents
            .into_iter()
            .map(|mut doc| {
                if let Some(obj) = doc.as_object_mut() {
                    obj.insert(
                        "@search.action".to_string(),
                        serde_json::Value::String("mergeOrUpload".to_string()),
                    );
                }
                doc
            })
            .collect();

        let batch = IndexBatch {
            value: docs_with_action,
        };

        let resp = self
            .request_with_retry(|| {
                self.client
                    .post(&url)
                    .header("api-key", &self.api_key)
                    .header("Content-Type", "application/json")
                    .json(&batch)
            })
            .await?;

        let status = resp.status();
        if status.is_success() || status.as_u16() == 207 {
            // 200 = all succeeded, 207 = partial success (check per-doc status)
            Ok(())
        } else {
            let status_code = status.as_u16();
            let body = resp.text().await.unwrap_or_default();
            Err(AzureError::Api {
                status: status_code,
                message: body,
            })
        }
    }

    /// Count documents in an index, optionally restricted by an OData
    /// `$filter`. Returns 0 if the index doesn't exist (treating a missing
    /// index as "empty" is what the TUI wants; it avoids surfacing a
    /// temporary 404 during initial setup as an error).
    pub async fn count_documents(
        &self,
        index_name: &str,
        filter: Option<&str>,
    ) -> Result<u64, AzureError> {
        let mut url = format!(
            "{}/indexes/{}/docs/$count?api-version={}",
            self.endpoint, index_name, API_VERSION
        );
        if let Some(f) = filter {
            // URL-encode the OData expression. Spaces → %20, apostrophes stay
            // as-is inside the quoted value (Azure accepts them raw).
            let encoded = f.replace(' ', "%20");
            url.push_str("&$filter=");
            url.push_str(&encoded);
        }
        let resp = self
            .client
            .get(&url)
            .header("api-key", &self.api_key)
            .send()
            .await?;
        match resp.status().as_u16() {
            200 => {
                let body = resp.text().await.unwrap_or_default();
                // Azure returns the count as a bare integer in the body
                // (no JSON envelope). Trim trailing whitespace / BOM.
                let trimmed = body.trim().trim_start_matches('\u{feff}');
                Ok(trimmed.parse::<u64>().unwrap_or(0))
            }
            404 => Ok(0),
            status => {
                let body = resp.text().await.unwrap_or_default();
                Err(AzureError::Api {
                    status,
                    message: body,
                })
            }
        }
    }

    /// Fetch all document IDs from an index (for delete detection).
    pub async fn fetch_all_ids(&self, index_name: &str) -> Result<Vec<String>, AzureError> {
        let mut ids = Vec::new();
        let mut skip: usize = 0;
        let top: usize = 1000;

        loop {
            let url = format!(
                "{}/indexes/{}/docs?api-version={}&search=*&$select=id&$top={}&$skip={}&$orderby=id",
                self.endpoint, index_name, API_VERSION, top, skip
            );

            let start = Instant::now();
            let send_result = self
                .client
                .get(&url)
                .header("api-key", &self.api_key)
                .send()
                .await;
            let resp = emit_azure_response(send_result, start)?;

            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                return Err(AzureError::Api {
                    status,
                    message: body,
                });
            }

            let search_resp: SearchResponse = resp.json().await?;
            let batch_len = search_resp.value.len();

            for doc in search_resp.value {
                if let Some(id) = doc.get("id").and_then(|v| v.as_str()) {
                    ids.push(id.to_string());
                }
            }

            if batch_len < top {
                break;
            }
            skip += top;
        }

        Ok(ids)
    }

    /// Delete documents by ID from an index.
    pub async fn delete_documents(
        &self,
        index_name: &str,
        ids: &[String],
    ) -> Result<(), AzureError> {
        if ids.is_empty() {
            return Ok(());
        }

        let url = format!(
            "{}/indexes/{}/docs/index?api-version={}",
            self.endpoint, index_name, API_VERSION
        );

        let docs: Vec<serde_json::Value> = ids
            .iter()
            .map(|id| {
                serde_json::json!({
                    "@search.action": "delete",
                    "id": id
                })
            })
            .collect();

        let batch = IndexBatch { value: docs };

        let resp = self
            .request_with_retry(|| {
                self.client
                    .post(&url)
                    .header("api-key", &self.api_key)
                    .header("Content-Type", "application/json")
                    .json(&batch)
            })
            .await?;

        if resp.status().is_success() {
            Ok(())
        } else {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            Err(AzureError::Api {
                status,
                message: body,
            })
        }
    }

    /// Execute a request with exponential backoff retry on 429/5xx.
    async fn request_with_retry<F>(&self, build_request: F) -> Result<reqwest::Response, AzureError>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let mut last_err = None;
        for attempt in 0..MAX_RETRY_ATTEMPTS {
            if attempt > 0 {
                let delay = std::time::Duration::from_secs(1 << attempt);
                // At `info!` so the TUI's `TuiLayer` (filter `quelch=info`)
                // catches it and lights up the backoff banner, while plain-log
                // default (`quelch=warn,sim=info`) stays quiet — an operator
                // doesn't need a per-retry line when the engine is making
                // forward progress. `-v` promotes the filter and shows it.
                tracing::info!(
                    phase = crate::sync::phases::BACKOFF_STARTED,
                    source = "azure",
                    reason = "HTTP 429 or 5xx",
                    delay_ms = delay.as_millis() as u64,
                    "Retrying after backoff"
                );
                tokio::time::sleep(delay).await;
                tracing::info!(
                    phase = crate::sync::phases::BACKOFF_FINISHED,
                    source = "azure",
                    "Backoff finished"
                );
            }

            let start = Instant::now();
            let send_result = build_request().send().await;
            let elapsed = start.elapsed();
            match send_result {
                Ok(resp) if resp.status() == 429 || resp.status().is_server_error() => {
                    let status = resp.status();
                    emit_response_event(status.as_u16(), elapsed);
                    let body = resp.text().await.unwrap_or_default();
                    // Transient: the next loop iteration will emit the
                    // structured `backoff_started` and retry. We only surface
                    // a WARN at MAX_RETRY_ATTEMPTS exhaustion (below) — before
                    // then, printing a per-attempt line is noise.
                    debug!(status = %status, body = %body, "Azure request failed (will retry)");
                    last_err = Some(AzureError::Api {
                        status: status.as_u16(),
                        message: body,
                    });
                }
                Ok(resp) => {
                    emit_response_event(resp.status().as_u16(), elapsed);
                    return Ok(resp);
                }
                Err(e) => {
                    emit_response_event(0, elapsed);
                    debug!(error = %e, "Azure transport error (will retry)");
                    last_err = Some(AzureError::Http(e));
                }
            }
        }
        Err(last_err.unwrap())
    }
}
