use std::task::{Context, Poll};

use crate::middleware::common::auth::API_KEY_HEADER;

use http::{HeaderValue, Request};

use tonic::body::BoxBody;

use tower::{Layer, Service};
use tracing::log;

#[derive(Debug, Clone)]
pub struct ApiAuthLayer {
    api_key: String,
}

impl ApiAuthLayer {
    #[allow(clippy::new_without_default)]
    pub fn new(api_key: String) -> Self {
        ApiAuthLayer { api_key }
    }
}

impl<S> Layer<S> for ApiAuthLayer {
    type Service = ApiAuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ApiAuthService {
            inner,
            api_key: self.api_key.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApiAuthService<S> {
    inner: S,
    api_key: String,
}

impl<S> Service<Request<BoxBody>> for ApiAuthService<S>
where
    S: Service<Request<BoxBody>> + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<BoxBody>) -> Self::Future {
        let mut request = request;
        let headers = request.headers_mut();

        if let Ok(api_key) = HeaderValue::from_str(&self.api_key) {
            headers.insert(API_KEY_HEADER, api_key);
        } else {
            log::error!("Failed to parse API key");
        }

        self.inner.call(request)
    }
}
