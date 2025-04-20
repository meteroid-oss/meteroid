use common_grpc::GrpcServiceMethod;
use common_grpc::middleware::common::auth::API_KEY_HEADER;

use cached::proc_macro::cached;
use common_grpc::middleware::client::LayeredClientService;
use common_grpc::middleware::server::auth::api_token_validator::ApiTokenValidator;
use futures::TryFutureExt;
use hyper::{HeaderMap, Request, Response, StatusCode};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tonic::Status;
use tonic::body::Body;
use tower::Service;
use tower_layer::Layer;
use tracing::{error, log};

use common_grpc::middleware::common::filters::Filter;

use common_domain::ids::{OrganizationId, TenantId};
use common_grpc::middleware::server::auth::{
    AuthenticatedState, AuthorizedAsTenant, AuthorizedState,
};
use meteroid_grpc::meteroid::internal::v1::ResolveApiKeyRequest;
use meteroid_grpc::meteroid::internal::v1::internal_service_client::InternalServiceClient;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ApiAuthMiddleware<S> {
    inner: S,
    filter: Option<Filter>,
    internal_client: InternalServiceClient<LayeredClientService>,
}

#[derive(Debug, Clone)]
pub struct ExternalApiAuthLayer {
    internal_client: InternalServiceClient<LayeredClientService>,
    filter: Option<Filter>,
}

impl ExternalApiAuthLayer {
    #[allow(clippy::new_without_default)]
    pub fn new(internal_client: InternalServiceClient<LayeredClientService>) -> Self {
        ExternalApiAuthLayer {
            internal_client,
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
            internal_client: self.internal_client.clone(),
        }
    }
}

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

impl<S, ReqBody> Service<Request<ReqBody>> for ApiAuthMiddleware<S>
where
    S: Service<Request<ReqBody>, Response = Response<Body>, Error = BoxError>
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

        // This is necessary because tonic internally uses `tower::buffer::Buffer`.
        // See https://github.com/tower-rs/tower/issues/547#issuecomment-767629149
        // for details on why this is necessary
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        let _sm = GrpcServiceMethod::extract(request.uri());

        let metadata = request.headers().clone();
        let mut internal_client = self.internal_client.clone();

        let future = async move {
            let authenticated_state = if metadata.contains_key(API_KEY_HEADER) {
                validate_api_key(&metadata, &mut internal_client)
                    .await
                    .map_err(|e| BoxError::from(e) as BoxError)
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
                _ => Err(Box::new(Status::permission_denied(
                    "Only Api Key authentication is enabled for this service",
                )) as BoxError),
            }?;

            request.extensions_mut().insert(authorized_state);

            inner.call(request).await
        };

        // if the future is an error , we recover by providing an empty REsponse
        let future = future.or_else(|e: BoxError| async move {
            log::warn!("Error in auth middleware: {}", e);
            // TODO grpc_status + message ? ex of current behavior: Could not ingest events: Status { code: Unauthenticated, message: "grpc-status header missing, mapped from HTTP status code 401",
            let response = Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::empty())
                .unwrap();
            Ok(response)
        });

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
    internal_client: &mut InternalServiceClient<LayeredClientService>,
    validator: &ApiTokenValidator,
    api_key_id: &Uuid,
) -> Result<(OrganizationId, TenantId), Status> {
    let res = internal_client
        .clone()
        .resolve_api_key(ResolveApiKeyRequest {
            api_key_id: api_key_id.to_string(),
        })
        .await
        .map_err(|err| {
            error!("Failed to resolve api key: {:?}", err);
            Status::permission_denied("Failed to resolve api key")
                .set_source(Arc::new(err))
                .clone()
        })?;

    let inner = res.into_inner();

    validator
        .validate_hash(&inner.hash)
        .map_err(|_e| Status::permission_denied("Unauthorized. Invalid hash"))?;

    let tenant_uuid = TenantId::from_proto(inner.tenant_id)?;

    let organization_uuid = OrganizationId::from_proto(inner.organization_id)?;

    Ok((organization_uuid, tenant_uuid))
}

pub async fn validate_api_key(
    header_map: &HeaderMap,
    internal_client: &mut InternalServiceClient<LayeredClientService>,
) -> Result<AuthenticatedState, Status> {
    let api_key = header_map
        .get(API_KEY_HEADER)
        .ok_or(Status::unauthenticated("Missing API key"))?
        .to_str()
        .map_err(|_| Status::permission_denied("Invalid API key"))?;

    let validator = ApiTokenValidator::parse_api_key(api_key)
        .map_err(|_| Status::permission_denied("Invalid API key format."))?;

    let id = validator.extract_identifier().map_err(|_| {
        Status::permission_denied("Invalid API key format. Failed to extract identifier")
    })?;

    let (organization_id, tenant_id) =
        validate_api_token_by_id_cached(internal_client, &validator, &id).await?;

    Ok(AuthenticatedState::ApiKey {
        id,
        tenant_id,
        organization_id,
    })
}
