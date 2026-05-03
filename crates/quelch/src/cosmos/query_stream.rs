//! Internal `QueryStream` plumbing — not public beyond the `cosmos` module.

use async_trait::async_trait;
use serde_json::Value;

use crate::cosmos::CosmosError;

/// Object-safe inner cursor for a query result set.
#[async_trait]
pub(crate) trait QueryStreamInner: Send {
    /// Fetch the next page of results. Returns `None` when exhausted.
    async fn next_page(&mut self) -> Result<Option<Vec<Value>>, CosmosError>;

    /// Return a continuation token if the backend supports pagination, else `None`.
    fn continuation_token(&self) -> Option<&str>;
}

/// A paginated stream of Cosmos DB query results.
///
/// Both in-memory and real Azure backends return one of these.  The caller
/// drives pagination by calling `next_page` in a loop until it returns `None`.
pub struct QueryStream {
    pub(crate) inner: Box<dyn QueryStreamInner>,
}

impl QueryStream {
    /// Construct from a boxed inner implementation.
    pub(crate) fn new(inner: Box<dyn QueryStreamInner>) -> Self {
        Self { inner }
    }

    /// Fetch the next page. Returns `None` when the stream is exhausted.
    pub async fn next_page(&mut self) -> Result<Option<Vec<Value>>, CosmosError> {
        self.inner.next_page().await
    }

    /// Return the continuation token for resuming a query on the next call.
    /// Always `None` for in-memory; only meaningful for the real Azure backend.
    pub fn continuation_token(&self) -> Option<&str> {
        self.inner.continuation_token()
    }
}

// ---------------------------------------------------------------------------
// Vec-backed implementation (used by InMemoryCosmos)
// ---------------------------------------------------------------------------

/// A single-page stream backed by a `Vec<Value>`.
pub(crate) struct VecQueryStream {
    results: Option<Vec<Value>>,
}

impl VecQueryStream {
    pub(crate) fn new(results: Vec<Value>) -> Self {
        Self {
            results: Some(results),
        }
    }
}

#[async_trait]
impl QueryStreamInner for VecQueryStream {
    async fn next_page(&mut self) -> Result<Option<Vec<Value>>, CosmosError> {
        Ok(self.results.take())
    }

    fn continuation_token(&self) -> Option<&str> {
        None
    }
}
