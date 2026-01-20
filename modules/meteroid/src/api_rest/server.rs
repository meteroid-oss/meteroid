use crate::adapters::stripe::Stripe;
use crate::api_rest::AppState;
use crate::api_rest::api_routes;
use crate::api_rest::error::{ErrorCode, RestErrorResponse};
use crate::api_rest::openapi::ApiDoc;
use crate::config::Config;
use crate::services::storage::ObjectStoreService;
use axum::response::Response;
use axum::routing::get;
use axum::{
    Json, Router, extract::DefaultBodyLimit, extract::Path, http::StatusCode, http::Uri,
    response::IntoResponse,
};
use axum_tracing_opentelemetry::middleware::{OtelAxumLayer, OtelInResponseLayer};
use meteroid_store::{Services, Store};
use std::any::Any;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::{
    DefaultMakeSpan, DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer,
};
use tracing::Level;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_scalar::{Scalar, Servable as ScalarServable};

pub async fn start_rest_server(
    config: Config,
    object_store: Arc<dyn ObjectStoreService>,
    stripe_adapter: Arc<Stripe>,
    store: Store,
    services: Services,
) -> Result<(), Box<dyn std::error::Error>> {
    let app_state = AppState {
        object_store,
        store: store.clone(),
        services,
        stripe_adapter,
        jwt_secret: config.jwt_secret,
        portal_url: config.public_url, // self-hosted - same url
    };

    let auth_layer =
        axum::middleware::from_fn_with_state(store.clone(), crate::api_rest::auth::auth_middleware);

    let (api_router, open_api) = OpenApiRouter::<AppState>::with_openapi(ApiDoc::openapi())
        .merge(api_routes())
        .split_for_parts();

    let openapi_json: bytes::Bytes = open_api
        .to_json()
        .expect("Failed to serialize OpenAPI")
        .into();

    let app = Router::new()
        .route("/health", get(|| async { "OK" }))
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
        .merge(Scalar::with_url("/scalar", open_api.clone()))
        //todo add "/api" to path and merge with api_routes
        .nest("/files", crate::api_rest::files::file_routes())
        .nest("/webhooks", crate::api_rest::webhooks::webhook_in_routes())
        .merge(crate::api_rest::oauth::oauth_routes())
        .merge(api_router)
        .fallback(handler_404)
        .with_state(app_state)
        .layer(auth_layer)
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(1024 * 1024)) // 1 MB
        .layer(CatchPanicLayer::custom(handle_500))
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
        );

    tracing::info!("Starting REST API on {}", config.rest_api_addr);

    let listener = TcpListener::bind(&config.rest_api_addr)
        .await
        .expect("Could not bind listener");

    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
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
