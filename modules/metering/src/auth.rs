use common_grpc::GrpcServiceMethod;
use common_grpc::middleware::common::auth::API_KEY_HEADER;

use cached::proc_macro::cached;
use common_grpc::middleware::client::LayeredClientService;
use common_grpc::middleware::server::auth::api_token_validator::{
    ApiTokenSecretDigest, ApiTokenValidator,
};
use futures::TryFutureExt;
use futures::future::BoxFuture;
use hyper::{HeaderMap, Request, Response, StatusCode};
use std::sync::Arc;
use std::task::{Context, Poll};
use tonic::Status;
use tonic::body::Body;
use tower::{BoxError, Service};
use tower_layer::Layer;
use tracing::{error, log};

use common_grpc::middleware::common::filters::Filter;

use common_domain::ids::{ApiTokenId, OrganizationId, TenantId};
use common_grpc::middleware::server::auth::{
    AuthenticatedState, AuthorizedAsTenant, AuthorizedState, TenantActor, TenantEnv,
};
use meteroid_grpc::meteroid::api::tenants::v1::TenantEnvironmentEnum;
use meteroid_grpc::meteroid::internal::v1::internal_service_client::InternalServiceClient;
use meteroid_grpc::meteroid::internal::v1::{ReportApiKeyUsageRequest, ResolveApiKeyRequest};
use uuid::Uuid;

/// Set on API-key requests by [`ApiAuthMiddleware`], read by the ingest path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TenantIngestSettings {
    pub keep_last_events: bool,
}

#[derive(Clone)]
struct ResolvedApiKey {
    organization_id: OrganizationId,
    tenant_id: TenantId,
    tenant_env: TenantEnv,
    settings: TenantIngestSettings,
}

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

impl<S, ReqBody> Service<Request<ReqBody>> for ApiAuthMiddleware<S>
where
    S: Service<Request<ReqBody>, Response = Response<Body>> + Clone + Send + 'static,
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
        if !self.filter.is_none_or(|f| f(request.uri().path())) {
            let mut inner = self.inner.clone();
            return Box::pin(async move { inner.call(request).await.map_err(Into::into) });
        }

        // tonic wraps services in `tower::buffer::Buffer`; swap in a clone so the readied
        // service is the one we call. See tower-rs/tower#547 (comment 767629149).
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        let _sm = GrpcServiceMethod::extract(request.uri());

        let metadata = request.headers().clone();
        let internal_client = self.internal_client.clone();

        let future = async move {
            let (authenticated_state, settings) = if metadata.contains_key(API_KEY_HEADER) {
                validate_api_key(&metadata, &internal_client)
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
                    tenant_env,
                } => Ok(AuthorizedState::Tenant(AuthorizedAsTenant {
                    tenant_id,
                    organization_id,
                    actor: TenantActor::ApiKey(id),
                    tenant_env,
                })),
                _ => Err(Box::new(Status::permission_denied(
                    "Only Api Key authentication is enabled for this service",
                )) as BoxError),
            }?;

            request.extensions_mut().insert(authorized_state);
            request.extensions_mut().insert(settings);

            inner.call(request).await.map_err(Into::into)
        };

        let future = future.or_else(|e: BoxError| async move {
            log::warn!("Error in metering auth middleware: {e}");
            let response = Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::empty())
                .unwrap();
            Ok(response)
        });

        Box::pin(future)
    }
}

// The key must include the presented secret: keying on the token id alone lets anyone
// quoting a known id reuse the entry a legitimate request populated, skipping validate_hash.
#[cached(
    result = true,
    size = 50_000,
    time = 120, // 2 min
    key = "(Uuid, ApiTokenSecretDigest)",
    convert = r#"{ (*api_key_id, validator.secret_digest()) }"#
)]
async fn validate_api_token_by_id_cached(
    internal_client: &InternalServiceClient<LayeredClientService>,
    validator: &ApiTokenValidator,
    api_key_id: &Uuid,
) -> Result<ResolvedApiKey, Status> {
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

    // Fire-and-forget usage stamp; debounced by this fn's cache TTL.
    {
        let mut client = internal_client.clone();
        let api_key_id = api_key_id.to_string();
        tokio::spawn(async move {
            let res = client
                .report_api_key_usage(ReportApiKeyUsageRequest { api_key_id })
                .await;
            if let Err(err) = res {
                error!("Failed to report api key usage: {:?}", err);
            }
        });
    }

    let tenant_uuid = TenantId::from_proto(inner.tenant_id)?;

    let organization_uuid = OrganizationId::from_proto(inner.organization_id)?;

    let tenant_env = match TenantEnvironmentEnum::try_from(inner.environment) {
        Ok(TenantEnvironmentEnum::Production) | Err(_) => TenantEnv::Production,
        Ok(_) => TenantEnv::NonProduction,
    };

    Ok(ResolvedApiKey {
        organization_id: organization_uuid,
        tenant_id: tenant_uuid,
        tenant_env,
        settings: TenantIngestSettings {
            keep_last_events: inner.keep_last_events,
        },
    })
}

pub async fn validate_api_key(
    header_map: &HeaderMap,
    internal_client: &InternalServiceClient<LayeredClientService>,
) -> Result<(AuthenticatedState, TenantIngestSettings), Status> {
    let api_key = header_map
        .get(API_KEY_HEADER)
        .ok_or(Status::unauthenticated("Missing API key"))?
        .to_str()
        .map_err(|_| Status::permission_denied("Invalid API key"))?;

    let validator = ApiTokenValidator::parse_api_key(api_key)
        .map_err(|_| Status::permission_denied("Invalid API key format."))?;

    let id = ApiTokenId::from_const(validator.extract_identifier().map_err(|_| {
        Status::permission_denied("Invalid API key format. Failed to extract identifier")
    })?);

    let resolved = validate_api_token_by_id_cached(internal_client, &validator, &id).await?;

    Ok((
        AuthenticatedState::ApiKey {
            id,
            tenant_id: resolved.tenant_id,
            organization_id: resolved.organization_id,
            tenant_env: resolved.tenant_env,
        },
        resolved.settings,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::password_hash::{PasswordHasher, SaltString};
    use common_config::auth::InternalAuthConfig;
    use common_grpc::middleware::client::build_layered_client_service;
    use hyper::http::{HeaderName, HeaderValue};
    use meteroid_grpc::meteroid::internal::v1::internal_service_server::{
        InternalService, InternalServiceServer,
    };
    use meteroid_grpc::meteroid::internal::v1::{
        ListBillableMetricsRequest, ListBillableMetricsResponse, ReportApiKeyUsageRequest,
        ReportApiKeyUsageResponse, ResolveApiKeyResponse, ResolveCustomerAliasesRequest,
        ResolveCustomerAliasesResponse,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tonic::transport::Endpoint;
    use tonic::transport::server::TcpIncoming;

    const SECRET: &str = "9XzHg0EYO2Usy9ITU6bbhBnkYYbx";
    const WRONG_SECRET: &str = "0000000000000000000000000000";

    struct MockInternal {
        hash: String,
        resolve_calls: Arc<AtomicUsize>,
    }

    #[tonic::async_trait]
    impl InternalService for MockInternal {
        async fn resolve_customer_aliases(
            &self,
            _: tonic::Request<ResolveCustomerAliasesRequest>,
        ) -> Result<tonic::Response<ResolveCustomerAliasesResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }

        async fn resolve_api_key(
            &self,
            _: tonic::Request<ResolveApiKeyRequest>,
        ) -> Result<tonic::Response<ResolveApiKeyResponse>, Status> {
            self.resolve_calls.fetch_add(1, Ordering::SeqCst);
            Ok(tonic::Response::new(ResolveApiKeyResponse {
                tenant_id: Uuid::from_u128(1).to_string(),
                organization_id: Uuid::from_u128(2).to_string(),
                hash: self.hash.clone(),
                environment:
                    meteroid_grpc::meteroid::api::tenants::v1::TenantEnvironmentEnum::Sandbox as i32,
                keep_last_events: true,
            }))
        }

        async fn report_api_key_usage(
            &self,
            _: tonic::Request<ReportApiKeyUsageRequest>,
        ) -> Result<tonic::Response<ReportApiKeyUsageResponse>, Status> {
            Ok(tonic::Response::new(ReportApiKeyUsageResponse {}))
        }

        async fn list_billable_metrics(
            &self,
            _: tonic::Request<ListBillableMetricsRequest>,
        ) -> Result<tonic::Response<ListBillableMetricsResponse>, Status> {
            Err(Status::unimplemented("unused"))
        }
    }

    fn argon2_hash(secret: &str) -> String {
        let salt = SaltString::encode_b64(b"meteroid-auth-test").unwrap();
        argon2::Argon2::default()
            .hash_password(secret.as_bytes(), &salt)
            .unwrap()
            .to_string()
    }

    /// Two API keys sharing one token id: the real one, and one with a forged secret.
    fn key_pair() -> (String, String) {
        let id_part = base62::encode(Uuid::now_v7().as_u128());
        (
            format!("pv_sand_{SECRET}/{id_part}"),
            format!("pv_sand_{WRONG_SECRET}/{id_part}"),
        )
    }

    fn headers(api_key: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static(API_KEY_HEADER),
            HeaderValue::from_str(api_key).unwrap(),
        );
        headers
    }

    async fn spawn_internal_service() -> (
        InternalServiceClient<LayeredClientService>,
        Arc<AtomicUsize>,
    ) {
        let resolve_calls = Arc::new(AtomicUsize::new(0));
        let incoming = TcpIncoming::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = incoming.local_addr().unwrap();

        let service = InternalServiceServer::new(MockInternal {
            hash: argon2_hash(SECRET),
            resolve_calls: resolve_calls.clone(),
        });

        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(service)
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });

        let channel = Endpoint::from_shared(format!("http://{addr}"))
            .unwrap()
            .connect_lazy();

        let client = InternalServiceClient::new(build_layered_client_service(
            channel,
            &InternalAuthConfig {
                hmac_secret: "secret".to_string().into(),
            },
        ));

        (client, resolve_calls)
    }

    /// Regression: the validation cache used to be keyed on the token id alone, so once a
    /// legitimate request warmed it, any request quoting that id authenticated without the
    /// secret ever reaching argon2.
    #[tokio::test]
    async fn cached_token_id_does_not_authenticate_a_forged_secret() {
        let (client, resolve_calls) = spawn_internal_service().await;
        let (valid_key, forged_key) = key_pair();

        validate_api_key(&headers(&valid_key), &client)
            .await
            .expect("valid key should authenticate and warm the cache");

        let forged = validate_api_key(&headers(&forged_key), &client).await;

        assert!(
            forged.is_err(),
            "a forged secret must not reuse the cache entry of a known token id"
        );
        assert_eq!(
            resolve_calls.load(Ordering::SeqCst),
            2,
            "the forged secret must miss the cache and be argon2-verified"
        );
    }

    #[tokio::test]
    async fn repeated_valid_requests_are_served_from_the_cache() {
        let (client, resolve_calls) = spawn_internal_service().await;
        let (valid_key, _) = key_pair();

        let first = validate_api_key(&headers(&valid_key), &client)
            .await
            .unwrap();
        let second = validate_api_key(&headers(&valid_key), &client)
            .await
            .unwrap();

        match (first, second) {
            (
                (AuthenticatedState::ApiKey { tenant_id: a, .. }, settings_a),
                (AuthenticatedState::ApiKey { tenant_id: b, .. }, settings_b),
            ) => {
                assert_eq!(a, b);
                assert!(
                    settings_a.keep_last_events,
                    "the flag comes from ResolveApiKey"
                );
                assert_eq!(settings_a, settings_b, "the cached result keeps the flag");
            }
            _ => panic!("expected api key authentication"),
        }
        assert_eq!(
            resolve_calls.load(Ordering::SeqCst),
            1,
            "the second valid request must not hit the internal service again"
        );
    }

    #[tokio::test]
    async fn a_forged_secret_never_populates_a_usable_entry() {
        let (client, resolve_calls) = spawn_internal_service().await;
        let (_, forged_key) = key_pair();

        assert!(
            validate_api_key(&headers(&forged_key), &client)
                .await
                .is_err()
        );
        assert!(
            validate_api_key(&headers(&forged_key), &client)
                .await
                .is_err()
        );

        assert_eq!(
            resolve_calls.load(Ordering::SeqCst),
            2,
            "failed validations must not be cached"
        );
    }
}
