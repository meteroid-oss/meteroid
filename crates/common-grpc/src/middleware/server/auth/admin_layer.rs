use std::task::{Context, Poll};

use futures_util::future;
use futures_util::future::Ready;
use hmac::{Hmac, Mac};
use hyper::{Request, Response};
use secrecy::{ExposeSecret, SecretString};
use sha2::Sha256;
use tonic::Status;
use tower::{Layer, Service};

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

    #[must_use]
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

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;
impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for AdminAuthService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>, Error = BoxError>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = future::Either<S::Future, Ready<Result<Self::Response, Self::Error>>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<ReqBody>) -> Self::Future {
        if !self.filter.map_or(true, |f| f(request.uri().path())) {
            return future::Either::Left(self.inner.call(request));
        }

        let mut request = request;
        let path = request.uri().path().to_string().clone();
        let headers = request.headers_mut();

        let signature_header = match headers.get(HMAC_SIGNATURE_HEADER) {
            Some(signature_header) => signature_header,
            None => {
                let error = Status::unauthenticated("Missing hmac signature");
                return future::Either::Right(future::ready(
                    Err(error).map_err(|e| BoxError::from(e) as BoxError),
                ));
            }
        };

        let signature_str = match signature_header.to_str() {
            Ok(signature_str) => signature_str,
            Err(_) => {
                let error = Status::unauthenticated("Invalid HMAC signature");
                return future::Either::Right(future::ready(
                    Err(error).map_err(|e| BoxError::from(e) as BoxError),
                ));
            }
        };

        let timestamp_header = match headers.get(HMAC_TIMESTAMP_HEADER) {
            Some(timestamp_header) => timestamp_header,
            None => {
                let error = Status::unauthenticated("Missing hmac timestamp");
                return future::Either::Right(future::ready(
                    Err(error).map_err(|e| BoxError::from(e) as BoxError),
                ));
            }
        };

        let timestamp_str = match timestamp_header.to_str() {
            Ok(timestamp_str) => timestamp_str,
            Err(_) => {
                let error = Status::unauthenticated("Invalid HMAC timestamp");
                return future::Either::Right(future::ready(
                    Err(error).map_err(|e| BoxError::from(e) as BoxError),
                ));
            }
        };

        // header was made with : let timestamp = format!("{}", chrono::Utc::now().timestamp());
        // let's validate that it's not too old
        let timestamp = timestamp_str.parse::<i64>().unwrap();
        let now = chrono::Utc::now().timestamp();

        if now - timestamp > 60 {
            return future::Either::Right(future::ready(
                Err(Status::permission_denied("HMAC signature is too old"))
                    .map_err(|e| BoxError::from(e) as BoxError),
            ));
        }

        let mut mac =
            match Hmac::<Sha256>::new_from_slice(self.hmac_secret.expose_secret().as_bytes()) {
                Ok(mac) => mac,
                Err(_) => {
                    let error = Status::unauthenticated("Invalid hmac secret");
                    return future::Either::Right(future::ready(
                        Err(error).map_err(|e| BoxError::from(e) as BoxError),
                    ));
                }
            };

        mac.update(timestamp_str.as_bytes());
        mac.update(path.as_bytes());

        let expected = hex::encode(mac.finalize().into_bytes());

        if *signature_str != expected {
            return future::Either::Right(future::ready(
                Err(Status::permission_denied(
                    "HMAC signature didn't pass validation",
                ))
                .map_err(|e| BoxError::from(e) as BoxError),
            ));
        }

        future::Either::Left(self.inner.call(request))
    }
}
