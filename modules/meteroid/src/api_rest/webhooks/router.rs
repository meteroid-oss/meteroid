use axum::{
    body::Body,
    extract::{Path, State},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};

use crate::api_rest::AppState;
use crate::api_rest::webhooks::event_handler;
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

/// Webhook router. Steps in order:
///
/// 1. Look up the connector by `(tenant_id, connection_alias)`.
/// 2. Read the raw payload.
/// 3. Resolve the connector adapter via [`initialize_payment_connector`].
/// 4. **Verify the signature** — adapter enforces signature + replay tolerance.
///    Bad signatures NEVER reach the object store or the database.
/// 5. **Archive** the verified payload to the object store (forensics).
/// 6. **Parse** into normalized events (a single delivery can batch several).
/// 7. For each event: skip if we already recorded it; otherwise process it
///    **synchronously**, recording it on success.
/// 8. ACK 200 if every event was handled (or skipped). If any event hit a
///    *transient* failure, return 5xx so the provider redelivers — the events
///    that did succeed are recorded and skipped on the retry, so only the
///    failed one is reprocessed.
///
/// Order rationale:
/// - Verify *before* any write (object store or DB) so an attacker who knows a
///   tenant + alias can't run up storage/egress or table growth with unsigned
///   payloads.
/// - Record a row only *after* the handler succeeds (or permanently rejects the
///   event). This is what makes delivery resilient: a transient handler failure
///   leaves no row, returns 5xx, and the provider's redelivery reprocesses it —
///   rather than being silently deduped away and lost forever. Handlers are
///   idempotent, so reprocessing is safe.
async fn handler(
    tenant_id: TenantId,
    connection_alias: String,
    req: Request<Body>,
    app_state: AppState,
) -> Result<Response, Report<errors::AdapterWebhookError>> {
    let received_at = chrono::Utc::now().naive_utc();

    log::trace!("Received webhook for tenant: {tenant_id}, connection: {connection_alias}");

    let connector = app_state
        .store
        .get_connector_with_data_by_alias(connection_alias.clone(), tenant_id)
        .await
        .change_context(errors::AdapterWebhookError::UnknownEndpointId)?;

    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .change_context(errors::AdapterWebhookError::BodyDecodingFailed)?;

    let connector_impl = initialize_payment_connector(&connector).map_err(|_| {
        Report::new(errors::AdapterWebhookError::ProviderNotSupported(format!(
            "{:?}",
            connector.provider
        )))
    })?;

    let headers = parts.headers;
    let raw_body = bytes.to_vec();

    let secret = webhook_secret(&connector)?;

    // Verify BEFORE we touch the object store or the database: an unsigned /
    // forged payload must cost us nothing (no storage, no rows).
    connector_impl
        .verify_signature(&connector, &raw_body, &headers, &secret)
        .map_err(|_| Report::new(errors::AdapterWebhookError::SignatureVerificationFailed))?;

    // Forensic archive of the verified payload.
    let prefix = Prefix::WebhookArchive {
        connection_alias: connection_alias.clone(),
        tenant_id,
    };
    let uid = app_state
        .object_store
        .store(bytes.clone(), prefix.clone())
        .await
        .change_context(errors::AdapterWebhookError::ObjectStoreUnreachable)?;
    let key = format!("{}/{}", prefix.to_path_string(), uid);

    // A single delivery can carry several events (GoCardless batches). Parse
    // them all — dropping any would lose it permanently once we ACK.
    let events = connector_impl
        .parse_events(&connector, &raw_body, &headers)
        .map_err(|_| Report::new(errors::AdapterWebhookError::BodyDecodingFailed))?;

    let response = (StatusCode::OK, "OK").into_response();

    if events.is_empty() {
        log::debug!(
            "Webhook for connector {} carried no actionable events; archived for forensics",
            connector.id
        );
        return Ok(response);
    }

    let mut retry_needed = false;

    for event in events {
        let provider_event_id = event.provider_event_id.clone();

        // Skip events we've already recorded (processed or permanently
        // rejected). Cheap indexed lookup; avoids redundant provider calls on a
        // redelivery triggered by a *different* event in the same batch failing.
        match app_state
            .services
            .find_webhook_in_event(connector.id.as_uuid(), &provider_event_id)
            .await
            .change_context(errors::AdapterWebhookError::DatabaseError)?
        {
            Some(_) => {
                log::info!(
                    "Webhook event {} for connector {} already recorded; skipping",
                    provider_event_id,
                    connector.id
                );
                continue;
            }
            None => {}
        }

        let provider_event_type = event.provider_event_type.clone();

        let result = event_handler::handle_normalized_event(
            event,
            &connector,
            connector_impl.as_ref(),
            app_state.store.clone(),
        )
        .await;

        match result {
            Ok(()) => {
                // Record success so a redelivery is deduped away.
                record_event(
                    &app_state,
                    &connector,
                    &key,
                    received_at,
                    &provider_event_type,
                    &provider_event_id,
                    true,
                    None,
                )
                .await?;
            }
            Err(e) if is_transient(e.current_context()) => {
                // Leave no row and signal the provider to redeliver. Idempotent
                // handlers make the eventual reprocess safe.
                log::error!(
                    "Transient failure handling webhook event {} (connector {}): {e:?}",
                    provider_event_id,
                    connector.id
                );
                retry_needed = true;
            }
            Err(e) => {
                // Permanent / unprocessable: record it (so we don't hammer the
                // provider with 5xx forever, which can get our endpoint
                // disabled) and move on. The raw payload is archived for replay.
                log::error!(
                    "Permanent failure handling webhook event {} (connector {}); recording and \
                     skipping: {e:?}",
                    provider_event_id,
                    connector.id
                );
                record_event(
                    &app_state,
                    &connector,
                    &key,
                    received_at,
                    &provider_event_type,
                    &provider_event_id,
                    false,
                    Some(format!("{e:?}")),
                )
                .await?;
            }
        }
    }

    if retry_needed {
        // 5xx → the provider redelivers the whole batch; already-recorded events
        // are skipped above, so only the transiently-failed ones reprocess.
        return Err(Report::new(errors::AdapterWebhookError::StoreError)
            .attach("one or more webhook events failed transiently; requesting redelivery"));
    }

    Ok(response)
}

/// Record an inbound event in `webhook_in_event` (idempotent on
/// `(provider_config_id, provider_event_id)`). `processed=true` for a handled
/// event, `false` for one we permanently rejected.
#[allow(clippy::too_many_arguments)]
async fn record_event(
    app_state: &AppState,
    connector: &Connector,
    key: &str,
    received_at: chrono::NaiveDateTime,
    provider_event_type: &str,
    provider_event_id: &str,
    processed: bool,
    error: Option<String>,
) -> Result<(), Report<errors::AdapterWebhookError>> {
    app_state
        .services
        .insert_webhook_in_event_idempotent(WebhookInEventNew {
            id: uuid::Uuid::now_v7(),
            received_at,
            attempts: 1,
            action: Some(provider_event_type.to_string()),
            key: key.to_string(),
            processed,
            error,
            provider_config_id: connector.id.as_uuid(),
            provider_event_id: Some(provider_event_id.to_string()),
        })
        .await
        .change_context(errors::AdapterWebhookError::DatabaseError)?;
    Ok(())
}

/// Whether a handler error is worth asking the provider to redeliver. Transient
/// = infrastructure hiccup that a retry can clear. Permanent = the event is
/// unprocessable as-is (bad / missing metadata), so retrying forever only risks
/// the provider disabling our endpoint.
fn is_transient(e: &errors::AdapterWebhookError) -> bool {
    matches!(
        e,
        errors::AdapterWebhookError::DatabaseError
            | errors::AdapterWebhookError::StoreError
            | errors::AdapterWebhookError::ProviderError
            | errors::AdapterWebhookError::ObjectStoreUnreachable
    )
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

#[allow(dead_code)]
fn _silence_unused_provider_enum() {
    let _ = ConnectorProviderEnum::Stripe;
}
