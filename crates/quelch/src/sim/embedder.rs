//! SimEmbedder: wraps DeterministicEmbedder with jittery sleep to make
//! Azure p50/p95 charts meaningful.

use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;
use std::sync::Mutex;
use std::time::Duration;

use crate::sync::embedder::{DeterministicEmbedder, EmbedFuture, Embedder};

pub struct SimEmbedder {
    inner: DeterministicEmbedder,
    rng: Mutex<StdRng>,
}

impl SimEmbedder {
    pub fn new(dims: usize, seed: Option<u64>) -> Self {
        let rng = match seed {
            Some(s) => StdRng::seed_from_u64(s.wrapping_add(1)),
            None => StdRng::from_entropy(),
        };
        Self {
            inner: DeterministicEmbedder::new(dims),
            rng: Mutex::new(rng),
        }
    }

    fn pick_jitter(&self) -> Duration {
        let mut r = self.rng.lock().unwrap();
        let ms = if r.r#gen::<f32>() < 0.01 {
            r.gen_range(300u64..=500)
        } else {
            r.gen_range(20u64..=150)
        };
        Duration::from_millis(ms)
    }
}

impl Embedder for SimEmbedder {
    fn embed_one<'a>(&'a self, text: &'a str) -> EmbedFuture<'a> {
        let jitter = self.pick_jitter();
        Box::pin(async move {
            tokio::time::sleep(jitter).await;
            self.inner.embed_one(text).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn same_text_same_vector() {
        let e = SimEmbedder::new(8, Some(42));
        let a = e.embed_one("hi").await.unwrap();
        let b = e.embed_one("hi").await.unwrap();
        assert_eq!(a, b);
    }

    #[tokio::test]
    async fn non_zero_latency() {
        let e = SimEmbedder::new(8, Some(42));
        let t = std::time::Instant::now();
        e.embed_one("x").await.unwrap();
        assert!(t.elapsed() >= Duration::from_millis(20));
    }
}
