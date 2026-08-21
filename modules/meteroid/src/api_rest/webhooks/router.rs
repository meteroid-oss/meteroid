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
use error_stack::{Report, ResultExt, bail};
use meteroid_store::adapters::payment::initialize_payment_connector;
use meteroid_store::domain::connectors::{Connector, ProviderSensitiveData};
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
    let bytes = axum::body::to_bytes(body, MAX_WEBHOOK_BODY_BYTES)
        .await
        .change_context(errors::AdapterWebhookError::PayloadTooLarge)?;

    let headers = parts.headers;
    let raw_body = bytes.to_vec();

    // Verify the signature before persisting anything, so unauthenticated callers
    // can never write to storage or the database.
    let secret = webhook_secret(&connector)?;
    connector_impl
        .verify_signature(&connector, &raw_body, &headers, &secret)
        .map_err(|_| Report::new(errors::AdapterWebhookError::SignatureVerificationFailed))?;

    // Signature is verified over the original raw body above. Now split the
    // payload into ingest units: for batching providers (GoCardless) one unit
    // per inner event, keyed by the event's own `EV...` id, so the
    // (provider_config_id, event_id) unique index dedups each event and a poison
    // event lands in its own pgmq message (it can't block its siblings). Other
    // providers (Stripe) ingest as a single unit keyed by the top-level id.
    let units = split_ingest_units(connector.provider, &raw_body, bytes)?;

    for unit in units {
        // Archive the unit's body; the worker re-reads it from object storage.
        let prefix = Prefix::WebhookArchive {
            connection_alias: connection_alias.clone(),
            tenant_id,
        };

        let uid = app_state
            .object_store
            .store(unit.body, prefix.clone())
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

/// One inbound webhook, ready to archive + enqueue: its dedup key (the provider
/// event id) and the body the worker will later parse.
struct IngestUnit {
    event_id: Option<String>,
    body: Bytes,
}

/// Split a verified payload into per-event ingest units. GoCardless delivers a
/// `{"events":[...]}` batch with no top-level id, so each inner event becomes
/// its own unit keyed by the event's `EV...` id — this makes the DB dedup index
/// fire per event and isolates a poison event into its own pgmq message. Every
/// other provider ingests as a single unit whose dedup key is the top-level id.
fn split_ingest_units(
    provider: ConnectorProviderEnum,
    raw_body: &[u8],
    full_body: Bytes,
) -> Result<Vec<IngestUnit>, Report<errors::AdapterWebhookError>> {
    let json: serde_json::Value = serde_json::from_slice(raw_body)
        .change_context(errors::AdapterWebhookError::BodyDecodingFailed)?;

    if provider == ConnectorProviderEnum::Gocardless
        && let Some(events) = json.get("events").and_then(|v| v.as_array())
    {
        let mut units = Vec::with_capacity(events.len());
        for ev in events {
            let event_id = ev.get("id").and_then(|v| v.as_str()).map(str::to_string);
            let slice = serde_json::json!({ "events": [ev] });
            let body = serde_json::to_vec(&slice)
                .change_context(errors::AdapterWebhookError::BodyDecodingFailed)?;
            units.push(IngestUnit {
                event_id,
                body: Bytes::from(body),
            });
        }
        return Ok(units);
    }

    let event_id = json.get("id").and_then(|v| v.as_str()).map(str::to_string);
    Ok(vec![IngestUnit {
        event_id,
        body: full_body,
    }])
}

/// Pull the webhook signing secret out of the connector's sensitive blob.
/// One arm per provider variant: each provider stores the secret under its
/// own struct field. Unknown provider variants yield `SignatureNotFound` —
/// the connector should not have reached this code path if it can't sign.
fn webhook_secret(
    connector: &Connector,
) -> Result<SecretString, Report<errors::AdapterWebhookError>> {
    match &connector.sensitive {
        Some(ProviderSensitiveData::Stripe(data)) => {
            Ok(SecretString::from(data.webhook_secret.clone()))
        }
        Some(ProviderSensitiveData::Gocardless(data)) => {
            Ok(SecretString::from(data.webhook_secret.clone()))
        }
        Some(_) => bail!(errors::AdapterWebhookError::ProviderNotSupported(format!(
            "{:?}",
            connector.provider
        ))),
        None => bail!(errors::AdapterWebhookError::SignatureNotFound),
    }
}
