use super::{client_span, record_outcome};
use http::Extensions;
use reqwest::{Request, Response};
use reqwest_middleware::{Middleware, Next};
use tracing::Instrument;

/// Client spans for `reqwest_middleware` clients. Add it after the retry middleware so each
/// attempt gets its own span.
pub struct ClientSpanMiddleware;

#[async_trait::async_trait]
impl Middleware for ClientSpanMiddleware {
    async fn handle(
        &self,
        req: Request,
        extensions: &mut Extensions,
        next: Next<'_>,
    ) -> reqwest_middleware::Result<Response> {
        let span = client_span(&req);
        let response = next.run(req, extensions).instrument(span.clone()).await;
        record_outcome(&span, response.as_ref().map(Response::status).ok());
        response
    }
}
