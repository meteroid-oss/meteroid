use crate::adapters::stripe::Stripe;
use crate::api_rest::AppState;
use crate::api_rest::api_routes;
use crate::api_rest::error::{ErrorCode, RestErrorResponse};
use crate::config::Config;
use crate::services::storage::ObjectStoreService;
use axum::response::Response;
use axum::routing::get;
use axum::{
    Json, Router, extract::DefaultBodyLimit, http::StatusCode, http::Uri, response::IntoResponse,
};
use meteroid_store::{Services, Store};
use std::any::Any;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::catch_panic::CatchPanicLayer;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder};
use utoipa::{Modify, OpenApi, openapi::security::SecurityScheme};
use utoipa_axum::router::OpenApiRouter;
use utoipa_rapidoc::RapiDoc;
use utoipa_redoc::{Redoc, Servable};
use utoipa_scalar::{Scalar, Servable as ScalarServable};
use utoipa_swagger_ui::SwaggerUi;

#[derive(OpenApi)]
#[openapi(
    modifiers(&SecurityAddon),
    nest(
        (path = "/files", api = crate::api_rest::files::FileApi),
    ),
    tags((name = "meteroid", description = "Meteroid API"))
)]
pub struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(HttpBuilder::new().scheme(HttpAuthScheme::Bearer).build()),
            )
        }
    }
}

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
    };

    let auth_layer =
        axum::middleware::from_fn_with_state(store.clone(), crate::api_rest::auth::auth_middleware);

    let (api_router, open_api) = OpenApiRouter::<AppState>::with_openapi(ApiDoc::openapi())
        .merge(api_routes())
        .split_for_parts();

    let app = Router::new()
        .route("/health", get(|| async { "OK" }))
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", open_api.clone()))
        .merge(Redoc::with_url("/redoc", open_api.clone()))
        .merge(RapiDoc::new("/api-docs/openapi.json").path("/rapidoc"))
        .merge(Scalar::with_url("/scalar", open_api.clone()))
        //todo add "/api" to path and merge with api_routes
        .nest("/files", crate::api_rest::files::file_routes())
        .nest("/webhooks", crate::api_rest::webhooks::webhook_in_routes())
        .merge(crate::api_rest::oauth::oauth_routes())
        .merge(api_router)
        .fallback(handler_404)
        .with_state(app_state)
        .layer(auth_layer)
        .layer(DefaultBodyLimit::max(4096))
        .layer(CatchPanicLayer::custom(handle_500));

    tracing::info!("Starting REST API on {}", config.rest_api_addr);

    let listener = TcpListener::bind(&config.rest_api_addr)
        .await
        .expect("Could not bind listener");

    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}

async fn handler_404(uri: Uri) -> impl IntoResponse {
    log::debug!("Not found {}", uri);
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
