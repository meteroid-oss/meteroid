use crate::api::connectors::error::ConnectorApiError;
use crate::api::connectors::{ConnectorsServiceComponents, mapping};
use common_domain::ids::{BaseId, ConnectorId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::connectors::v1::connectors_service_server::ConnectorsService;
use meteroid_grpc::meteroid::api::connectors::v1::{
    ConnectHubspotRequest, ConnectHubspotResponse, ConnectPaymentProviderRequest,
    ConnectPaymentProviderResponse, ConnectPennylaneRequest, ConnectPennylaneResponse,
    ConnectorTypeEnum, DisconnectConnectorRequest, DisconnectConnectorResponse,
    ListConnectorsRequest, ListConnectorsResponse, UpdateHubspotConnectorRequest,
    UpdateHubspotConnectorResponse,
};
use meteroid_oauth::model::OauthProvider;
use meteroid_store::adapters::payment::events::NormalizedEventSubscription;
use meteroid_store::adapters::payment::{ConnectorError, initialize_payment_connector};
use meteroid_store::domain::connectors::{Connector, HubspotPublicData};
use meteroid_store::domain::enums::ConnectorTypeEnum as DomainConnectorType;
use meteroid_store::domain::oauth::{ConnectHubspotData, ConnectPennylaneData, OauthVerifierData};
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::oauth::OauthInterface;
use secrecy::ExposeSecret;
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl ConnectorsService for ConnectorsServiceComponents {
    async fn list_connectors(
        &self,
        request: Request<ListConnectorsRequest>,
    ) -> Result<Response<ListConnectorsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let filter = match req.connector_type {
            Some(connector_type) => {
                let connector_type = ConnectorTypeEnum::try_from(connector_type).map_err(|_| {
                    ConnectorApiError::InvalidArgument("invalid connector type enum".to_string())
                })?;

                Some(mapping::connectors::connector_type_from_server(
                    &connector_type,
                ))
            }
            None => None,
        };

        let connectors = self
            .store
            .list_connectors(filter, tenant_id)
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        let response = ListConnectorsResponse {
            connectors: connectors
                .into_iter()
                .filter_map(|x| mapping::connectors::connector_to_server(&x))
                .collect(),
        };

        Ok(Response::new(response))
    }

    async fn disconnect_connector(
        &self,
        request: Request<DisconnectConnectorRequest>,
    ) -> Result<Response<DisconnectConnectorResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let connector_id: ConnectorId = ConnectorId::from_proto(&req.id)?;

        self.store
            .delete_connector(actor, connector_id, tenant_id)
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(DisconnectConnectorResponse {}))
    }

    /// Checks the credentials with the provider, registers our webhook endpoint where supported,
    /// then persists the encrypted connector.
    async fn connect_payment_provider(
        &self,
        request: Request<ConnectPaymentProviderRequest>,
    ) -> Result<Response<ConnectPaymentProviderResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let credentials = req.credentials.ok_or(ConnectorApiError::MissingArgument(
            "Missing provider credentials".to_string(),
        ))?;
        let credentials = mapping::connectors::credentials_to_domain(credentials)?;

        // Not persisted yet: the id only seeds idempotency keys.
        let mut transient = Connector {
            id: ConnectorId::new(),
            created_at: chrono::Utc::now().naive_utc(),
            tenant_id,
            alias: credentials.alias,
            connector_type: DomainConnectorType::PaymentProvider,
            provider: credentials.provider,
            data: Some(credentials.data),
            sensitive: Some(credentials.sensitive),
        };
        let connector_impl = initialize_payment_connector(&transient)
            .map_err(|e| ConnectorApiError::InvalidArgument(e.current_context().to_string()))?;

        let validated = connector_impl
            .validate_credentials(&transient)
            .await
            .map_err(credential_error)?;
        transient.data = Some(validated);

        let mut registered_endpoint_id: Option<String> = None;
        if connector_impl
            .capabilities()
            .supports_self_webhook_registration
            && transient.webhook_secret().is_none()
        {
            let url = req.auto_register_webhook_url.as_deref().ok_or(
                ConnectorApiError::MissingArgument(
                    "webhook_secret is required when auto_register_webhook_url is not provided"
                        .to_string(),
                ),
            )?;
            validate_auto_register_webhook_url(url)?;
            let registered = connector_impl
                .register_webhook(&transient, url, &ALL_EVENT_SUBSCRIPTIONS)
                .await
                .map_err(|e| {
                    log::warn!(
                        "Auto-registering webhook for alias {} failed: {e:?}",
                        transient.alias
                    );
                    ConnectorApiError::InvalidArgument(format!(
                        "Webhook auto-registration failed: {}. Paste a webhook secret manually, \
                         or grant the API key the scope to create webhook endpoints.",
                        e.current_context()
                    ))
                })?;
            let stored = transient.sensitive.clone().and_then(|s| {
                s.with_registered_webhook(&registered.endpoint_id, &registered.secret)
            });
            let Some(stored) = stored else {
                if let Err(e) = connector_impl
                    .unregister_webhook(&transient, &registered.endpoint_id)
                    .await
                {
                    log::warn!(
                        "Failed to remove webhook endpoint {} whose secret has nowhere to live: {e:?}",
                        registered.endpoint_id
                    );
                }
                return Err(ConnectorApiError::InvalidArgument(
                    "this provider cannot store a self-registered webhook secret".to_string(),
                )
                .into());
            };
            transient.sensitive = Some(stored);
            registered_endpoint_id = Some(registered.endpoint_id);
        }

        let store_result = self
            .store
            .connect_payment_provider(
                actor,
                tenant_id,
                transient.alias.clone(),
                transient.provider.clone(),
                transient.data.clone().expect("validated public data"),
                transient.sensitive.clone().expect("credentials"),
            )
            .await;

        // Persisting failed after creating a live webhook endpoint in the merchant's account:
        // delete it so it isn't left orphaned. Best-effort; never hides the original error.
        if let (Err(_), Some(endpoint_id)) = (&store_result, &registered_endpoint_id)
            && let Err(e) = connector_impl
                .unregister_webhook(&transient, endpoint_id)
                .await
        {
            log::warn!(
                "Failed to clean up orphaned webhook endpoint {endpoint_id} for alias {} after \
                 connector persistence failure: {e:?}",
                transient.alias
            );
        }

        let res = store_result.map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(ConnectPaymentProviderResponse {
            connector: mapping::connectors::connector_meta_to_server(&res),
        }))
    }

    async fn connect_hubspot(
        &self,
        request: Request<ConnectHubspotRequest>,
    ) -> Result<Response<ConnectHubspotResponse>, Status> {
        let tenant_id = request.tenant()?;
        let initiated_by = request.actor().ok();

        let auto_sync = request.into_inner().auto_sync;

        let url = self
            .store
            .oauth_auth_url(
                OauthProvider::Hubspot,
                OauthVerifierData::ConnectHubspot(ConnectHubspotData {
                    tenant_id,
                    auto_sync,
                    initiated_by,
                }),
            )
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(ConnectHubspotResponse {
            auth_url: url.expose_secret().to_owned(),
        }))
    }

    async fn update_hubspot_connector(
        &self,
        request: Request<UpdateHubspotConnectorRequest>,
    ) -> Result<Response<UpdateHubspotConnectorResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let connector_id: ConnectorId = ConnectorId::from_proto(&req.id)?;

        let connector = self
            .store
            .get_connector_with_data(connector_id, tenant_id)
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        let company_id = connector
            .hubspot_data()
            .ok_or(ConnectorApiError::InvalidArgument(
                "missing hubspot data".into(),
            ))?
            .external_company_id
            .clone();

        let connector = self
            .store
            .update_hubspot_connector(
                connector_id,
                tenant_id,
                HubspotPublicData {
                    auto_sync: req.auto_sync,
                    external_company_id: company_id,
                },
            )
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(UpdateHubspotConnectorResponse {
            connector: mapping::connectors::connector_to_server(&connector),
        }))
    }

    async fn connect_pennylane(
        &self,
        request: Request<ConnectPennylaneRequest>,
    ) -> Result<Response<ConnectPennylaneResponse>, Status> {
        let tenant_id = request.tenant()?;
        let initiated_by = request.actor().ok();

        let url = self
            .store
            .oauth_auth_url(
                OauthProvider::Pennylane,
                OauthVerifierData::ConnectPennylane(ConnectPennylaneData {
                    tenant_id,
                    initiated_by,
                }),
            )
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(ConnectPennylaneResponse {
            auth_url: url.expose_secret().to_owned(),
        }))
    }
}

/// All event types the adapters parse; a self-registered endpoint subscribes to all of them.
const ALL_EVENT_SUBSCRIPTIONS: [NormalizedEventSubscription; 4] = [
    NormalizedEventSubscription::Payments,
    NormalizedEventSubscription::Mandates,
    NormalizedEventSubscription::Refunds,
    NormalizedEventSubscription::Disputes,
];

/// Rejected credentials get a user-facing message; an unreachable provider says nothing about
/// the credentials.
fn credential_error(report: error_stack::Report<ConnectorError>) -> ConnectorApiError {
    match report.current_context() {
        ConnectorError::Configuration(message) => ConnectorApiError::InvalidInput(message.clone()),
        ConnectorError::Transport(_) => {
            log::warn!("credential check failed: {report:?}");
            ConnectorApiError::InvalidInput(
                "Couldn't reach the payment provider to verify the credentials. Please try again."
                    .to_string(),
            )
        }
        other => ConnectorApiError::InvalidArgument(other.to_string()),
    }
}

/// Validates the webhook URL before registering it: must parse and use https (http only for
/// localhost). No host allowlist.
fn validate_auto_register_webhook_url(raw: &str) -> Result<(), ConnectorApiError> {
    let parsed = url::Url::parse(raw).map_err(|e| {
        ConnectorApiError::InvalidArgument(format!(
            "auto_register_webhook_url is not a valid URL: {e}"
        ))
    })?;

    let is_localhost = matches!(
        parsed.host_str(),
        Some("localhost" | "127.0.0.1" | "::1" | "[::1]")
    );

    match parsed.scheme() {
        "https" => Ok(()),
        "http" if is_localhost => Ok(()),
        _ => Err(ConnectorApiError::InvalidArgument(
            "auto_register_webhook_url must be an https URL (http is allowed only for localhost)"
                .to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::credential_error;
    use error_stack::Report;
    use meteroid_store::adapters::payment::ConnectorError;

    /// Only a provider refusal blames the credentials; anything else is retryable.
    #[test]
    fn credential_errors_blame_the_credentials_only_on_refusal() {
        let refused = credential_error(Report::new(ConnectorError::Configuration(
            "Mollie rejected this API key".into(),
        )));
        assert!(
            refused
                .to_string()
                .starts_with("Mollie rejected this API key")
        );

        let unreachable =
            credential_error(Report::new(ConnectorError::Transport("timeout".into())));
        assert!(
            unreachable
                .to_string()
                .starts_with("Couldn't reach the payment provider")
        );
    }
}
