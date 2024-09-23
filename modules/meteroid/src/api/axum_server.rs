use crate::adapters::stripe::Stripe;
use crate::api::axum_routers;
use crate::services::storage::ObjectStoreService;
use axum::extract::DefaultBodyLimit;
use axum::response::IntoResponse;
use axum::Router;
use http::{StatusCode, Uri};
use meteroid_store::Store;
use secrecy::SecretString;
use std::net::SocketAddr;
use std::sync::Arc;

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
        .nest("/files", axum_routers::file_routes())
        .nest("/webhooks", axum_routers::webhook_in_routes())
        .fallback(handler_404)
        .with_state(app_state)
        .layer(DefaultBodyLimit::max(4096));

    tracing::info!("listening on {}", listen_addr);
    axum::Server::bind(&listen_addr)
        .serve(app.into_make_service())
        .await
        .expect("Could not bind server");
}

async fn handler_404(uri: Uri) -> impl IntoResponse {
    log::warn!("Not found {}", uri);
    (StatusCode::NOT_FOUND, "Not found")
}
