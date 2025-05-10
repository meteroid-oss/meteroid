use governor::{DefaultKeyedRateLimiter, Jitter, Quota, RateLimiter};
use http::Extensions;
use reqwest::{Request, Response};
use reqwest_middleware::{Middleware, Next};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

pub struct RateLimitMiddleware {
    rate_limiter: Arc<DefaultKeyedRateLimiter<String>>,
    jitter: Jitter,
}

impl RateLimitMiddleware {
    pub fn new(rps: NonZeroU32) -> Self {
        let quota = Quota::per_second(rps);
        let rate_limiter: Arc<DefaultKeyedRateLimiter<String>> =
            Arc::new(RateLimiter::keyed(quota));
        Self::from_rate_limiter(rate_limiter)
    }

    pub fn from_rate_limiter(rate_limiter: Arc<DefaultKeyedRateLimiter<String>>) -> Self {
        let jitter = Jitter::up_to(Duration::from_secs(1));
        Self {
            rate_limiter,
            jitter,
        }
    }

    fn extract_access_token(&self, req: &Request) -> Option<String> {
        req.headers()
            .get(http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|bearer| bearer.strip_prefix("Bearer "))
            .map(|token| token.to_string())
    }
}

#[async_trait::async_trait]
impl Middleware for RateLimitMiddleware {
    async fn handle(
        &self,
        req: Request,
        extensions: &mut Extensions,
        next: Next<'_>,
    ) -> reqwest_middleware::Result<Response> {
        if let Some(access_token) = self.extract_access_token(&req) {
            self.rate_limiter
                .until_key_ready_with_jitter(&access_token, self.jitter)
                .await;
        }

        next.run(req, extensions).await
    }
}
