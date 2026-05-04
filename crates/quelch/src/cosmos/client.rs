//! Production Azure Cosmos DB backend backed by `azure_data_cosmos`.
//!
//! # Authentication
//!
//! Uses `DefaultAzureCredential` from `azure_identity`, which tries (in order):
//! 1. Environment variables (`AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_TENANT_ID`)
//! 2. App Service managed identity
//! 3. Virtual-machine IMDS managed identity
//! 4. `az login` cached credential (developer workstations)
//!
//! # Usage
//!
//! ```no_run
//! use quelch::cosmos::client::CosmosClient;
//!
//! # async fn run() -> anyhow::Result<()> {
//! let client = CosmosClient::new(
//!     "https://my-account.documents.azure.com:443/",
//!     "quelch",
//! ).await?;
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use azure_data_cosmos::prelude::{
    AuthorizationToken, CollectionClient, CosmosClient as AzCosmosClient, CosmosClientBuilder,
    GetDocumentResponse, Param, Query, QueryCrossPartition,
};
use azure_identity::DefaultAzureCredentialBuilder;
use futures::StreamExt as _;
use serde_json::Value;

use crate::cosmos::{
    CosmosBackend, CosmosError, QueryStream, document::CosmosDocument,
    query_stream::QueryStreamInner,
};

// ---------------------------------------------------------------------------
// Public client
// ---------------------------------------------------------------------------

/// Production `CosmosBackend` implementation backed by `azure_data_cosmos`.
///
/// Construct with [`CosmosClient::new`].  All operations are async and can be
/// shared across tasks (the struct is `Clone`, `Send`, `Sync`).
#[derive(Clone)]
pub struct CosmosClient {
    inner: AzCosmosClient,
    database_name: String,
}

impl CosmosClient {
    /// Construct a new `CosmosClient`.
    ///
    /// * `account_endpoint` — The full Cosmos DB account endpoint URL, e.g.
    ///   `"https://my-account.documents.azure.com:443/"`.
    /// * `database_name` — Name of the Cosmos DB database.
    ///
    /// Authentication uses `DefaultAzureCredential`: it will try environment
    /// variables, managed identity, and `az login` in that order.
    pub async fn new(account_endpoint: &str, database_name: &str) -> Result<Self, CosmosError> {
        let credential = DefaultAzureCredentialBuilder::default()
            .build()
            .map_err(|e| {
                CosmosError::Backend(format!("failed to build DefaultAzureCredential: {e}"))
            })?;

        let auth_token = AuthorizationToken::from_token_credential(Arc::new(credential));

        // Strip trailing slash to get the account name part (CosmosClientBuilder
        // with_location / Custom variant takes the full URI).
        let inner =
            CosmosClientBuilder::with_location(azure_data_cosmos::prelude::CloudLocation::Custom {
                uri: account_endpoint.trim_end_matches('/').to_string(),
                auth_token,
            })
            .build();

        Ok(Self {
            inner,
            database_name: database_name.to_string(),
        })
    }

    /// Return a [`CollectionClient`] for the named container.
    fn collection(&self, container: &str) -> CollectionClient {
        self.inner
            .database_client(self.database_name.clone())
            .collection_client(container.to_string())
    }
}

// ---------------------------------------------------------------------------
// CosmosBackend implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl CosmosBackend for CosmosClient {
    async fn upsert(&self, container: &str, doc: Value) -> Result<(), CosmosError> {
        let id = CosmosDocument::extract_id(&doc)?.to_string();
        let pk = CosmosDocument::extract_partition_key(&doc)?.to_string();

        let collection = self.collection(container);

        // `create_document` with `is_upsert(true)` = upsert semantics.
        // `Value` implements `CosmosEntity` (returns itself as the partition key)
        // but we need the *actual* partition key string, so we supply it explicitly.
        collection
            .create_document(doc)
            .is_upsert(true)
            .partition_key(&pk)
            .map_err(sdk_err)?
            .into_future()
            .await
            .map_err(sdk_err)?;

        let _ = id; // captured to ensure extract_id was called for validation
        Ok(())
    }

    async fn get(
        &self,
        container: &str,
        id: &str,
        partition_key: &str,
    ) -> Result<Option<Value>, CosmosError> {
        let collection = self.collection(container);

        let document_client = collection
            .document_client(id, &partition_key)
            .map_err(sdk_err)?;

        let response: GetDocumentResponse<Value> = document_client
            .get_document()
            .into_future()
            .await
            .map_err(sdk_err)?;

        match response {
            GetDocumentResponse::Found(found) => Ok(Some(found.document.document)),
            GetDocumentResponse::NotFound(_) => Ok(None),
        }
    }

    async fn query(
        &self,
        container: &str,
        sql: &str,
        params: Vec<(String, Value)>,
    ) -> Result<QueryStream, CosmosError> {
        let sdk_params: Vec<Param> = params
            .into_iter()
            .map(|(name, value)| Param::new(name, value))
            .collect();

        let query = Query::with_params(sql.to_string(), sdk_params);

        let collection = self.collection(container);

        // Cross-partition queries are enabled so the SQL subset documented
        // in `InMemoryCosmos` works regardless of partition layout.
        let pageable = collection
            .query_documents(query)
            .query_cross_partition(QueryCrossPartition::Yes)
            .into_stream::<Value>();

        Ok(QueryStream::new(Box::new(SdkQueryStream {
            pageable,
            continuation: None,
        })))
    }
}

// ---------------------------------------------------------------------------
// QueryStreamInner backed by the SDK Pageable
// ---------------------------------------------------------------------------

type SdkPageable = azure_data_cosmos::prelude::QueryDocuments<Value>;

struct SdkQueryStream {
    pageable: SdkPageable,
    continuation: Option<String>,
}

// SAFETY: `SdkPageable` is `Send` (azure_core declares `+ Send` on
// non-wasm32 targets).  We wrap it in a plain struct so we need the
// explicit assertion.
unsafe impl Send for SdkQueryStream {}

#[async_trait]
impl QueryStreamInner for SdkQueryStream {
    async fn next_page(&mut self) -> Result<Option<Vec<Value>>, CosmosError> {
        match self.pageable.next().await {
            None => {
                self.continuation = None;
                Ok(None)
            }
            Some(Err(e)) => Err(sdk_err(e)),
            Some(Ok(response)) => {
                // Update the stored continuation token. We extract the raw
                // string via the `Header` trait — `Continuation`'s public API
                // exposes only `Debug` (which produces `Continuation("token")`)
                // and the `Header::value()` method.
                use azure_core::headers::Header;
                self.continuation = response
                    .continuation_token
                    .as_ref()
                    .map(|c| c.value().as_str().to_owned());

                // Extract the document payloads; ignore `DocumentAttributes`.
                let docs: Vec<Value> = response
                    .results
                    .into_iter()
                    .map(|(doc, _attrs)| doc)
                    .collect();

                Ok(Some(docs))
            }
        }
    }

    fn continuation_token(&self) -> Option<&str> {
        self.continuation.as_deref()
    }
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

/// Map an `azure_core::error::Error` into `CosmosError::Backend`.
fn sdk_err(e: impl std::fmt::Display) -> CosmosError {
    CosmosError::Backend(e.to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- Unit: id/partition_key extraction ---------------------------------

    #[test]
    fn extracts_id_and_partition_key() {
        let doc = json!({
            "id": "prod::my-jira::DO-1",
            "_partition_key": "prod",
            "title": "Fix the bug"
        });

        let id = CosmosDocument::extract_id(&doc).unwrap();
        let pk = CosmosDocument::extract_partition_key(&doc).unwrap();

        assert_eq!(id, "prod::my-jira::DO-1");
        assert_eq!(pk, "prod");
    }

    #[test]
    fn extract_id_missing_returns_validation_error() {
        let doc = json!({ "_partition_key": "prod" });
        let result = CosmosDocument::extract_id(&doc);
        assert!(matches!(result, Err(CosmosError::Validation(_))));
    }

    #[test]
    fn extract_partition_key_missing_returns_validation_error() {
        let doc = json!({ "id": "some-id" });
        let result = CosmosDocument::extract_partition_key(&doc);
        assert!(matches!(result, Err(CosmosError::Validation(_))));
    }

    // ---- Unit: sdk_err mapping --------------------------------------------

    #[test]
    fn sdk_err_wraps_message_in_backend_variant() {
        let err = sdk_err("connection refused");
        assert!(matches!(err, CosmosError::Backend(_)));
        let CosmosError::Backend(msg) = err else {
            panic!("wrong variant");
        };
        assert!(msg.contains("connection refused"));
    }

    #[test]
    fn sdk_err_wraps_any_display_type() {
        // Use a plain std error to verify the generic `impl Display` overload.
        let io_err = std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out connecting");
        let cosmos_err = sdk_err(io_err);
        assert!(matches!(cosmos_err, CosmosError::Backend(_)));
        let CosmosError::Backend(msg) = cosmos_err else {
            panic!("wrong variant");
        };
        assert!(msg.contains("timed out connecting"));
    }

    // ---- Integration: end-to-end against a real Cosmos account -------------

    /// Full round-trip against a live Azure Cosmos DB account.
    ///
    /// Requires:
    /// - The env var `QUELCH_COSMOS_E2E_ENDPOINT` set to the account endpoint.
    /// - A database named `quelch-test` with a container `e2e-test` (partition
    ///   key `/∫_partition_key`).
    /// - Azure credentials available (`az login` or env vars).
    #[tokio::test]
    #[ignore = "requires Azure Cosmos; set QUELCH_COSMOS_E2E_ENDPOINT to enable"]
    async fn e2e_upsert_get_query_round_trip() {
        let endpoint = match std::env::var("QUELCH_COSMOS_E2E_ENDPOINT") {
            Ok(e) => e,
            Err(_) => return, // defensive; #[ignore] should prevent this from running
        };

        let client = CosmosClient::new(&endpoint, "quelch-test")
            .await
            .expect("CosmosClient::new should succeed with valid credentials");

        let container = "e2e-test";
        let doc = json!({
            "id":              "e2e-test-doc-1",
            "_partition_key":  "e2e",
            "title":           "E2E round-trip document",
            "value":           42
        });

        // ---- upsert --------------------------------------------------------
        client
            .upsert(container, doc.clone())
            .await
            .expect("upsert should succeed");

        // ---- get -----------------------------------------------------------
        let fetched = client
            .get(container, "e2e-test-doc-1", "e2e")
            .await
            .expect("get should succeed");

        assert!(fetched.is_some(), "document should exist after upsert");
        let fetched = fetched.unwrap();
        assert_eq!(fetched["title"], "E2E round-trip document");
        assert_eq!(fetched["value"], 42);

        // ---- query ---------------------------------------------------------
        let mut stream = client
            .query(
                container,
                "SELECT * FROM c WHERE c.id = @id",
                vec![("@id".into(), json!("e2e-test-doc-1"))],
            )
            .await
            .expect("query should succeed");

        let page = stream
            .next_page()
            .await
            .expect("next_page should succeed")
            .expect("should have at least one page");

        assert_eq!(page.len(), 1);
        assert_eq!(page[0]["id"], "e2e-test-doc-1");
        assert_eq!(page[0]["value"], 42);

        // ---- get missing ---------------------------------------------------
        let missing = client
            .get(container, "definitely-does-not-exist", "e2e")
            .await
            .expect("get of missing doc should return Ok(None)");

        assert!(missing.is_none());

        // ---- second upsert overwrites ---------------------------------------
        let updated = json!({
            "id":             "e2e-test-doc-1",
            "_partition_key": "e2e",
            "title":          "Updated document",
            "value":          99
        });
        client
            .upsert(container, updated)
            .await
            .expect("upsert overwrite should succeed");

        let refetched = client
            .get(container, "e2e-test-doc-1", "e2e")
            .await
            .expect("get after overwrite should succeed")
            .expect("document should still exist");

        assert_eq!(refetched["value"], 99);
    }
}
