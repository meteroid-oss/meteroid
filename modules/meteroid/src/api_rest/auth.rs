use common_grpc::middleware::common::auth::API_KEY_HEADER;

use axum::http::StatusCode;
use axum::response::Response;
use cached::proc_macro::cached;
use common_grpc::middleware::server::auth::api_token_validator::ApiTokenValidator;
use futures::future::BoxFuture;
use http::{HeaderMap, Request};
use std::task::{Context, Poll};
use tower::Service;
use tower_layer::Layer;
use tracing::{error, log};

use common_grpc::middleware::common::filters::Filter;

use common_domain::ids::{OrganizationId, TenantId};
use common_grpc::middleware::server::auth::{AuthenticatedState, AuthorizedAsTenant};
use meteroid_store::Store;
use meteroid_store::repositories::api_tokens::ApiTokensInterface;
use uuid::Uuid;

#[allow(unused)]
#[derive(Debug)]
pub struct AuthStatus {
    status: StatusCode,
    msg: Option<String>,
}

#[derive(Clone)]
pub struct ApiAuthMiddleware<S> {
    inner: S,
    filter: Option<Filter>,
    store: Store,
}

#[derive(Clone)]
pub struct ExternalApiAuthLayer {
    store: Store,
    filter: Option<Filter>,
}

impl ExternalApiAuthLayer {
    #[allow(clippy::new_without_default)]
    pub fn new(store: Store) -> Self {
        ExternalApiAuthLayer {
            store,
            filter: None,
        }
    }

    #[must_use]
    pub fn filter(mut self, filter: Filter) -> Self {
        self.filter = Some(filter);
        self
    }
}

impl<S> Layer<S> for ExternalApiAuthLayer {
    type Service = ApiAuthMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ApiAuthMiddleware {
            inner,
            filter: self.filter,
            store: self.store.clone(),
        }
    }
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for ApiAuthMiddleware<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    ReqBody: Default + Send + 'static,
    ResBody: Default + Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: Request<ReqBody>) -> Self::Future {
        if !self.filter.is_none_or(|f| f(request.uri().path())) {
            return Box::pin(self.inner.call(request));
        }

        // This is necessary because axum internally uses `tower::buffer::Buffer`.
        // See https://github.com/tower-rs/tower/issues/547#issuecomment-767629149
        // for details on why this is necessary
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        let metadata = request.headers().clone();
        let mut store = self.store.clone();

        let future = async move {
            let authenticated_state = if metadata.contains_key(API_KEY_HEADER) {
                validate_api_key(&metadata, &mut store).await.map_err(|e| {
                    log::debug!("Failed to validate api key: {:?}", e);
                    Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .body(ResBody::default())
                })
            } else {
                Err(Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .body(ResBody::default()))
            };

            match authenticated_state {
                Ok(AuthenticatedState::ApiKey {
                    tenant_id,
                    id,
                    organization_id,
                }) => {
                    let state = AuthorizedAsTenant {
                        tenant_id,
                        organization_id,
                        actor_id: id,
                    };
                    request.extensions_mut().insert(state);
                    inner.call(request).await
                }
                Ok(_) => Ok(Response::builder()
                    .status(StatusCode::FORBIDDEN)
                    .body(ResBody::default())
                    .expect("Failed to build response")),
                Err(e) => Ok(e.expect("Failed to build response")),
            }
        };

        Box::pin(future)
    }
}

#[cached(
    result = true,
    size = 100,
    time = 120, // 2 min
    key = "Uuid",
    convert = r#"{ *api_key_id }"#
)]
async fn validate_api_token_by_id_cached(
    store: &mut Store,
    validator: &ApiTokenValidator,
    api_key_id: &Uuid,
) -> Result<(OrganizationId, TenantId), AuthStatus> {
    let res = store
        .get_api_token_by_id_for_validation(api_key_id)
        .await
        .map_err(|err| {
            error!("Failed to resolve api key: {:?}", err);
            AuthStatus {
                status: StatusCode::UNAUTHORIZED,
                msg: Some("Failed to resolve api key".to_string()),
            }
        })?;

    validator
        .validate_hash(&res.hash)
        .map_err(|_e| AuthStatus {
            status: StatusCode::UNAUTHORIZED,
            msg: Some("Unauthorized. Invalid hash".to_string()),
        })?;

    Ok((res.organization_id, res.tenant_id))
}

pub async fn validate_api_key(
    header_map: &HeaderMap,
    store: &mut Store,
) -> Result<AuthenticatedState, AuthStatus> {
    let api_key = header_map
        .get(API_KEY_HEADER)
        .ok_or(AuthStatus {
            status: StatusCode::UNAUTHORIZED,
            msg: Some("Missing API key".to_string()),
        })?
        .to_str()
        .map_err(|_| AuthStatus {
            status: StatusCode::UNAUTHORIZED,
            msg: Some("Invalid API key format".to_string()),
        })?;

    let validator = ApiTokenValidator::parse_api_key(api_key).map_err(|_| AuthStatus {
        status: StatusCode::UNAUTHORIZED,
        msg: Some("Invalid API key format".to_string()),
    })?;

    let id = validator.extract_identifier().map_err(|_| AuthStatus {
        status: StatusCode::UNAUTHORIZED,
        msg: Some("Invalid API key format. Failed to extract identifier".to_string()),
    })?;

    let (organization_id, tenant_id) =
        validate_api_token_by_id_cached(store, &validator, &id).await?;

    Ok(AuthenticatedState::ApiKey {
        id,
        tenant_id,
        organization_id,
    })
}
