//! Embedder abstraction — engine uses `&dyn Embedder`, not a concrete client.
//!
//! Production wiring in `main.rs` passes `ailloy::Client` as `&dyn Embedder`.
//! Tests pass `DeterministicEmbedder` to avoid any network I/O.
//!
//! Note: the trait uses `Pin<Box<dyn Future>>` return types rather than
//! `async fn` so that it remains dyn-compatible (required by the sync engine's
//! `Option<&dyn Embedder>` parameter).

use anyhow::Result;
use std::future::Future;
use std::pin::Pin;

/// Boxed future returned by [`Embedder::embed_one`].
pub type EmbedFuture<'a> = Pin<Box<dyn Future<Output = Result<Vec<f32>>> + Send + 'a>>;

/// Abstraction over an embedding model; implemented by `ailloy::Client` in
/// production and `DeterministicEmbedder` in tests.
pub trait Embedder: Send + Sync {
    /// Embed a single piece of text into a dense vector.
    fn embed_one<'a>(&'a self, text: &'a str) -> EmbedFuture<'a>;
}

impl Embedder for ailloy::Client {
    fn embed_one<'a>(&'a self, text: &'a str) -> EmbedFuture<'a> {
        Box::pin(async move { ailloy::Client::embed_one(self, text).await })
    }
}

/// Deterministic test embedder: hashes text to a fixed-size vector.
/// Same input always produces the same vector — good for assertions.
pub struct DeterministicEmbedder {
    /// Dimensionality of the output vectors produced by [`Embedder::embed_one`].
    pub dims: usize,
}

impl DeterministicEmbedder {
    /// Construct a new [`DeterministicEmbedder`] producing vectors of `dims` floats.
    pub fn new(dims: usize) -> Self {
        Self { dims }
    }
}

impl Embedder for DeterministicEmbedder {
    fn embed_one<'a>(&'a self, text: &'a str) -> EmbedFuture<'a> {
        Box::pin(async move {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};

            let mut out = Vec::with_capacity(self.dims);
            for i in 0..self.dims {
                let mut h = DefaultHasher::new();
                (i as u32).hash(&mut h);
                text.hash(&mut h);
                let raw = h.finish();
                // Map to [-1.0, 1.0]
                let f = (raw as f64 / u64::MAX as f64) * 2.0 - 1.0;
                out.push(f as f32);
            }
            Ok(out)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn deterministic_embedder_is_stable() {
        let e = DeterministicEmbedder::new(8);
        let v1 = e.embed_one("hello").await.unwrap();
        let v2 = e.embed_one("hello").await.unwrap();
        assert_eq!(v1, v2);
        assert_eq!(v1.len(), 8);
    }

    #[tokio::test]
    async fn deterministic_embedder_differs_by_input() {
        let e = DeterministicEmbedder::new(16);
        let a = e.embed_one("foo").await.unwrap();
        let b = e.embed_one("bar").await.unwrap();
        assert_ne!(a, b);
    }
}
