use std::task::{Context, Poll};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use hmac::{Hmac, Mac};
use hyper::{Request, Response};
use secrecy::{ExposeSecret, SecretString};
use sha2::Sha256;
use tonic::Status;
use tower::{BoxError, Layer, Service};

use common_config::auth::InternalAuthConfig;

use crate::middleware::common::auth::{HMAC_SIGNATURE_HEADER, HMAC_TIMESTAMP_HEADER};
use crate::middleware::common::filters::Filter;

#[derive(Debug, Clone)]
pub struct AdminAuthLayer {
    hmac_secret: SecretString,
    filter: Option<Filter>,
}

impl AdminAuthLayer {
    #[allow(clippy::new_without_default)]
    pub fn new(config: &InternalAuthConfig) -> Self {
        AdminAuthLayer {
            hmac_secret: config.hmac_secret.clone(),
            filter: None,
        }
    }

    pub fn filter(mut self, filter: Filter) -> Self {
        self.filter = Some(filter);
        self
    }
}

impl<S> Layer<S> for AdminAuthLayer {
    type Service = AdminAuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AdminAuthService {
            inner,
            hmac_secret: self.hmac_secret.clone(),
            filter: self.filter,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AdminAuthService<S> {
    inner: S,
    hmac_secret: SecretString,
    filter: Option<Filter>,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for AdminAuthService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Into<BoxError>,
    ReqBody: Send + 'static,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, mut request: Request<ReqBody>) -> Self::Future {
        // Fast-path: filtered out → just forward, mapping error to BoxError
        if !self.filter.is_none_or(|f| f(request.uri().path())) {
            let mut inner = self.inner.clone();
            return async move { inner.call(request).await.map_err(Into::into) }.boxed();
        }

        // Buffer workaround
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        let path = request.uri().path().to_string();
        let headers = request.headers_mut();

        // Helpers to make early error returns nice and typed
        let err = |status: Status| -> Result<Self::Response, BoxError> {
            Err(Box::new(status) as BoxError)
        };

        let signature_header = match headers.get(HMAC_SIGNATURE_HEADER) {
            Some(v) => v,
            None => {
                return async move { err(Status::unauthenticated("Missing hmac signature")) }
                    .boxed();
            }
        };

        let signature_str = match signature_header.to_str() {
            Ok(s) => s,
            Err(_) => {
                return async move { err(Status::unauthenticated("Invalid HMAC signature")) }
                    .boxed();
            }
        };

        let timestamp_header = match headers.get(HMAC_TIMESTAMP_HEADER) {
            Some(v) => v,
            None => {
                return async move { err(Status::unauthenticated("Missing hmac timestamp")) }
                    .boxed();
            }
        };

        let timestamp_str = match timestamp_header.to_str() {
            Ok(s) => s,
            Err(_) => {
                return async move { err(Status::unauthenticated("Invalid HMAC timestamp")) }
                    .boxed();
            }
        };

        // Avoid panic on bad parse
        let timestamp = match timestamp_str.parse::<i64>() {
            Ok(t) => t,
            Err(_) => {
                return async move { err(Status::unauthenticated("Invalid HMAC timestamp")) }
                    .boxed();
            }
        };

        let now = chrono::Utc::now().timestamp();
        if now - timestamp > 60 {
            return async move { err(Status::permission_denied("HMAC signature is too old")) }
                .boxed();
        }

        let mut mac =
            match Hmac::<Sha256>::new_from_slice(self.hmac_secret.expose_secret().as_bytes()) {
                Ok(mac) => mac,
                Err(_) => {
                    return async move { err(Status::unauthenticated("Invalid hmac secret")) }
                        .boxed();
                }
            };

        mac.update(timestamp_str.as_bytes());
        mac.update(path.as_bytes());
        let expected = hex::encode(mac.finalize().into_bytes());

        if signature_str != expected {
            return async move {
                err(Status::permission_denied(
                    "HMAC signature didn't pass validation",
                ))
            }
            .boxed();
        }

        // Success: forward to inner, mapping error to BoxError
        async move { inner.call(request).await.map_err(Into::into) }.boxed()
    }
}
