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
/// 2. Read the raw payload, archive it to the object store (forensics).
/// 3. Resolve the connector adapter via [`initialize_payment_connector`].
/// 4. **Verify the signature** — adapter enforces signature + replay tolerance.
///    Bad signatures NEVER touch the database.
/// 5. **Parse** into a normalized event.
/// 6. **Idempotency check**: insert into `webhook_in_event` with the provider
///    event id; the partial unique index on
///    `(provider_config_id, provider_event_id)` returns no rows when the
///    provider redelivered an event we already stored.
/// 7. **ACK 200 immediately** — duplicates and successes both ACK so the
///    provider stops retrying.
/// 8. Spawn an async task to dispatch the event onto the store. Skipped on
///    duplicates.
///
/// Order rationale: verify *before* DB writes prevents an attacker from
/// filling our `webhook_in_event` table with garbage by hammering us with
/// unsigned payloads. Idempotency *before* ACK prevents a "ACK then crash"
/// race from losing an event we never recorded.
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

    // Forensic archive happens first, before any validation, so we can
    // inspect malformed / unsigned payloads after the fact.
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

    let connector_impl = initialize_payment_connector(&connector).map_err(|_| {
        Report::new(errors::AdapterWebhookError::ProviderNotSupported(format!(
            "{:?}",
            connector.provider
        )))
    })?;

    let headers = parts.headers;
    let raw_body = bytes.to_vec();

    let secret = webhook_secret(&connector)?;

    connector_impl
        .verify_signature(&connector, &raw_body, &headers, &secret)
        .map_err(|_| Report::new(errors::AdapterWebhookError::SignatureVerificationFailed))?;

    let parsed = connector_impl
        .parse_event(&connector, &raw_body, &headers)
        .map_err(|_| Report::new(errors::AdapterWebhookError::BodyDecodingFailed))?;

    // Idempotency: try to record the event with its provider id. If the
    // partial unique index rejects the insert (same connector + same event id
    // already seen), `insert_webhook_in_event_idempotent` returns `Ok(None)`
    // and we skip processing.
    let provider_event_id = parsed.as_ref().map(|e| e.provider_event_id.clone());
    let inserted = app_state
        .services
        .insert_webhook_in_event_idempotent(WebhookInEventNew {
            id: uid.as_uuid(),
            received_at,
            attempts: 0,
            action: parsed.as_ref().map(|e| e.provider_event_type.clone()),
            key,
            processed: false,
            error: None,
            provider_config_id: connector.id.as_uuid(),
            provider_event_id,
        })
        .await
        .change_context(errors::AdapterWebhookError::DatabaseError)?;

    let response = (StatusCode::OK, "OK").into_response();

    if inserted.is_none() {
        log::info!(
            "Duplicate webhook delivery for connector {} (event {:?}); skipping",
            connector.id,
            parsed.as_ref().map(|e| &e.provider_event_id),
        );
        return Ok(response);
    }

    if let Some(event) = parsed {
        // The Box from initialize_payment_connector isn't Sync; rebuilding it
        // inside the spawned task is cheap because the underlying HTTP client
        // is a process-wide singleton.
        let store = app_state.store.clone();
        let connector_for_task = connector.clone();
        tokio::spawn(async move {
            let connector_impl = match initialize_payment_connector(&connector_for_task) {
                Ok(c) => c,
                Err(e) => {
                    log::error!("Failed to re-resolve connector in webhook task: {e}");
                    return;
                }
            };
            if let Err(e) = event_handler::handle_normalized_event(
                event,
                &connector_for_task,
                connector_impl.as_ref(),
                store,
            )
            .await
            {
                log::error!("Webhook event handling failed: {e}");
            }
        });
    }

    Ok(response)
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
