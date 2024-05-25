use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
    http::{Request, Uri},
    response::{IntoResponse, Response},
};
use axum::{routing::post, Router};
use hyper::StatusCode;
use object_store::ObjectStore;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::{adapters::types::WebhookAdapter, errors};
use crate::{
    adapters::{stripe::Stripe, types::ParsedRequest},
    encoding,
};

use error_stack::{bail, Result, ResultExt};
use meteroid_store::domain::enums::InvoicingProviderEnum;
use meteroid_store::domain::webhooks::WebhookInEventNew;
use meteroid_store::repositories::configs::ConfigsInterface;
use meteroid_store::repositories::webhooks::WebhooksInterface;
use meteroid_store::Store;
use secrecy::SecretString;

#[derive(Clone)]
pub struct AppState {
    pub object_store: Arc<dyn ObjectStore>,
    pub store: Store,
    pub stripe_adapter: Arc<Stripe>,
}

pub async fn serve(
    listen_addr: SocketAddr,
    object_store_client: Arc<dyn ObjectStore>,
    stripe_adapter: Arc<Stripe>,
    store: Store,
) {
    let app_state = AppState {
        object_store: object_store_client.clone(),
        store,
        stripe_adapter: stripe_adapter.clone(),
    };

    // db: Arc<Database>,
    let app = Router::new()
        .route("/v1/:provider/:endpoint_uid", post(axum_handler))
        .fallback(handler_404)
        .with_state(app_state)
        .layer(DefaultBodyLimit::max(4096));

    tracing::info!("listening on {}", listen_addr);
    axum::Server::bind(&listen_addr)
        .serve(app.into_make_service())
        .await
        .expect("Could not bind server");
}

#[axum::debug_handler]
async fn axum_handler(
    Path((provider, endpoint_uid)): Path<(String, String)>,
    State(app_state): State<AppState>,
    req: Request<Body>,
) -> impl IntoResponse {
    match handler(provider, endpoint_uid, req, app_state).await {
        Ok(r) => r.into_response(),
        Err(e) => {
            log::error!("Error handling webhook: {}", e);
            e.current_context().clone().into_response()
        }
    }
}

async fn handler(
    provider_str: String,
    endpoint_uid: String,
    req: Request<Body>,
    app_state: AppState,
) -> Result<Response, errors::AdapterWebhookError> {
    let received_at = chrono::Utc::now().naive_utc();

    log::trace!(
        "Received webhook for provider: {}, uid: {}",
        provider_str,
        endpoint_uid
    );

    let provider = match provider_str.as_str() {
        "stripe" => InvoicingProviderEnum::Stripe,
        // add other providers here
        _ => bail!(errors::AdapterWebhookError::UnknownProvider(provider_str)),
    };

    let tenant_id_str = encoding::base64_decode(&endpoint_uid)
        .change_context(errors::AdapterWebhookError::InvalidEndpointId)?;

    let tenant_id = uuid::Uuid::parse_str(&tenant_id_str)
        .change_context(errors::AdapterWebhookError::InvalidEndpointId)?;

    // - get webhook from storage (db, optional redis cache)
    let provider_config = app_state
        .store
        .find_provider_config(provider.clone(), tenant_id)
        .await
        .change_context(errors::AdapterWebhookError::UnknownEndpointId)?;

    let (parts, body) = req.into_parts();
    let bytes = hyper::body::to_bytes(body)
        .await
        .change_context(errors::AdapterWebhookError::BodyDecodingFailed)?;

    let event_id = uuid::Uuid::now_v7();
    let object_store_key = format!("webhooks/{}/{}/{}", provider_str, endpoint_uid, event_id);

    let object_store_key_clone = object_store_key.clone();
    let bytes_clone = bytes.clone();
    let path = object_store::path::Path::from(object_store_key_clone);

    app_state
        .object_store
        .put(&path, bytes_clone)
        .await
        .change_context(errors::AdapterWebhookError::ObjectStoreUnreachable)?;

    // index in db
    app_state
        .store
        .insert_webhook_in_event(WebhookInEventNew {
            id: event_id,
            received_at,
            attempts: 0,
            action: None,
            key: object_store_key,
            processed: false,
            error: None,
            provider_config_id: provider_config.id,
        })
        .await
        .change_context(errors::AdapterWebhookError::DatabaseError)?;

    // metrics TODO

    // - get adapter
    let adapter = match provider {
        InvoicingProviderEnum::Stripe => app_state.stripe_adapter,
    };

    // - decode body

    let headers = parts.headers.clone();
    let method = parts.method;
    let raw_body = bytes.clone().to_vec();
    let query_params = parts.uri.query().map(String::from);

    let json_body: serde_json::Value = serde_json::from_slice(&raw_body)
        .change_context(errors::AdapterWebhookError::BodyDecodingFailed)?;

    let parsed_request = ParsedRequest {
        headers,
        method,
        json_body,
        query_params,
        raw_body,
    };

    // verify webhook source (signature, origin ip address, bearer ..)
    adapter
        .verify_webhook(
            &parsed_request,
            &SecretString::new(provider_config.webhook_security.secret),
        )
        .await?;
    // TODO save errors in webhook_events db

    let response = adapter.get_optimistic_webhook_response();

    // then process specific event
    tokio::spawn(async move {
        adapter
            .process_webhook_event(&parsed_request, app_state.store.clone())
            .await
    });

    Ok(response)
}

async fn handler_404(uri: Uri) -> impl IntoResponse {
    log::warn!("Not found {}", uri);
    (StatusCode::NOT_FOUND, "Not found")
}
