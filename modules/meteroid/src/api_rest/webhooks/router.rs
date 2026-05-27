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

/// Verify → archive → parse → process each event synchronously.
///
/// Two invariants: verify before any write (unsigned payloads cost nothing),
/// and record a row only after the handler succeeds (or permanently rejects) —
/// a transient failure leaves no row and returns 5xx so the provider redelivers
/// and we reprocess. Handlers are idempotent.
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

    let events = connector_impl
        .parse_events(&connector, &raw_body, &headers)
        .map_err(|_| Report::new(errors::AdapterWebhookError::BodyDecodingFailed))?;

    let response = (StatusCode::OK, "OK").into_response();

    if events.is_empty() {
        return Ok(response);
    }

    let mut retry_needed = false;

    for event in events {
        let provider_event_id = event.provider_event_id.clone();

        if app_state
            .services
            .find_webhook_in_event(connector.id.as_uuid(), &provider_event_id)
            .await
            .change_context(errors::AdapterWebhookError::DatabaseError)?
            .is_some()
        {
            continue;
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
                // No row → provider redelivers → reprocess.
                log::error!("Transient webhook failure for event {provider_event_id}: {e:?}");
                retry_needed = true;
            }
            Err(e) => {
                // Unprocessable: record it so we don't 5xx-loop (which can get
                // our endpoint disabled). Raw payload stays archived for replay.
                log::error!(
                    "Permanent webhook failure for event {provider_event_id}; skipping: {e:?}"
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
        // 5xx → provider redelivers the batch; recorded events skip, failed ones reprocess.
        return Err(Report::new(errors::AdapterWebhookError::StoreError)
            .attach("one or more webhook events failed transiently; requesting redelivery"));
    }

    Ok(response)
}

/// `processed=true` for a handled event, `false` for a permanently rejected one.
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

/// Transient (infra) → retry via 5xx; permanent (bad data) → don't, to avoid a
/// 5xx-loop that gets our endpoint disabled.
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
