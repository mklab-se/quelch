pub mod schema;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, warn};

use self::schema::IndexSchema;

const API_VERSION: &str = "2024-07-01";
const MAX_RETRY_ATTEMPTS: u32 = 3;

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

        let resp = self
            .client
            .get(&url)
            .header("api-key", &self.api_key)
            .send()
            .await?;

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

        let resp = self
            .client
            .delete(&url)
            .header("api-key", &self.api_key)
            .send()
            .await?;

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

        let resp = self
            .client
            .post(&url)
            .header("api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

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

            let resp = self
                .client
                .get(&url)
                .header("api-key", &self.api_key)
                .send()
                .await?;

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
                warn!(
                    "Retrying after {:?} (attempt {}/{})",
                    delay,
                    attempt + 1,
                    MAX_RETRY_ATTEMPTS
                );
                tokio::time::sleep(delay).await;
            }

            match build_request().send().await {
                Ok(resp) if resp.status() == 429 || resp.status().is_server_error() => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    warn!("Request failed with {}: {}", status, body);
                    last_err = Some(AzureError::Api {
                        status: status.as_u16(),
                        message: body,
                    });
                }
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    warn!("Request error: {}", e);
                    last_err = Some(AzureError::Http(e));
                }
            }
        }
        Err(last_err.unwrap())
    }
}
