use axum::{
    body::{Body, Bytes},
    extract::{Path, State},
    http::Request,
    response::{IntoResponse, Response},
};

use crate::AppState;
use common_domain::ids::{BaseId, ConnectorId, TenantId};
use error_stack::{Report, ResultExt};
use meteroid_app::errors;
use meteroid_app::services::storage::{self, ObjectKind};
use meteroid_store::adapters::payment::connector::REQUEST_QUERY_HEADER;
use meteroid_store::adapters::payment::initialize_payment_connector;
use meteroid_store::domain::connectors::Connector;
use meteroid_store::domain::enums::ConnectorStatusEnum;
use meteroid_store::domain::webhooks::WebhookInEventNew;
use meteroid_store::repositories::connectors::ConnectorsInterface;

/// Upper bound on the inbound webhook body we will buffer. Stripe does not
/// document a hard maximum; observed payloads are well under 500 KB, so 1 MiB
/// leaves ample headroom while rejecting abusive/oversized requests.
const MAX_WEBHOOK_BODY_BYTES: usize = 1024 * 1024;

/// Legacy URL: `/webhooks/v1/{tenant_id}/{alias}`.
#[axum::debug_handler]
pub async fn axum_handler(
    Path((tenant_id, connection_alias)): Path<(TenantId, String)>,
    State(app_state): State<AppState>,
    req: Request<Body>,
) -> impl IntoResponse {
    log::info!("Received webhook for tenant: {tenant_id}, connection: {connection_alias}");
    let connector = app_state
        .store
        .get_connector_with_data_by_alias(connection_alias, tenant_id)
        .await
        .change_context(errors::AdapterWebhookError::UnknownEndpointId);
    respond(connector, req, app_state).await
}

/// `/webhooks/v1/payments/{connector_id}`.
#[axum::debug_handler]
pub async fn axum_payments_handler(
    Path(connector_id): Path<ConnectorId>,
    State(app_state): State<AppState>,
    req: Request<Body>,
) -> impl IntoResponse {
    log::info!("Received webhook for connector: {connector_id}");
    // The URL carries no tenant, so the connector is looked up across tenants. Nothing is trusted
    // or written until `handler` verifies the request against that connector's secret.
    let connector = app_state
        .store
        .get_connector_unscoped(connector_id)
        .await
        .change_context(errors::AdapterWebhookError::UnknownEndpointId);
    respond(connector, req, app_state).await
}

async fn respond(
    connector: Result<Connector, Report<errors::AdapterWebhookError>>,
    req: Request<Body>,
    app_state: AppState,
) -> Response {
    match async { handler(connector?, req, app_state).await }.await {
        Ok(r) => r.into_response(),
        Err(e) => {
            if matches!(
                e.current_context(),
                errors::AdapterWebhookError::UnknownEndpointId
                    | errors::AdapterWebhookError::ConnectorDeleted
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
    connector: Connector,
    req: Request<Body>,
    app_state: AppState,
) -> Result<Response, Report<errors::AdapterWebhookError>> {
    let received_at = common_domain::clock::now().naive_utc();
    let tenant_id = connector.tenant_id;

    // Disabled connectors keep receiving: payments already in flight must still settle.
    if connector.status == ConnectorStatusEnum::Deleted {
        return Err(Report::new(errors::AdapterWebhookError::ConnectorDeleted));
    }

    // Resolve the multi-provider connector impl (reject unsupported providers
    // before doing any work).
    let connector_impl = initialize_payment_connector(&connector).map_err(|_| {
        Report::new(errors::AdapterWebhookError::ProviderNotSupported(format!(
            "{:?}",
            connector.provider()
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
        .verify_signature(&raw_body, &headers, &secret)
        .map_err(|_| Report::new(errors::AdapterWebhookError::SignatureVerificationFailed))?;

    // A draft is still being set up: acknowledge (so the provider's "send test webhook" passes)
    // but process nothing.
    if connector.status == ConnectorStatusEnum::Draft {
        log::info!(
            "Webhook for draft connector {} acknowledged, not processed",
            connector.id
        );
        return Ok((axum::http::StatusCode::OK, "OK").into_response());
    }

    // The adapter splits a delivery into units keyed by provider event id, so the
    // (provider_config_id, event_id) unique index dedupes each and a bad event gets its own pgmq
    // message.
    let units = connector_impl
        .split_delivery(&raw_body)
        .change_context(errors::AdapterWebhookError::BodyDecodingFailed)?;

    for unit in units {
        // Archive the unit's body; the worker re-reads it from object storage.
        let stored = storage::put(
            app_state.object_store.as_ref(),
            &app_state.store,
            ObjectKind::WebhookArchive {
                connector: connector.id.as_base62(),
            },
            tenant_id,
            Bytes::from(unit.body),
        )
        .await
        .change_context(errors::AdapterWebhookError::ObjectStoreUnreachable)?;

        // Persist the audit row and enqueue it in one transaction; a duplicate
        // delivery (same provider event id) is skipped and returns false.
        let enqueued = app_state
            .services
            .ingest_webhook_in_event(
                WebhookInEventNew {
                    id: stored.id.as_uuid(),
                    received_at,
                    attempts: 0,
                    action: None,
                    key: stored.key,
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
                "Duplicate webhook event ignored (tenant {tenant_id}, connector {})",
                connector.id
            );
        }
    }

    // Ack only after the event is durably stored and queued; it is processed
    // asynchronously by the webhook_in worker.
    Ok((axum::http::StatusCode::OK, "OK").into_response())
}
