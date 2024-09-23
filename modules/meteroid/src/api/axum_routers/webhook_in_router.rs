use super::AppState;

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
    http::Request,
    response::{IntoResponse, Response},
};
use axum::{routing::post, Router};

use crate::{adapters::types::ParsedRequest, encoding};
use crate::{adapters::types::WebhookAdapter, errors};

use crate::services::storage::Prefix;
use error_stack::{bail, Result, ResultExt};
use meteroid_store::domain::enums::InvoicingProviderEnum;
use meteroid_store::domain::webhooks::WebhookInEventNew;
use meteroid_store::repositories::configs::ConfigsInterface;
use meteroid_store::repositories::webhooks::WebhooksInterface;
use secrecy::SecretString;

pub fn webhook_in_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/:provider/:endpoint_uid", post(axum_handler))
        .layer(DefaultBodyLimit::max(4096))
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

    let prefix = Prefix::WebhookArchive {
        provider_uid: provider_str,
        endpoint_uid,
    };

    let uid = app_state
        .object_store
        .store(bytes.clone(), prefix.clone())
        .await
        .change_context(errors::AdapterWebhookError::ObjectStoreUnreachable)?;

    let key = format!("{}/{}", prefix.to_path_string(), uid);

    // index in db
    app_state
        .store
        .insert_webhook_in_event(WebhookInEventNew {
            id: uid,
            received_at,
            attempts: 0,
            action: None,
            key,
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
        InvoicingProviderEnum::Manual => bail!(errors::AdapterWebhookError::ProviderNotSupported(
            "Manual".into()
        )),
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
