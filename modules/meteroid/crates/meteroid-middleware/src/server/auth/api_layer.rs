use hyper::{Request, Response};

use secrecy::SecretString;

use crate::server::auth::strategies::api_key_strategy::validate_api_key;
use crate::server::auth::strategies::jwt_strategy::{authorize_user, validate_jwt};
use crate::server::auth::strategies::portal_jwt_strategy::{authorize_portal, validate_portal_jwt};
use common_grpc::GrpcServiceMethod;
use common_grpc::middleware::common::auth::{
    API_KEY_HEADER, BEARER_AUTH_HEADER, PORTAL_KEY_HEADER,
};
use common_grpc::middleware::common::filters::Filter;
use common_grpc::middleware::server::AuthorizedState;
use common_grpc::middleware::server::auth::{AuthenticatedState, AuthorizedAsTenant};
use futures_util::TryFutureExt;
use http::StatusCode;
use meteroid_store::Store;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tonic::Status;
use tonic::body::{BoxBody, empty_body};
use tower::Service;
use tower_layer::Layer;
use tracing::log;

#[derive(Clone)]
pub struct ApiAuthMiddleware<S> {
    inner: S,
    filter: Option<Filter>,
    jwt_secret: SecretString,
    store: Store,
}

#[derive(Clone)]
pub struct ApiAuthLayer {
    jwt_secret: SecretString,
    store: Store,
    filter: Option<Filter>,
}

impl ApiAuthLayer {
    #[allow(clippy::new_without_default)]
    pub fn new(jwt_secret: SecretString, store: Store) -> Self {
        ApiAuthLayer {
            jwt_secret,
            filter: None,
            store,
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
            store: self.store.clone(),
            jwt_secret: self.jwt_secret.clone(),
        }
    }
}

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

// services that don't require authentication
const ANONYMOUS_SERVICES: [&str; 6] = [
    "/meteroid.api.instance.v1.InstanceService/GetInstance",
    "/meteroid.api.users.v1.UsersService/InitRegistration",
    "/meteroid.api.users.v1.UsersService/CompleteRegistration",
    "/meteroid.api.users.v1.UsersService/InitResetPassword",
    "/meteroid.api.users.v1.UsersService/ResetPassword",
    "/meteroid.api.users.v1.UsersService/Login",
];

// services require authentication but no authorization (no organization/tenant)
const UNAUTHORIZED_SERVICES: [&str; 5] = [
    "/meteroid.api.organizations.v1.OrganizationsService/ListOrganizations",
    "/meteroid.api.organizations.v1.OrganizationsService/CreateOrganization",
    "/meteroid.api.users.v1.UsersService/Me",
    "/meteroid.api.users.v1.UsersService/OnboardMe",
    "/meteroid.api.instance.v1.InstanceService/GetCountries",
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
        if !self.filter.is_none_or(|f| f(request.uri().path())) {
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

        let metadata = request.headers().clone();

        let store = self.store.clone();
        let jwt_secret = self.jwt_secret.clone();

        let future = async move {
            let authenticated_state = if metadata.contains_key(API_KEY_HEADER) {
                validate_api_key(&metadata, &store, &sm)
                    .await
                    .map_err(|e| BoxError::from(e) as BoxError)
            } else if metadata.contains_key(PORTAL_KEY_HEADER) {
                validate_portal_jwt(&metadata, jwt_secret)
                    .map_err(|e| BoxError::from(e) as BoxError)
            } else if metadata.contains_key(BEARER_AUTH_HEADER) {
                validate_jwt(&metadata, jwt_secret).map_err(|e| BoxError::from(e) as BoxError)
            } else {
                Err(Box::new(Status::unauthenticated("No authentication provided")) as BoxError)
            }?;

            let authorized_state = match authenticated_state {
                AuthenticatedState::ApiKey {
                    tenant_id,
                    id,
                    organization_id,
                } => Ok(AuthorizedState::Tenant(AuthorizedAsTenant {
                    tenant_id,
                    organization_id,
                    actor_id: id,
                })),
                AuthenticatedState::User { id } => {
                    if UNAUTHORIZED_SERVICES.contains(&request.uri().path()) {
                        Ok(AuthorizedState::User { user_id: id })
                    } else {
                        authorize_user(&metadata, id, store, sm)
                            .await
                            .map_err(|e| BoxError::from(e) as BoxError)
                    }
                }
                AuthenticatedState::Shared {
                    resource_access,
                    tenant_id,
                } => authorize_portal(tenant_id, resource_access, sm)
                    .await
                    .map_err(|e| BoxError::from(e) as BoxError),
            }?;

            request.extensions_mut().insert(authorized_state);

            inner.call(request).await
        };

        // if the future is an error , we recover by providing an empty REsponse
        let future = future.or_else(|e: BoxError| async move {
            log::warn!("Error in auth middleware: {:?}", e);
            let response = Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(empty_body())
                .unwrap();
            Ok(response)
        });

        Box::pin(future)
    }
}
