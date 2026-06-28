use crate::adapters::types::ParsedRequest;
use crate::{adapters::types::WebhookAdapter, errors};
use axum::{
    body::Body,
    extract::{Path, State},
    http::Request,
    response::{IntoResponse, Response},
};

use crate::api_rest::AppState;
use crate::services::storage::Prefix;
use common_domain::ids::{BaseId, TenantId};
use error_stack::{Report, ResultExt, bail};
use meteroid_store::domain::connectors::ProviderSensitiveData;
use meteroid_store::domain::enums::ConnectorProviderEnum;
use meteroid_store::domain::webhooks::WebhookInEventNew;
use meteroid_store::repositories::connectors::ConnectorsInterface;
use secrecy::SecretString;

/// Upper bound on the inbound webhook body we will buffer. Stripe does not
/// document a hard maximum; observed payloads are well under 500 KB, so 1 MiB
/// leaves ample headroom while rejecting abusive/oversized requests.
const MAX_WEBHOOK_BODY_BYTES: usize = 1024 * 1024;

#[axum::debug_handler]
pub async fn axum_handler(
    Path((tenant_id, connection_alias)): Path<(TenantId, String)>,
    State(app_state): State<AppState>,
    req: Request<Body>,
) -> impl IntoResponse {
    match handler(tenant_id, connection_alias, req, app_state).await {
        Ok(r) => r.into_response(),
        Err(e) => {
            log::error!("Error handling webhook: {e}");
            e.current_context().clone().into_response()
        }
    }
}

async fn handler(
    tenant_id: TenantId,
    connection_alias: String,
    req: Request<Body>,
    app_state: AppState,
) -> Result<Response, Report<errors::AdapterWebhookError>> {
    let received_at = chrono::Utc::now().naive_utc();

    log::info!("Received webhook for tenant: {tenant_id}, connection: {connection_alias}");

    let connector = app_state
        .store
        .get_connector_with_data_by_alias(connection_alias.clone(), tenant_id)
        .await
        .change_context(errors::AdapterWebhookError::UnknownEndpointId)?;

    // - get adapter (reject unsupported providers before doing any work)
    let adapter = match connector.provider {
        ConnectorProviderEnum::Stripe => app_state.stripe_adapter.clone(),
        ConnectorProviderEnum::Hubspot => bail!(errors::AdapterWebhookError::ProviderNotSupported(
            "hubspot".to_owned(),
        )),
        ConnectorProviderEnum::Pennylane => bail!(
            errors::AdapterWebhookError::ProviderNotSupported("pennylane".to_owned(),)
        ),
        ConnectorProviderEnum::Mock => bail!(errors::AdapterWebhookError::ProviderNotSupported(
            "mock".to_owned(),
        )),
    };

    // The signature is verified over the raw bytes, so the whole body is buffered
    // before the caller is authenticated. Cap it to avoid buffering unbounded
    // memory for an unauthenticated request.
    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, MAX_WEBHOOK_BODY_BYTES)
        .await
        .change_context(errors::AdapterWebhookError::PayloadTooLarge)?;

    let headers = parts.headers.clone();
    let method = parts.method;
    let query_params = parts.uri.query().map(String::from);

    let json_body: serde_json::Value = serde_json::from_slice(&bytes)
        .change_context(errors::AdapterWebhookError::BodyDecodingFailed)?;

    let parsed_request = ParsedRequest {
        method,
        headers,
        raw_body: bytes.clone(),
        json_body,
        query_params,
    };

    // Verify the signature before persisting anything, so unauthenticated callers
    // can never write to storage or the database.
    if let Some(ProviderSensitiveData::Stripe(sensitive_data)) = &connector.sensitive {
        adapter
            .verify_webhook(
                &parsed_request,
                &SecretString::from(sensitive_data.webhook_secret.as_str()),
            )
            .await?;
    }

    // Archive the raw body; the worker re-reads it from object storage to process.
    let prefix = Prefix::WebhookArchive {
        connection_alias: connection_alias.clone(),
        tenant_id,
    };

    let uid = app_state
        .object_store
        .store(bytes, prefix.clone())
        .await
        .change_context(errors::AdapterWebhookError::ObjectStoreUnreachable)?;

    let key = format!("{}/{}", prefix.to_path_string(), uid);

    // Provider event id (e.g. Stripe `evt_...`), used to dedup repeated deliveries.
    let event_id = parsed_request
        .json_body
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Persist the audit row and enqueue it in one transaction; a duplicate
    // delivery (same event id) is skipped and returns false.
    let enqueued = app_state
        .services
        .ingest_webhook_in_event(
            WebhookInEventNew {
                id: uid.as_uuid(),
                received_at,
                attempts: 0,
                action: None,
                key,
                error: None,
                provider_config_id: connector.id.as_uuid(),
                event_id,
                processed_at: None,
            },
            tenant_id,
        )
        .await
        .change_context(errors::AdapterWebhookError::DatabaseError)?;

    if !enqueued {
        log::info!("Duplicate webhook ignored (tenant {tenant_id}, connection {connection_alias})");
    }

    // Ack only after the event is durably stored and queued; it is processed
    // asynchronously by the webhook_in worker.
    Ok(adapter.get_optimistic_webhook_response())
}
