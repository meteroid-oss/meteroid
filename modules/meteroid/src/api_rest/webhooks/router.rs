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
use error_stack::{Result, ResultExt, bail};
use meteroid_store::domain::connectors::ProviderSensitiveData;
use meteroid_store::domain::enums::ConnectorProviderEnum;
use meteroid_store::domain::webhooks::WebhookInEventNew;
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::webhooks::WebhooksInterface;
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
            log::error!("Error handling webhook: {}", e);
            e.current_context().clone().into_response()
        }
    }
}

async fn handler(
    tenant_id: TenantId,
    connection_alias: String,
    req: Request<Body>,
    app_state: AppState,
) -> Result<Response, errors::AdapterWebhookError> {
    let received_at = chrono::Utc::now().naive_utc();

    log::trace!(
        "Received webhook for tenant: {}, connection: {}",
        tenant_id,
        connection_alias
    );

    // - get webhook from storage (db, optional redis cache)
    let connector = app_state
        .store
        .get_connector_with_data_by_alias(connection_alias.clone(), tenant_id)
        .await
        .change_context(errors::AdapterWebhookError::UnknownEndpointId)?;

    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .change_context(errors::AdapterWebhookError::BodyDecodingFailed)?;

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
            provider_config_id: connector.id.as_uuid(),
        })
        .await
        .change_context(errors::AdapterWebhookError::DatabaseError)?;

    // metrics TODO

    // - get adapter
    let adapter = match connector.provider {
        ConnectorProviderEnum::Stripe => app_state.stripe_adapter,
        ConnectorProviderEnum::Hubspot => bail!(errors::AdapterWebhookError::ProviderNotSupported(
            "hubspot".to_owned(),
        )),
        ConnectorProviderEnum::Pennylane => bail!(
            errors::AdapterWebhookError::ProviderNotSupported("pennylane".to_owned(),)
        ),
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
    if let Some(ProviderSensitiveData::Stripe(sensitive_data)) = connector.sensitive {
        adapter
            .verify_webhook(
                &parsed_request,
                &SecretString::new(sensitive_data.webhook_secret),
            )
            .await?;
    };

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
