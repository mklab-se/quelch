//! Minimal ARM REST client for Cosmos DB control-plane operations.
//!
//! All requests target `management.azure.com` and authenticate with a
//! `TokenCredential` (typically `DefaultAzureCredential`). The client only
//! covers the three verbs Quelch needs: ensure database, list containers,
//! create container.

use anyhow::{Context, Result};
use azure_core::auth::TokenCredential;
use serde::Deserialize;
use std::sync::Arc;

use super::diff::ContainerSpec;

const ARM: &str = "https://management.azure.com";
const COSMOS_API_VERSION: &str = "2024-05-15";
const ARM_SCOPE: &str = "https://management.azure.com/.default";

/// ARM REST client for a single Cosmos DB account.
///
/// Construct directly — fields are public so callers (typically `azure apply`)
/// can wire any [`TokenCredential`], not just `DefaultAzureCredential`.
pub struct ArmCosmosClient {
    /// Token source (e.g. `DefaultAzureCredential`).
    pub credential: Arc<dyn TokenCredential>,
    /// Reusable HTTP client.
    pub http: reqwest::Client,
    /// Azure subscription containing the Cosmos account.
    pub subscription_id: String,
    /// Resource group containing the Cosmos account.
    pub resource_group: String,
    /// Cosmos DB account name.
    pub account: String,
}

impl ArmCosmosClient {
    async fn token(&self) -> Result<String> {
        let token = self
            .credential
            .get_token(&[ARM_SCOPE])
            .await
            .context("acquire ARM access token")?;
        Ok(token.token.secret().to_string())
    }

    /// Ensure a SQL database exists (idempotent PUT).
    ///
    /// Returns `Ok(())` on 2xx. Any other status returns an error containing
    /// the URL, status, and response body to make RBAC / 403 issues
    /// debuggable.
    pub async fn ensure_database(&self, db: &str) -> Result<()> {
        let url = format!(
            "{ARM}/subscriptions/{}/resourceGroups/{}/providers/Microsoft.DocumentDB/databaseAccounts/{}/sqlDatabases/{}?api-version={COSMOS_API_VERSION}",
            self.subscription_id, self.resource_group, self.account, db
        );
        let body = serde_json::json!({
            "properties": { "resource": { "id": db } }
        });
        let token = self.token().await?;
        let resp = self
            .http
            .put(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("PUT {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("PUT database {db} -> {url}: {status} - {body}");
        }
        Ok(())
    }

    /// List the SQL containers in `db`.
    ///
    /// Returns an empty vector if the database itself does not exist (ARM
    /// 404), so callers can use this method as the read side of a plan
    /// without first probing the database.
    pub async fn list_containers(&self, db: &str) -> Result<Vec<ContainerSpec>> {
        let url = format!(
            "{ARM}/subscriptions/{}/resourceGroups/{}/providers/Microsoft.DocumentDB/databaseAccounts/{}/sqlDatabases/{}/containers?api-version={COSMOS_API_VERSION}",
            self.subscription_id, self.resource_group, self.account, db
        );
        let token = self.token().await?;
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(vec![]);
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("LIST containers {url}: {status} - {body}");
        }

        #[derive(Deserialize)]
        struct ListResp {
            value: Vec<ContainerArmRecord>,
        }
        #[derive(Deserialize)]
        struct ContainerArmRecord {
            name: String,
            properties: ContainerArmProps,
        }
        #[derive(Deserialize)]
        struct ContainerArmProps {
            resource: ContainerArmResource,
        }
        #[derive(Deserialize)]
        struct ContainerArmResource {
            #[serde(rename = "partitionKey")]
            partition_key: PartitionKey,
        }
        #[derive(Deserialize)]
        struct PartitionKey {
            paths: Vec<String>,
        }

        let parsed: ListResp = resp.json().await.context("parse container list")?;
        Ok(parsed
            .value
            .into_iter()
            .map(|r| ContainerSpec {
                name: r.name,
                partition_key: r
                    .properties
                    .resource
                    .partition_key
                    .paths
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| "/id".to_string()),
            })
            .collect())
    }

    /// Create a SQL container with the given partition key.
    ///
    /// Caller is responsible for not invoking this when the container already
    /// exists with a conflicting partition key — see
    /// [`super::diff::diff_containers`] and `super::apply`.
    pub async fn create_container(&self, db: &str, spec: &ContainerSpec) -> Result<()> {
        let url = format!(
            "{ARM}/subscriptions/{}/resourceGroups/{}/providers/Microsoft.DocumentDB/databaseAccounts/{}/sqlDatabases/{}/containers/{}?api-version={COSMOS_API_VERSION}",
            self.subscription_id, self.resource_group, self.account, db, spec.name
        );
        let body = serde_json::json!({
            "properties": {
                "resource": {
                    "id": spec.name,
                    "partitionKey": { "paths": [spec.partition_key], "kind": "Hash" }
                }
            }
        });
        let token = self.token().await?;
        let resp = self
            .http
            .put(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("PUT {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("PUT container {} -> {url}: {status} - {body}", spec.name);
        }
        Ok(())
    }
}
