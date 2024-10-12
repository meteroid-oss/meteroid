use crate::adapters::stripe::Stripe;
use crate::api::axum_routers;
use crate::services::storage::ObjectStoreService;
use axum::{
    extract::DefaultBodyLimit, http::StatusCode, http::Uri, response::IntoResponse, Router,
};
use meteroid_store::Store;
use secrecy::SecretString;
use std::net::SocketAddr;
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
        (path = "/files", api = axum_routers::FileApi)
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
pub async fn serve(
    listen_addr: SocketAddr,
    object_store: Arc<dyn ObjectStoreService>,
    stripe_adapter: Arc<Stripe>,
    store: Store,
    jwt_secret: SecretString,
) {
    let app_state = axum_routers::AppState {
        object_store,
        store,
        stripe_adapter,
        jwt_secret,
    };

    let app = Router::new()
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .merge(Redoc::with_url("/redoc", ApiDoc::openapi()))
        // There is no need to create `RapiDoc::with_openapi` because the OpenApi is served
        // via SwaggerUi instead we only make rapidoc to point to the existing doc.
        .merge(RapiDoc::new("/api-docs/openapi.json").path("/rapidoc"))
        // Alternative to above
        // .merge(RapiDoc::with_openapi("/api-docs/openapi2.json", ApiDoc::openapi()).path("/rapidoc"))
        .merge(Scalar::with_url("/scalar", ApiDoc::openapi()))
        .nest("/files", axum_routers::file_routes())
        .nest("/webhooks", axum_routers::webhook_in_routes())
        .fallback(handler_404)
        .with_state(app_state)
        .layer(DefaultBodyLimit::max(4096));

    tracing::info!("listening on {}", listen_addr);

    let listener = TcpListener::bind(&listen_addr)
        .await
        .expect("Could not bind listener");
    axum::serve(listener, app.into_make_service())
        .await
        .expect("Could not bind server");
}

async fn handler_404(uri: Uri) -> impl IntoResponse {
    log::warn!("Not found {}", uri);
    (StatusCode::NOT_FOUND, "Not found")
}
