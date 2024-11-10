use crate::adapters::stripe::Stripe;
use crate::api_rest::auth::ExternalApiAuthLayer;
use crate::api_rest::files;
use crate::api_rest::subscriptions;
use crate::api_rest::AppState;
use crate::config::Config;
use crate::services::storage::ObjectStoreService;
use axum::{
    extract::DefaultBodyLimit, http::StatusCode, http::Uri, response::IntoResponse, Router,
};
use meteroid_store::Store;
use std::sync::Arc;
use tokio::net::TcpListener;
use utoipa::{
    openapi::security::{ApiKey, ApiKeyValue, SecurityScheme},
    Modify, OpenApi,
};
use utoipa_rapidoc::RapiDoc;
use utoipa_redoc::{Redoc, Servable};
use utoipa_scalar::{Scalar, Servable as ScalarServable};
use utoipa_swagger_ui::SwaggerUi;

#[derive(OpenApi)]
#[openapi(
    modifiers(&SecurityAddon),
    nest(
        (path = "/files", api = files::FileApi),
        (path = "/api/v1/subscriptions", api = subscriptions::SubscriptionApi)
    ),
    tags(
        (name = "meteroid", description = "Meteroid API")
    )
)]
struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "api_key",
                SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("apikey"))),
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
) -> Result<(), Box<dyn std::error::Error>> {
    let app_state = AppState {
        object_store,
        store: store.clone(),
        stripe_adapter,
        jwt_secret: config.jwt_secret.clone(),
    };

    let auth_layer = ExternalApiAuthLayer::new(store.clone()).filter(only_api);

    let app = Router::new()
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .merge(Redoc::with_url("/redoc", ApiDoc::openapi()))
        // There is no need to create `RapiDoc::with_openapi` because the OpenApi is served
        // via SwaggerUi instead we only make rapidoc to point to the existing doc.
        .merge(RapiDoc::new("/api-docs/openapi.json").path("/rapidoc"))
        // Alternative to above
        // .merge(RapiDoc::with_openapi("/api-docs/openapi2.json", ApiDoc::openapi()).path("/rapidoc"))
        .merge(Scalar::with_url("/scalar", ApiDoc::openapi()))
        //todo add "/api" to path and merge with api_routes
        .nest("/files", crate::api_rest::files::file_routes())
        .nest("/webhooks", crate::api_rest::webhooks::webhook_routes())
        //
        .nest("/api", crate::api_rest::api_routes())
        .fallback(handler_404)
        .with_state(app_state)
        //.layer(ServiceBuilder::new().layer(auth_layer))
        .layer(auth_layer)
        .layer(DefaultBodyLimit::max(4096));

    tracing::info!("listening on {}", config.rest_api_addr.clone());

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
