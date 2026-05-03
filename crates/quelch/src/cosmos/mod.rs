//! Cosmos DB abstraction layer.
//!
//! # Overview
//!
//! This module provides a `CosmosBackend` trait that abstracts over Cosmos DB
//! upsert / point-read / SQL-query operations, plus:
//!
//! - `InMemoryCosmos` — a fully in-process implementation for tests and
//!   `quelch dev`.
//! - `meta` — cursor state CRUD stored in the `quelch-meta` container.
//!
//! The real `azure_data_cosmos`-backed client lives in `client.rs`.

pub mod client;
pub mod document;
pub mod error;
pub mod factory;
pub mod in_memory;
pub mod meta;
pub(crate) mod query_stream;

pub use client::CosmosClient;
pub use document::CosmosDocument;
pub use error::CosmosError;
pub use in_memory::InMemoryCosmos;
pub use query_stream::QueryStream;

use async_trait::async_trait;
use serde_json::Value;

/// Abstraction over Cosmos DB operations needed by Quelch.
///
/// All implementations must be `Send + Sync` so they can be shared across
/// async tasks.
#[async_trait]
pub trait CosmosBackend: Send + Sync {
    /// Upsert a document by id.
    ///
    /// The document must contain both an `id` string field and a
    /// `_partition_key` string field.  Returns `CosmosError::Validation` if
    /// either is missing.
    async fn upsert(&self, container: &str, doc: Value) -> Result<(), CosmosError>;

    /// Bulk upsert.
    ///
    /// The default implementation falls back to N individual `upsert` calls.
    /// Concrete backends (e.g. the real Azure client) may override this to use
    /// the Cosmos batch API.
    async fn bulk_upsert(&self, container: &str, docs: Vec<Value>) -> Result<(), CosmosError> {
        for doc in docs {
            self.upsert(container, doc).await?;
        }
        Ok(())
    }

    /// Point-read by id and partition key.
    ///
    /// Returns `None` if the document does not exist.
    async fn get(
        &self,
        container: &str,
        id: &str,
        partition_key: &str,
    ) -> Result<Option<Value>, CosmosError>;

    /// Run a parameterised SQL query, returning a paginated stream.
    ///
    /// Only the minimal SQL subset documented in `InMemoryCosmos` is
    /// guaranteed to work across all backends.  Callers should stick to that
    /// subset for portability.
    async fn query(
        &self,
        container: &str,
        sql: &str,
        params: Vec<(String, Value)>,
    ) -> Result<QueryStream, CosmosError>;
}
