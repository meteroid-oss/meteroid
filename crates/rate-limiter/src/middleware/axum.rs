//! Axum middleware for rate limiting
//!
//! Enable with the `axum` feature flag.

use crate::{RateLimitResult, RateLimiter};
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use http::Request;
use serde_json::json;
use std::sync::Arc;

/// Key extractor: given a request, returns the rate-limit key and the
/// pre-configured [`RateLimiter`] to use (which already carries its
/// [`RateLimitConfig`]).  Returning `None` skips rate limiting for that
/// request.
pub type KeyExtractor<K> =
    Arc<dyn Fn(&Request<axum::body::Body>) -> Option<(K, RateLimiter)> + Send + Sync>;

/// Axum middleware state for rate limiting.
#[derive(Clone)]
pub struct RateLimitState<K> {
    pub filter: Option<fn(&str) -> bool>,
    pub key_extractor: KeyExtractor<K>,
}

impl<K> RateLimitState<K>
where
    K: ToString + Send + Sync + 'static,
{
    pub fn new<F>(key_extractor: F) -> Self
    where
        F: Fn(&Request<axum::body::Body>) -> Option<(K, RateLimiter)> + Send + Sync + 'static,
    {
        Self {
            filter: None,
            key_extractor: Arc::new(key_extractor),
        }
    }

    pub fn with_filter(mut self, filter: fn(&str) -> bool) -> Self {
        self.filter = Some(filter);
        self
    }
}

/// Axum middleware for rate limiting REST API requests.
pub async fn rate_limit_middleware<K>(
    State(state): State<RateLimitState<K>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, Response>
where
    K: ToString + Send + Sync + 'static,
{
    let path = req.uri().path();

    // Skip if path filter doesn't match
    if let Some(filter) = state.filter
        && !filter(path)
    {
        return Ok(next.run(req).await);
    }

    let extracted = (state.key_extractor)(&req);

    if let Some((key, limiter)) = extracted {
        // Skip if rate limiting is disabled
        if !limiter.is_enabled() {
            return Ok(next.run(req).await);
        }

        let key_str = key.to_string();
        let result = limiter.check(&key_str).await;

        match result {
            RateLimitResult::Allowed {
                limit,
                remaining,
                reset,
            } => {
                let mut response = next.run(req).await;
                let headers = response.headers_mut();
                headers.insert("RateLimit-Limit", limit.to_string().parse().unwrap());
                headers.insert(
                    "RateLimit-Remaining",
                    remaining.to_string().parse().unwrap(),
                );
                headers.insert("RateLimit-Reset", reset.to_string().parse().unwrap());
                Ok(response)
            }
            RateLimitResult::RateLimited {
                limit,
                reset,
                retry_after,
            } => {
                tracing::warn!(
                    "Rate limit exceeded for key: {}. Retry after {} seconds",
                    key_str,
                    retry_after
                );
                Err((
                    StatusCode::TOO_MANY_REQUESTS,
                    [
                        ("RateLimit-Limit", limit.to_string()),
                        ("RateLimit-Remaining", "0".to_string()),
                        ("RateLimit-Reset", reset.to_string()),
                        ("Retry-After", retry_after.to_string()),
                    ],
                    Json(json!({
                        "code": "TOO_MANY_REQUESTS",
                        "message": format!("Rate limit exceeded. Retry after {retry_after} seconds")
                    })),
                )
                    .into_response())
            }
        }
    } else {
        tracing::debug!("No rate limit key in request, skipping rate limiting");
        Ok(next.run(req).await)
    }
}
