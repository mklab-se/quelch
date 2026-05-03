//! Rate-limit-aware HTTP client.
//!
//! Wraps `reqwest` with a middleware stack that:
//!
//! 1. Respects `Retry-After` headers on 429 responses (first priority).
//! 2. Falls back to exponential backoff for transient 5xx errors and network failures.
//!
//! See `docs/sync.md` — "Rate limits and backoff".

use std::time::Duration;

use anyhow::anyhow;
use axum::http::Extensions;
use reqwest::{Client, Request, Response, StatusCode};
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware, Error, Middleware, Next, Result};
use reqwest_retry::{
    DefaultRetryableStrategy, RetryTransientMiddleware, RetryableStrategy,
    policies::ExponentialBackoff,
};
use tracing::debug;

/// Build a rate-limit-aware `reqwest` client.
///
/// The client applies two layers of retry logic:
///
/// - **`Retry-After` middleware** (outer): intercepts 429 responses, reads the
///   `Retry-After` header (seconds or HTTP-date), and sleeps exactly that long
///   before allowing the inner stack to re-send.  If the header is absent, it
///   falls back to 1 second.
/// - **Exponential backoff middleware** (inner): retries transient 5xx errors
///   and network failures with jittered exponential backoff, bounded between
///   1 s and 60 s.
///
/// # Arguments
///
/// * `base` — a pre-configured `reqwest::Client` (TLS, timeouts, user-agent, …).
/// * `max_retries` — maximum number of retries across both layers.
pub fn build_rate_limited_client(base: Client, max_retries: u32) -> ClientWithMiddleware {
    let backoff_policy = ExponentialBackoff::builder()
        .retry_bounds(Duration::from_secs(1), Duration::from_secs(60))
        .build_with_max_retries(max_retries);

    ClientBuilder::new(base)
        // Outer: honour Retry-After on 429 before the backoff layer sees it.
        .with(RetryAfterMiddleware)
        // Inner: exponential backoff for transient errors (5xx, network failures).
        // We skip 429 here so RetryAfterMiddleware owns that status code.
        .with(RetryTransientMiddleware::new_with_policy_and_strategy(
            backoff_policy,
            No429Strategy,
        ))
        .build()
}

// ---------------------------------------------------------------------------
// Retry-After middleware
// ---------------------------------------------------------------------------

/// Middleware that intercepts HTTP 429 responses and sleeps for the duration
/// specified in the `Retry-After` response header before retrying.
///
/// The `Retry-After` value is interpreted as integer seconds.  HTTP-date format
/// is not currently supported — if the header is absent or unparseable, a 1-second
/// fallback delay is used.
struct RetryAfterMiddleware;

#[async_trait::async_trait]
impl Middleware for RetryAfterMiddleware {
    async fn handle(
        &self,
        req: Request,
        extensions: &mut Extensions,
        next: Next<'_>,
    ) -> Result<Response> {
        let duplicate = req.try_clone().ok_or_else(|| {
            Error::Middleware(anyhow!(
                "Request object is not clonable. Are you passing a streaming body?"
            ))
        })?;

        let response = next.clone().run(duplicate, extensions).await?;

        if response.status() != StatusCode::TOO_MANY_REQUESTS {
            return Ok(response);
        }

        // Parse Retry-After header (integer seconds).
        let wait = response
            .headers()
            .get("Retry-After")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(1);

        debug!(
            wait_secs = wait,
            "429 Too Many Requests — sleeping before retry"
        );

        tokio::time::sleep(Duration::from_secs(wait)).await;

        // Re-clone the original request for the retry.
        let retry_req = req.try_clone().ok_or_else(|| {
            Error::Middleware(anyhow!("Request object is not clonable on retry attempt"))
        })?;

        next.run(retry_req, extensions).await
    }
}

// ---------------------------------------------------------------------------
// Exponential-backoff strategy that skips 429 (owned by RetryAfterMiddleware)
// ---------------------------------------------------------------------------

/// A [`RetryableStrategy`] that delegates to the default strategy but treats
/// 429 as non-retryable so the `RetryAfterMiddleware` outer layer owns it.
struct No429Strategy;

impl RetryableStrategy for No429Strategy {
    fn handle(
        &self,
        res: &std::result::Result<reqwest::Response, reqwest_middleware::Error>,
    ) -> Option<reqwest_retry::Retryable> {
        match res {
            Ok(r) if r.status() == StatusCode::TOO_MANY_REQUESTS => {
                // Let RetryAfterMiddleware handle 429; don't retry here.
                None
            }
            other => DefaultRetryableStrategy.handle(other),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    #[tokio::test]
    async fn honours_retry_after_seconds() {
        let server = wiremock::MockServer::start().await;
        let attempts = Arc::new(AtomicU32::new(0));
        {
            let attempts = attempts.clone();
            wiremock::Mock::given(wiremock::matchers::method("GET"))
                .and(wiremock::matchers::path("/x"))
                .respond_with(move |_: &wiremock::Request| {
                    let n = attempts.fetch_add(1, Ordering::SeqCst);
                    if n == 0 {
                        wiremock::ResponseTemplate::new(429).insert_header("Retry-After", "1")
                    } else {
                        wiremock::ResponseTemplate::new(200).set_body_string("ok")
                    }
                })
                .mount(&server)
                .await;
        }

        let client = build_rate_limited_client(reqwest::Client::new(), 3);
        let start = std::time::Instant::now();
        let resp = client
            .get(format!("{}/x", server.uri()))
            .send()
            .await
            .unwrap();
        let elapsed = start.elapsed();

        assert_eq!(resp.status(), 200);
        // Must have waited at least 1 second for Retry-After.
        assert!(
            elapsed >= Duration::from_secs(1),
            "expected ≥1 s elapsed, got {elapsed:?}"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 2, "expected 2 attempts");
    }

    #[tokio::test]
    async fn fails_after_max_retries_on_5xx() {
        let server = wiremock::MockServer::start().await;
        let attempts = Arc::new(AtomicU32::new(0));
        {
            let attempts = attempts.clone();
            wiremock::Mock::given(wiremock::matchers::method("GET"))
                .and(wiremock::matchers::path("/y"))
                .respond_with(move |_: &wiremock::Request| {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    wiremock::ResponseTemplate::new(503)
                })
                .mount(&server)
                .await;
        }

        let max_retries: u32 = 2;
        let client = build_rate_limited_client(reqwest::Client::new(), max_retries);
        let resp = client
            .get(format!("{}/y", server.uri()))
            .send()
            .await
            .unwrap(); // The middleware returns a Response, not an Err, for HTTP errors.

        // After exhausting retries the final response (503) is returned.
        assert_eq!(resp.status(), 503);
        // 1 initial attempt + max_retries retries.
        let total = attempts.load(Ordering::SeqCst);
        assert_eq!(
            total,
            max_retries + 1,
            "expected {} attempts, got {total}",
            max_retries + 1
        );
    }
}
