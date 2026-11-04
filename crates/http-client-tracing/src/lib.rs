//! Client spans for outbound HTTP calls, so a provider's latency and failures show in the trace
//! of the request or unit of work that made the call.
//!
//! Each attempt is its own span, named `{method} {host}` (`POST api.stripe.com`) and carrying the
//! method, host, path and response status. The query string is never recorded. A call made
//! outside any span starts no trace (see `common_logging::otel`).

use reqwest::{Request, RequestBuilder, Response, StatusCode};
use tracing::{Instrument, Span};

#[cfg(feature = "middleware")]
mod middleware;
#[cfg(feature = "middleware")]
pub use middleware::ClientSpanMiddleware;

/// The way to build a `reqwest` client (clippy bans the others): reqwest is compiled without a
/// TLS crypto provider and panics on `build()` until one is installed.
#[allow(clippy::disallowed_methods)]
pub fn client_builder() -> reqwest::ClientBuilder {
    // Fails only when already installed.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
}

/// [`reqwest::Client::new`], built through [`client_builder`].
pub fn client() -> reqwest::Client {
    client_builder()
        .build()
        .expect("the TLS backend initializes")
}

/// Sends a plain `reqwest` request inside a client span.
pub async fn send(request: RequestBuilder) -> reqwest::Result<Response> {
    let (client, request) = request.build_split();
    let request = request?;
    let span = client_span(&request);
    let response = client.execute(request).instrument(span.clone()).await;
    record_outcome(&span, response.as_ref().map(Response::status).ok());
    response
}

fn client_span(request: &Request) -> Span {
    let method = request.method().as_str();
    let host = request.url().host_str().unwrap_or_default();
    tracing::info_span!(
        "http_client_request",
        otel.name = %format_args!("{method} {host}"),
        otel.kind = "client",
        otel.status_code = tracing::field::Empty,
        http.request.method = method,
        server.address = host,
        url.path = request.url().path(),
        http.response.status_code = tracing::field::Empty,
    )
}

/// A response with a 4xx or 5xx status, or none at all, marks the span as failed.
fn record_outcome(span: &Span, status: Option<StatusCode>) {
    if let Some(status) = status {
        span.record("http.response.status_code", status.as_u16());
    }
    if status.is_none_or(|s| s.is_client_error() || s.is_server_error()) {
        span.record("otel.status_code", "ERROR");
    }
}
