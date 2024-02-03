use crate::middleware::common::auth::{API_KEY_HEADER, BEARER_AUTH_HEADER};
use crate::middleware::server::auth::strategies::api_key_strategy::validate_api_key;
use crate::middleware::server::auth::strategies::{AuthenticatedState, AuthorizedState};
use crate::GrpcServiceMethod;
use common_repository::Pool;

use hyper::{Request, Response};

use secrecy::SecretString;

use futures_util::TryFutureExt;
use http::StatusCode;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tonic::body::{empty_body, BoxBody};
use tonic::Status;
use tower::Service;
use tower_layer::Layer;
use tracing::log;

use crate::middleware::common::filters::Filter;
use crate::middleware::server::auth::strategies::jwt_strategy::{authorize_user, validate_jwt};

#[derive(Debug, Clone)]
pub struct ApiAuthMiddleware<S> {
    inner: S,
    filter: Option<Filter>,
    jwt_secret: SecretString,
    pool: Pool,
}

#[derive(Debug, Clone)]
pub struct ApiAuthLayer {
    jwt_secret: SecretString,
    pool: Pool,
    filter: Option<Filter>,
}

impl ApiAuthLayer {
    #[allow(clippy::new_without_default)]
    pub fn new(jwt_secret: SecretString, pool: Pool) -> Self {
        ApiAuthLayer {
            jwt_secret,
            filter: None,
            pool,
        }
    }

    #[must_use]
    pub fn filter(mut self, filter: Filter) -> Self {
        self.filter = Some(filter);
        self
    }
}

impl<S> Layer<S> for ApiAuthLayer {
    type Service = ApiAuthMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ApiAuthMiddleware {
            inner,
            filter: self.filter,
            pool: self.pool.clone(),
            jwt_secret: self.jwt_secret.clone(),
        }
    }
}

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

const ANONYMOUS_SERVICES: [&str; 3] = [
    "/meteroid.api.instance.v1.InstanceService/GetInstance",
    "/meteroid.api.users.v1.UsersService/Register",
    "/meteroid.api.users.v1.UsersService/Login",
];

impl<S, ReqBody> Service<Request<ReqBody>> for ApiAuthMiddleware<S>
where
    S: Service<Request<ReqBody>, Response = Response<BoxBody>, Error = BoxError>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: Request<ReqBody>) -> Self::Future {
        if !self.filter.map_or(true, |f| f(request.uri().path())) {
            return Box::pin(self.inner.call(request));
        }
        if ANONYMOUS_SERVICES.contains(&request.uri().path()) {
            return Box::pin(self.inner.call(request));
        }

        // This is necessary because tonic internally uses `tower::buffer::Buffer`.
        // See https://github.com/tower-rs/tower/issues/547#issuecomment-767629149
        // for details on why this is necessary
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        let sm = GrpcServiceMethod::extract(request.uri());

        let mut metadata = request.headers_mut().clone();

        let pool = self.pool.clone();
        let jwt_secret = self.jwt_secret.clone();

        let future = async move {
            let authenticated_state = if metadata.contains_key(API_KEY_HEADER) {
                validate_api_key(&mut metadata, &pool, &sm)
                    .await
                    .map_err(|e| BoxError::from(e) as BoxError)
            } else if metadata.contains_key(BEARER_AUTH_HEADER) {
                validate_jwt(&mut metadata, jwt_secret).map_err(|e| BoxError::from(e) as BoxError)
            } else {
                Err(Box::new(Status::unauthenticated("No authentication provided")) as BoxError)
            }?;

            let authorized_state = match authenticated_state {
                AuthenticatedState::ApiKey { tenant_id, id } => Ok(AuthorizedState::Tenant {
                    tenant_id,
                    actor_id: id,
                }),
                AuthenticatedState::User { id } => authorize_user(&metadata, id, &pool, sm)
                    .await
                    .map_err(|e| BoxError::from(e) as BoxError),
            }?;

            request.extensions_mut().insert(authorized_state);

            inner.call(request).await.map_err(Into::into)
        };

        // if the future is an error , we recover by providing an empty REsponse
        let future = future.or_else(|e: BoxError| async move {
            log::warn!("Error in auth middleware: {}", e);
            let response = Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(empty_body())
                .unwrap();
            Ok(response)
        });

        Box::pin(future)
    }
}
