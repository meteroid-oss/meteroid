use http::header::HeaderName;
use std::time::Duration;
use tower_http::cors::{AllowOrigin, CorsLayer};

const DEFAULT_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const DEFAULT_EXPOSED_HEADERS: [&str; 7] = [
    "grpc-status",
    "grpc-message",
    "grpc-status-details-bin",
    "retry-after",
    "ratelimit-limit",
    "ratelimit-remaining",
    "ratelimit-reset",
];
const DEFAULT_ALLOW_HEADERS: [&str; 9] = [
    "x-grpc-web",
    "content-type",
    "x-user-agent",
    "grpc-timeout",
    "x-api-key",
    "authorization",
    "x-md-context",
    "x-portal-token",
    "meteroid-version",
];

pub fn cors() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::mirror_request())
        .allow_credentials(true)
        .max_age(DEFAULT_MAX_AGE)
        .expose_headers(
            DEFAULT_EXPOSED_HEADERS
                .iter()
                .copied()
                .map(HeaderName::from_static)
                .collect::<Vec<HeaderName>>(),
        )
        .allow_headers(
            DEFAULT_ALLOW_HEADERS
                .iter()
                .copied()
                .map(HeaderName::from_static)
                .collect::<Vec<HeaderName>>(),
        )
}
