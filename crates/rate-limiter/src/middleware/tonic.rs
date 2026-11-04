//! Tonic/gRPC middleware for rate limiting
//!
//! Enable with the `tonic` feature flag.

use crate::{RateLimitResult, RateLimiter};
use futures_util::future::BoxFuture;
use http::{Request, Response};
use std::sync::Arc;
use std::task::{Context, Poll};
use tonic::body::Body;
use tower::layer::Layer;
use tower::{BoxError, Service};

pub type Filter = fn(&str) -> bool;

/// Key extractor: given a request, returns the rate-limit key and the
/// pre-configured [`RateLimiter`] to use.  Returning `None` skips rate
/// limiting for that request.
pub type KeyExtractor<ReqBody, K> =
    Arc<dyn Fn(&Request<ReqBody>) -> Option<(K, RateLimiter)> + Send + Sync>;

pub struct RateLimitMiddleware<S, ReqBody, K> {
    inner: S,
    filter: Option<Filter>,
    key_extractor: KeyExtractor<ReqBody, K>,
}

impl<S, ReqBody, K> Clone for RateLimitMiddleware<S, ReqBody, K>
where
    S: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            filter: self.filter,
            key_extractor: self.key_extractor.clone(),
        }
    }
}

pub struct RateLimitLayer<ReqBody, K> {
    filter: Option<Filter>,
    key_extractor: KeyExtractor<ReqBody, K>,
}

impl<ReqBody, K> RateLimitLayer<ReqBody, K>
where
    ReqBody: 'static,
    K: ToString + Send + Sync + 'static,
{
    pub fn new<F>(key_extractor: F) -> Self
    where
        F: Fn(&Request<ReqBody>) -> Option<(K, RateLimiter)> + Send + Sync + 'static,
    {
        Self {
            filter: None,
            key_extractor: Arc::new(key_extractor),
        }
    }

    #[must_use]
    pub fn filter(mut self, filter: Filter) -> Self {
        self.filter = Some(filter);
        self
    }
}

impl<ReqBody, K> Clone for RateLimitLayer<ReqBody, K> {
    fn clone(&self) -> Self {
        Self {
            filter: self.filter,
            key_extractor: self.key_extractor.clone(),
        }
    }
}

impl<S, ReqBody, K> Layer<S> for RateLimitLayer<ReqBody, K> {
    type Service = RateLimitMiddleware<S, ReqBody, K>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitMiddleware {
            inner,
            filter: self.filter,
            key_extractor: self.key_extractor.clone(),
        }
    }
}

impl<S, ReqBody, K> Service<Request<ReqBody>> for RateLimitMiddleware<S, ReqBody, K>
where
    S: Service<Request<ReqBody>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Into<BoxError>,
    ReqBody: Send + 'static,
    K: ToString + Send + Sync + 'static,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, request: Request<ReqBody>) -> Self::Future {
        // Skip if filter doesn't match
        if !self.filter.is_none_or(|f| f(request.uri().path())) {
            let mut inner = self.inner.clone();
            return Box::pin(async move { inner.call(request).await.map_err(Into::into) });
        }

        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let key_extractor = self.key_extractor.clone();

        Box::pin(async move {
            let extracted = (key_extractor)(&request);

            if let Some((key, limiter)) = extracted {
                // Skip if rate limiting is disabled
                if !limiter.is_enabled() {
                    return inner.call(request).await.map_err(Into::into);
                }

                let key_str = key.to_string();
                let result = limiter.check(&key_str).await;

                match result {
                    RateLimitResult::Allowed {
                        limit,
                        remaining,
                        reset,
                    } => {
                        let mut response = inner.call(request).await.map_err(Into::into)?;
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
                        limit: _,
                        reset: _,
                        retry_after,
                    } => {
                        tracing::warn!(
                            "Rate limit exceeded for key: {}. Retry after {} seconds",
                            key_str,
                            retry_after
                        );
                        let status = tonic::Status::resource_exhausted(format!(
                            "Rate limit exceeded. Retry after {retry_after} seconds"
                        ));
                        Ok(status.into_http())
                    }
                }
            } else {
                tracing::debug!("No rate limit key in request, skipping rate limiting");
                inner.call(request).await.map_err(Into::into)
            }
        })
    }
}
