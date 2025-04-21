use crate::middleware::common::auth::{HMAC_SIGNATURE_HEADER, HMAC_TIMESTAMP_HEADER};
use common_config::auth::InternalAuthConfig;

use hmac::{Hmac, Mac};
use secrecy::{ExposeSecret, SecretString};
use sha2::Sha256;
use std::task::{Context, Poll};
use tower::{BoxError, Service};

use http::HeaderValue;
use hyper::Request;

use tonic::body::BoxBody;
use tower::Layer;
use tracing::log;

#[derive(Debug, Clone)]
pub struct AdminAuthLayer {
    hmac_secret: SecretString,
}

impl AdminAuthLayer {
    #[allow(clippy::new_without_default)]
    pub fn new(config: &InternalAuthConfig) -> Self {
        AdminAuthLayer {
            hmac_secret: config.hmac_secret.clone(),
        }
    }
}

impl<S> Layer<S> for AdminAuthLayer {
    type Service = AdminAuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AdminAuthService {
            inner,
            hmac_secret: self.hmac_secret.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AdminAuthService<S> {
    inner: S,
    hmac_secret: SecretString,
}

impl<S> Service<Request<BoxBody>> for AdminAuthService<S>
where
    S: Service<Request<BoxBody>>,
    S::Error: Into<BoxError>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<BoxBody>) -> Self::Future {
        let mut request = request;
        let path = request.uri().path().to_string().clone();
        let headers = request.headers_mut();

        if let Ok(mut mac) =
            Hmac::<Sha256>::new_from_slice(self.hmac_secret.expose_secret().as_bytes())
        {
            let timestamp = format!("{}", chrono::Utc::now().timestamp());
            mac.update(timestamp.as_bytes());
            mac.update(path.as_bytes());
            let signature = hex::encode(mac.finalize().into_bytes());
            headers.insert(
                HMAC_SIGNATURE_HEADER,
                HeaderValue::from_str(&signature).unwrap(),
            );
            headers.insert(
                HMAC_TIMESTAMP_HEADER,
                HeaderValue::from_str(&timestamp).unwrap(),
            );
        } else {
            log::error!("Failed to create hmac")
        }

        self.inner.call(request)
    }
}
