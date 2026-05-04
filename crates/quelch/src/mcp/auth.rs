//! MCP server authentication middleware.
//!
//! # API key auth
//!
//! When the `QUELCH_MCP_API_KEY` environment variable is set (or the `--api-key`
//! CLI flag overrides it), every request must carry:
//!
//! ```http
//! Authorization: Bearer <key>
//! ```
//!
//! If the key is absent or wrong the middleware returns `401 Unauthorized`.
//!
//! When `QUELCH_MCP_API_KEY` is **not** set, all requests are accepted (dev mode).
//!
//! # Entra (future)
//!
//! TODO(mcp-entra): token validation via `azure_identity` / MSAL.
//! Leave behind a `cfg(feature = "entra")` guard when implementing.

use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};

/// Axum middleware that enforces API key authentication when
/// `QUELCH_MCP_API_KEY` is set.
pub async fn api_key_middleware(req: Request, next: Next) -> Result<Response, StatusCode> {
    let expected = std::env::var("QUELCH_MCP_API_KEY").ok();

    if let Some(expected) = expected {
        let auth = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok());

        let provided = auth.and_then(|h| h.strip_prefix("Bearer "));

        if provided != Some(expected.as_str()) {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }

    // If QUELCH_MCP_API_KEY is not set: dev mode — accept all requests.
    Ok(next.run(req).await)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{Request, StatusCode};
    use axum::{Router, body::Body, middleware, routing::get};
    use std::sync::Mutex;
    use tower::ServiceExt;

    /// Serialise all env-mutating tests so they don't interfere with each other.
    /// The tokio test executor runs tests in parallel by default; a single global
    /// mutex ensures only one auth test touches the env at a time.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn app_with_auth() -> Router {
        Router::new()
            .route("/ping", get(|| async { "pong" }))
            .layer(middleware::from_fn(api_key_middleware))
    }

    async fn response_status(router: Router, key_header: Option<&str>) -> StatusCode {
        let mut builder = Request::builder().method("GET").uri("/ping");

        if let Some(k) = key_header {
            builder = builder.header("Authorization", k);
        }

        let req = builder.body(Body::empty()).unwrap();
        router.oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    // The guard must be held across the await to prevent another test from
    // changing QUELCH_MCP_API_KEY while the request is in-flight.
    #[allow(clippy::await_holding_lock)]
    async fn api_key_middleware_no_auth_required_when_env_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        let prev = std::env::var("QUELCH_MCP_API_KEY").ok();
        // SAFETY: protected by ENV_LOCK; no other thread modifies this var.
        unsafe { std::env::remove_var("QUELCH_MCP_API_KEY") };

        let status = response_status(app_with_auth(), None).await;
        assert_eq!(status, StatusCode::OK, "no env var → accept all");

        if let Some(v) = prev {
            unsafe { std::env::set_var("QUELCH_MCP_API_KEY", v) };
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn api_key_middleware_rejects_missing_header() {
        let _guard = ENV_LOCK.lock().unwrap();
        let prev = std::env::var("QUELCH_MCP_API_KEY").ok();
        unsafe { std::env::set_var("QUELCH_MCP_API_KEY", "secret123") };

        let status = response_status(app_with_auth(), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "missing header → 401");

        unsafe { std::env::remove_var("QUELCH_MCP_API_KEY") };
        if let Some(v) = prev {
            unsafe { std::env::set_var("QUELCH_MCP_API_KEY", v) };
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn api_key_middleware_rejects_wrong_value() {
        let _guard = ENV_LOCK.lock().unwrap();
        let prev = std::env::var("QUELCH_MCP_API_KEY").ok();
        unsafe { std::env::set_var("QUELCH_MCP_API_KEY", "secret123") };

        let status = response_status(app_with_auth(), Some("Bearer wrong-key")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "wrong key → 401");

        unsafe { std::env::remove_var("QUELCH_MCP_API_KEY") };
        if let Some(v) = prev {
            unsafe { std::env::set_var("QUELCH_MCP_API_KEY", v) };
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn api_key_middleware_passes_correct_value() {
        let _guard = ENV_LOCK.lock().unwrap();
        let prev = std::env::var("QUELCH_MCP_API_KEY").ok();
        unsafe { std::env::set_var("QUELCH_MCP_API_KEY", "secret123") };

        let status = response_status(app_with_auth(), Some("Bearer secret123")).await;
        assert_eq!(status, StatusCode::OK, "correct key → 200");

        unsafe { std::env::remove_var("QUELCH_MCP_API_KEY") };
        if let Some(v) = prev {
            unsafe { std::env::set_var("QUELCH_MCP_API_KEY", v) };
        }
    }
}
