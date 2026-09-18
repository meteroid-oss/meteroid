use axum::{
    body::{Body, Bytes},
    extract::{Path, State},
    http::Request,
    response::{IntoResponse, Response},
};

use crate::api_rest::AppState;
use crate::errors;
use crate::services::storage::Prefix;
use common_domain::ids::{BaseId, TenantId};
use error_stack::{Report, ResultExt};
use meteroid_store::adapters::payment::connector::REQUEST_QUERY_HEADER;
use meteroid_store::adapters::payment::initialize_payment_connector;
use meteroid_store::domain::webhooks::WebhookInEventNew;
use meteroid_store::repositories::connectors::ConnectorsInterface;

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
            if matches!(
                e.current_context(),
                errors::AdapterWebhookError::UnknownEndpointId
            ) {
                log::warn!("Webhook received for unregistered endpoint: {e}");
            } else {
                log::error!("Error handling webhook: {e}");
            }
            e.current_context().clone().into_response()
        }
    }
}

/// Verify → archive → enqueue, then ack 200. The event is processed
/// asynchronously by the `webhook_in` worker (dequeue → parse → dispatch),
/// which retries on failure via pgmq. Verifying before any write means an
/// unauthenticated caller can never write to storage or the database.
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

    // Resolve the multi-provider connector impl (reject unsupported providers
    // before doing any work).
    let connector_impl = initialize_payment_connector(&connector).map_err(|_| {
        Report::new(errors::AdapterWebhookError::ProviderNotSupported(format!(
            "{:?}",
            connector.provider
        )))
    })?;

    // The signature is verified over the raw bytes, so the whole body is buffered
    // before the caller is authenticated. Cap it to avoid buffering unbounded
    // memory for an unauthenticated request.
    let (parts, body) = req.into_parts();
    let raw_body = axum::body::to_bytes(body, MAX_WEBHOOK_BODY_BYTES)
        .await
        .change_context(errors::AdapterWebhookError::PayloadTooLarge)?
        .to_vec();

    let mut headers = parts.headers;

    // Unsigned webhooks (Mollie) authenticate with a per-connector URL token, so pass the query
    // string to the adapter. Any incoming header of that name is stripped first.
    headers.remove(REQUEST_QUERY_HEADER);
    if let Some(query) = parts.uri.query()
        && let Ok(value) = axum::http::HeaderValue::from_str(query)
    {
        headers.insert(REQUEST_QUERY_HEADER, value);
    }

    // Verify the signature before persisting anything, so unauthenticated callers
    // can never write to storage or the database.
    let secret = connector
        .webhook_secret()
        .ok_or_else(|| Report::new(errors::AdapterWebhookError::SignatureNotFound))?;
    connector_impl
        .verify_signature(&connector, &raw_body, &headers, &secret)
        .map_err(|_| Report::new(errors::AdapterWebhookError::SignatureVerificationFailed))?;

    // The adapter splits a delivery into units keyed by provider event id, so the
    // (provider_config_id, event_id) unique index dedupes each and a bad event gets its own pgmq
    // message.
    let units = connector_impl
        .split_delivery(&connector, &raw_body)
        .change_context(errors::AdapterWebhookError::BodyDecodingFailed)?;

    for unit in units {
        // Archive the unit's body; the worker re-reads it from object storage.
        let prefix = Prefix::WebhookArchive {
            connection_alias: connection_alias.clone(),
            tenant_id,
        };

        let uid = app_state
            .object_store
            .store(Bytes::from(unit.body), prefix.clone())
            .await
            .change_context(errors::AdapterWebhookError::ObjectStoreUnreachable)?;

        let key = format!("{}/{}", prefix.to_path_string(), uid);

        // Persist the audit row and enqueue it in one transaction; a duplicate
        // delivery (same provider event id) is skipped and returns false.
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
                    event_id: unit.event_id,
                    processed_at: None,
                },
                tenant_id,
            )
            .await
            .change_context(errors::AdapterWebhookError::DatabaseError)?;

        if !enqueued {
            log::info!(
                "Duplicate webhook event ignored (tenant {tenant_id}, connection {connection_alias})"
            );
        }
    }

    // Ack only after the event is durably stored and queued; it is processed
    // asynchronously by the webhook_in worker.
    Ok((axum::http::StatusCode::OK, "OK").into_response())
}
