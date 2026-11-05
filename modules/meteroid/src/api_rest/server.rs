use crate::api_rest::AppState;
use crate::api_rest::client::AuthorizedAsCustomer;
use crate::api_rest::rate_limit::{
    build_client_rate_limit_state, build_rate_limit_state, build_rest_rate_limiters,
};
use crate::config::Config;
use crate::module::{ApiContext, Modules};
use crate::services::egress::{Egress, EgressPurpose};
use crate::services::storage::ObjectStoreService;
use axum::body::Body;
use axum::response::Response;
use axum::routing::get;
use axum::{
    Json, Router, extract::DefaultBodyLimit, extract::Path, http::StatusCode, http::Uri,
    response::IntoResponse,
};
use axum_tracing_opentelemetry::middleware::{OtelAxumLayer, OtelInResponseLayer};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use common_grpc::middleware::server::context::RequestContext;
use meteroid_app::cors::cors;
use meteroid_rest_common::error::{ErrorCode, RestErrorResponse};
use meteroid_store::{Services, Store};
use rate_limiter::RateLimiter;
use rate_limiter::middleware::axum::rate_limit_middleware;
use std::any::Any;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::{
    DefaultMakeSpan, DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer,
};
use tracing::{Instrument, Level};
use utoipa_axum::router::OpenApiRouter;
use utoipa_scalar::{Scalar, Servable as ScalarServable};

pub async fn start_rest_server(
    config: Config,
    object_store: Arc<dyn ObjectStoreService>,
    store: Store,
    services: Services,
    ready: Arc<std::sync::atomic::AtomicBool>,
    rate_limiter: RateLimiter,
    modules: Modules,
    egress: Egress,
) -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(&config.api.rest_api_addr)
        .await
        .expect("Could not bind listener");
    start_rest_server_with_listener(
        config,
        object_store,
        store,
        services,
        ready,
        listener,
        rate_limiter,
        modules,
        egress,
    )
    .await
}

pub async fn start_rest_server_with_listener<L>(
    config: Config,
    object_store: Arc<dyn ObjectStoreService>,
    store: Store,
    services: Services,
    ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
    listener: L,
    rate_limiter: RateLimiter,
    modules: Modules,
    egress: Egress,
) -> Result<(), Box<dyn std::error::Error>>
where
    L: axum::serve::Listener<Addr = std::net::SocketAddr>,
{
    let rest_rate_limiters = build_rest_rate_limiters(&rate_limiter, &config.api.rate_limit.rest);

    let feature_resolver = modules.entitlements(&store);
    let analytics = modules.analytics_hook(&store, &config, &egress);

    let module_ctx = ApiContext {
        config: config.clone(),
        store: store.clone(),
        services: services.clone(),
        feature_resolver: feature_resolver.clone(),
        feature_gates: modules.feature_gates().into(),
        rate_limiter: rate_limiter.clone(),
        redis: None,
        analytics,
        egress: egress.clone(),
    };
    let mut module_api = OpenApiRouter::new();
    let mut module_client = OpenApiRouter::new();
    let mut module_other = Router::new();
    for module in &modules.api {
        let routes = module.rest(&module_ctx).await;
        module_api = module_api.merge(routes.api);
        module_client = module_client.merge(routes.client);
        module_other = module_other.merge(routes.other);
    }
    // Same SSRF guards as the delivery worker, so an endpoint the worker would refuse
    // is rejected at create/update time instead.
    let webhook_egress = egress.client(EgressPurpose::Webhooks)?;
    let app_state = AppState {
        object_store,
        store: store.clone(),
        services,
        jwt_secret: config.jwt_secret,
        portal_url: config.public_url, // TODO separate portal URL from public URL
        api_url: config
            .rest_api_external_url
            .trim_end_matches('/')
            .to_string(),
        rest_api_external_url: config.rest_api_external_url.clone(),
        trusted_proxy_hops: config.api.trusted_proxy_hops,
        ready,
        rest_rate_limiters: rest_rate_limiters.clone(),
        feature_resolver,
        subscription_usage: crate::api_rest::usage::model::new_subscription_usage_cache(),
        webhook_egress,
        webhook_out: config.webhook_out,
    };

    let auth_layer = axum::middleware::from_fn_with_state(
        crate::api_rest::auth::AuthState {
            store: store.clone(),
            delegated: modules.rest_auth(&module_ctx),
        },
        crate::api_rest::auth::auth_middleware,
    );

    let rate_limit_layer = axum::middleware::from_fn_with_state(
        build_rate_limit_state(&rest_rate_limiters),
        rate_limit_middleware,
    );

    let client_auth_layer = axum::middleware::from_fn_with_state(
        app_state.clone(),
        crate::api_rest::client::client_auth_middleware,
    );

    let client_rate_limit_layer = axum::middleware::from_fn_with_state(
        build_client_rate_limit_state(&rest_rate_limiters),
        rate_limit_middleware,
    );

    // The modules' share of the specs as the edition publishes it (`ApiModule::openapi`).
    let (module_api, _) = module_api.split_for_parts();
    let (module_client, _) = module_client.split_for_parts();
    let edition = modules.spec();
    let (api_router, mut open_api) = crate::api_rest::openapi::edition_spec(&edition);
    crate::api_rest::openapi::add_rate_limit_responses(&mut open_api);
    crate::api_rest::openapi::discriminate_tagged_variants(&mut open_api);
    crate::api_rest::openapi::apply_sorted_tags(&mut open_api);
    crate::api_rest::openapi::label_beta_operations(&mut open_api);

    let openapi_json: bytes::Bytes =
        serde_json::to_vec(&crate::api_rest::openapi::spec_json(&open_api))
            .expect("Failed to serialize OpenAPI")
            .into();

    let (client_router, client_open_api) = crate::api_rest::client::client_api(edition.client);
    let client_openapi_json: bytes::Bytes =
        serde_json::to_vec(&crate::api_rest::openapi::spec_json(&client_open_api))
            .expect("Failed to serialize client OpenAPI")
            .into();

    let app = Router::new()
        .route("/health", get(health_check))
        .route("/id/{id}", get(resolve_id))
        .route(
            "/api-docs/openapi.json",
            get({
                let spec = openapi_json.clone();
                || async move {
                    (
                        [(http::header::CONTENT_TYPE, "application/json")],
                        spec.clone(),
                    )
                }
            }),
        )
        .route(
            "/api-docs/client/openapi.json",
            get(|| async move {
                (
                    [(http::header::CONTENT_TYPE, "application/json")],
                    client_openapi_json,
                )
            }),
        )
        .merge(Scalar::with_url("/scalar", open_api.clone()))
        //todo add "/api" to path and merge with api_routes
        .nest("/files", crate::api_rest::files::file_routes())
        .nest("/webhooks", crate::api_rest::webhooks::webhook_in_routes())
        .merge(crate::api_rest::hosted_return::hosted_return_routes())
        .merge(crate::api_rest::portal_links::portal_link_routes())
        .merge(crate::api_rest::mock::mock_routes())
        .merge(crate::api_rest::oauth::oauth_routes())
        .merge(crate::api_rest::sso::sso_routes())
        .merge(api_router)
        .merge(client_router)
        .fallback(handler_404)
        .with_state(app_state)
        // Module routes come with their own state.
        .merge(module_api)
        .merge(module_client)
        .merge(module_other);

    let app = app
        .layer(axum::middleware::from_fn(request_context))
        .layer(rate_limit_layer)
        .layer(client_rate_limit_layer)
        .layer(client_auth_layer)
        .layer(auth_layer)
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(1024 * 1024)) // 1 MB
        .layer(CatchPanicLayer::custom(handle_500))
        // Log the final 4xx/5xx body (including axum-extractor/axum-valid rejections that bypass RestApiError).
        .layer(axum::middleware::from_fn(log_error_responses))
        // include trace context as a header in the response
        .layer(OtelInResponseLayer)
        //start OpenTelemetry trace on an incoming request
        .layer(OtelAxumLayer::default().filter(otel_filter))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(
                    DefaultMakeSpan::new()
                        .level(Level::TRACE)
                        .include_headers(false),
                )
                .on_request(DefaultOnRequest::new().level(Level::DEBUG))
                .on_response(DefaultOnResponse::new().level(Level::DEBUG))
                .on_failure(DefaultOnFailure::new().level(Level::WARN)),
        )
        // Outermost so browser preflight/origin reflection applies before auth,
        // rate limiting, etc. Needed for the browser-facing `/sso/*` handshake
        // (login page fetches these cross-origin).
        .layer(cors());

    tracing::info!("Starting REST API on {}", config.api.rest_api_addr);

    // `tap_io` is what gives any listener's peer address to `ConnectInfo`.
    axum::serve(
        axum::serve::ListenerExt::tap_io(listener, |_| {}),
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;

    Ok(())
}

async fn health_check(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl IntoResponse {
    if state.ready.load(std::sync::atomic::Ordering::Acquire) {
        (StatusCode::OK, "OK")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "NOT_READY")
    }
}

async fn handler_404(uri: Uri) -> impl IntoResponse {
    log::debug!("Not found {uri}");
    (
        StatusCode::NOT_FOUND,
        Json(RestErrorResponse {
            code: ErrorCode::NotFound,
            message: "Resource not found".to_string(),
        }),
    )
}

fn handle_500(_panic: Box<dyn Any + Send>) -> Response {
    let body = Json(RestErrorResponse {
        code: ErrorCode::InternalServerError,
        message: "Internal Server Error".to_string(),
    });

    (StatusCode::INTERNAL_SERVER_ERROR, body).into_response()
}

async fn resolve_id(Path(id): Path<String>) -> impl IntoResponse {
    let base62_part = id.rsplit_once('_').map(|(_, b)| b).unwrap_or(&id);

    match base62::decode(base62_part) {
        Ok(decoded) => {
            let uuid = uuid::Uuid::from_u128(decoded.rotate_right(67));
            (StatusCode::OK, uuid.to_string())
        }
        Err(_) => (StatusCode::BAD_REQUEST, "invalid id".to_string()),
    }
}

const OTEL_SKIP_PATH_PREFIXES: &[&str] = &["/health", "/api-docs/", "/scalar"];

fn otel_filter(path: &str) -> bool {
    !OTEL_SKIP_PATH_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

/// Max body bytes to read/log when a 4xx/5xx response is produced. Bodies
/// larger than this are truncated in the log but re-streamed to the client.
const ERROR_LOG_BODY_LIMIT: usize = 16 * 1024;

/// Runs an authenticated request inside its `request_context` span. Sits inside the auth
/// middlewares, which resolve the caller, and passes the context back on the response for
/// `log_error_responses`, which sits outside them.
async fn request_context(req: http::Request<Body>, next: axum::middleware::Next) -> Response {
    let ctx = req
        .extensions()
        .get::<AuthorizedAsTenant>()
        .map(RequestContext::of_tenant)
        .or_else(|| {
            req.extensions()
                .get::<AuthorizedAsCustomer>()
                .map(|customer| RequestContext {
                    tenant_id: Some(customer.tenant_id),
                    customer_id: Some(customer.customer_id),
                    ..RequestContext::default()
                })
        });
    let Some(ctx) = ctx else {
        return next.run(req).await;
    };
    let ctx = ctx.with_ids_from_path(req.uri().path());
    let mut response = next.run(req).instrument(ctx.span()).await;
    response.extensions_mut().insert(ctx);
    response
}

/// Logs the method/path/status/body of any 4xx/5xx response so that errors
/// generated by extractor rejections (axum-valid, `Query`, `Json`, etc.) —
/// which don't flow through `RestApiError` — still show up in server logs.
async fn log_error_responses(req: http::Request<Body>, next: axum::middleware::Next) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let response = next.run(req).await;

    let status = response.status();
    if !status.is_client_error() && !status.is_server_error() {
        return response;
    }

    // Skip high-volume attack/noise statuses
    if matches!(
        status,
        StatusCode::NOT_FOUND | StatusCode::UNAUTHORIZED | StatusCode::TOO_MANY_REQUESTS
    ) {
        return response;
    }

    let ctx = response
        .extensions()
        .get::<RequestContext>()
        .copied()
        .unwrap_or_default();
    let tenant_id = ctx.tenant_id.map(tracing::field::display);
    let customer_id = ctx.customer_id.map(tracing::field::display);
    let subscription_id = ctx.subscription_id.map(tracing::field::display);
    let invoice_id = ctx.invoice_id.map(tracing::field::display);

    let (parts, body) = response.into_parts();
    match axum::body::to_bytes(body, ERROR_LOG_BODY_LIMIT).await {
        Ok(bytes) => {
            let body_str = std::str::from_utf8(&bytes).unwrap_or("<non-utf8 body>");
            // Debug-formatted so control chars / ANSI escapes are neutralized
            tracing::warn!(
                method = %method,
                path = %uri.path(),
                status = status.as_u16(),
                tenant_id,
                customer_id,
                subscription_id,
                invoice_id,
                body = ?body_str,
                "REST API error response"
            );
            Response::from_parts(parts, Body::from(bytes))
        }
        Err(err) => {
            tracing::warn!(
                method = %method,
                path = %uri.path(),
                status = status.as_u16(),
                tenant_id,
                customer_id,
                subscription_id,
                invoice_id,
                error = %err,
                "REST API error response (body read failed)"
            );
            // Body was consumed by to_bytes; return an empty body rather than
            // hanging the client.
            Response::from_parts(parts, Body::empty())
        }
    }
}
