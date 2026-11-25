//! Axum middleware for rate limiting
//!
//! Enable with the `axum` feature flag.
//!
//! Every response it lets through or refuses carries the bucket's state twice: as
//! `RateLimit-Limit`, `RateLimit-Remaining` and `RateLimit-Reset` (the bucket's capacity, what is
//! left of it, and the Unix time it is full again), and as the IETF httpapi draft's structured
//! fields, `RateLimit-Policy: "<policy>";q=<capacity>;w=<window seconds>` and
//! `RateLimit: "<policy>";r=<remaining>;t=<seconds until full>`. A refusal adds `Retry-After`
//! (seconds).

use crate::{RateLimitResult, RateLimiter};
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use http::{HeaderMap, HeaderValue, Request};
use serde_json::json;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

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
                insert_state(response.headers_mut(), &limiter, limit, remaining, reset);
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
                let mut response = (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(json!({
                        "code": "TOO_MANY_REQUESTS",
                        "message": format!("Rate limit exceeded. Retry after {retry_after} seconds")
                    })),
                )
                    .into_response();
                let headers = response.headers_mut();
                insert_state(headers, &limiter, limit, 0, reset);
                headers.insert(http::header::RETRY_AFTER, HeaderValue::from(retry_after));
                Err(response)
            }
        }
    } else {
        tracing::debug!("No rate limit key in request, skipping rate limiting");
        Ok(next.run(req).await)
    }
}

/// The name a bucket's state is given in `RateLimit-Policy` and `RateLimit`: its bucket, else
/// `default`.
fn policy_name(limiter: &RateLimiter) -> &str {
    limiter.config().bucket.as_deref().unwrap_or("default")
}

/// The bucket's state, in the legacy `RateLimit-*` triplet and in the draft's `RateLimit-Policy`
/// and `RateLimit` structured fields.
fn insert_state(
    headers: &mut HeaderMap,
    limiter: &RateLimiter,
    limit: u64,
    remaining: u64,
    reset: u64,
) {
    let now = UNIX_EPOCH.elapsed().unwrap_or_default().as_secs();
    headers.insert("RateLimit-Limit", HeaderValue::from(limit));
    headers.insert("RateLimit-Remaining", HeaderValue::from(remaining));
    headers.insert("RateLimit-Reset", HeaderValue::from(reset));
    let (policy, state) = structured_fields(
        policy_name(limiter),
        limit,
        limiter.config().window_seconds,
        remaining,
        reset.saturating_sub(now),
    );
    if let (Ok(policy), Ok(state)) = (
        HeaderValue::from_str(&policy),
        HeaderValue::from_str(&state),
    ) {
        headers.insert("RateLimit-Policy", policy);
        headers.insert("RateLimit", state);
    }
}

/// `RateLimit-Policy` and `RateLimit` for one policy: its quota over its window, and what is left
/// of it with the seconds until it is whole again.
fn structured_fields(
    name: &str,
    quota: u64,
    window: u64,
    remaining: u64,
    seconds_left: u64,
) -> (String, String) {
    let name: String = name
        .chars()
        .filter(|c| c.is_ascii_graphic() && !matches!(c, '"' | '\\'))
        .collect();
    let policy = if window > 0 {
        format!("\"{name}\";q={quota};w={window}")
    } else {
        format!("\"{name}\";q={quota}")
    };
    (policy, format!("\"{name}\";r={remaining};t={seconds_left}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_reads_as_the_drafts_structured_fields() {
        assert_eq!(
            structured_fields("rest:api", 100, 1, 42, 1),
            (
                "\"rest:api\";q=100;w=1".to_owned(),
                "\"rest:api\";r=42;t=1".to_owned()
            )
        );
        assert_eq!(
            structured_fields("de\"fault", 10, 0, 0, 7).0,
            "\"default\";q=10"
        );
    }
}
