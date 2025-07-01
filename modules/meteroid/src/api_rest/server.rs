use crate::adapters::stripe::Stripe;
use crate::api_rest::AppState;
use crate::api_rest::api_routes;
use crate::api_rest::auth::ExternalApiAuthLayer;
use crate::config::Config;
use crate::services::storage::ObjectStoreService;
use axum::routing::get;
use axum::{
    Router, extract::DefaultBodyLimit, http::StatusCode, http::Uri, response::IntoResponse,
};
use meteroid_store::{Services, Store};
use std::sync::Arc;
use tokio::net::TcpListener;
use utoipa::{
    Modify, OpenApi,
    openapi::security::{ApiKey, ApiKeyValue, SecurityScheme},
};
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
                "api-key",
                SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("x-api-key"))),
            )
        }
    }
}
fn only_api(path: &str) -> bool {
    path.starts_with("/api/")
}

pub async fn start_rest_server(
    config: &Config,
    object_store: Arc<dyn ObjectStoreService>,
    stripe_adapter: Arc<Stripe>,
    store: Store,
    services: Services,
) -> Result<(), Box<dyn std::error::Error>> {
    let app_state = AppState {
        object_store,
        store: store.clone(),
        services ,
        stripe_adapter,
        jwt_secret: config.jwt_secret.clone(),
    };

    let auth_layer = ExternalApiAuthLayer::new(store.clone()).filter(only_api);

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
        .layer(DefaultBodyLimit::max(4096));

    tracing::info!("Starting REST API on {}", config.rest_api_addr.clone());

    let listener = TcpListener::bind(&config.rest_api_addr)
        .await
        .expect("Could not bind listener");

    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}

async fn handler_404(uri: Uri) -> impl IntoResponse {
    log::warn!("Not found {}", uri);
    (StatusCode::NOT_FOUND, "Not found")
}
