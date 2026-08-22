use crate::api::connectors::error::ConnectorApiError;
use crate::api::connectors::{ConnectorsServiceComponents, mapping};
use common_domain::ids::ConnectorId;
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::connectors::v1::connectors_service_server::ConnectorsService;
use meteroid_grpc::meteroid::api::connectors::v1::{
    ConnectGoCardlessRequest, ConnectGoCardlessResponse, ConnectHubspotRequest,
    ConnectHubspotResponse, ConnectPennylaneRequest, ConnectPennylaneResponse,
    ConnectStancerRequest, ConnectStancerResponse, ConnectStripeRequest, ConnectStripeResponse,
    ConnectorTypeEnum, DisconnectConnectorRequest, DisconnectConnectorResponse,
    ListConnectorsRequest, ListConnectorsResponse, UpdateHubspotConnectorRequest,
    UpdateHubspotConnectorResponse,
};
use meteroid_oauth::model::OauthProvider;
use meteroid_store::domain::connectors::HubspotPublicData;
use meteroid_store::domain::oauth::{ConnectHubspotData, ConnectPennylaneData, OauthVerifierData};
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::oauth::OauthInterface;
use secrecy::{ExposeSecret, SecretString};
use stancer_client::client::StancerClient;
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

    async fn connect_stripe(
        &self,
        request: Request<ConnectStripeRequest>,
    ) -> Result<Response<ConnectStripeResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let data = req.data.ok_or(ConnectorApiError::MissingArgument(
            "Missing stripe data".to_string(),
        ))?;

        let mut sensitive_data = mapping::connectors::stripe_data_to_domain(&data);

        let account_id = self
            .services
            .get_stripe_account_id(&sensitive_data)
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        // Auto-register the webhook endpoint when the user didn't paste one
        // and provided a URL we should listen on. The Stripe API key needs
        // the "Webhook Endpoints (write)" scope; if it doesn't, we surface
        // the error and the user can fall back to pasting a secret manually.
        let mut auto_registered_endpoint_id: Option<String> = None;
        if sensitive_data.webhook_secret.is_empty() {
            if let Some(url) = req.auto_register_webhook_url.as_deref() {
                validate_auto_register_webhook_url(url)?;
                let registered = auto_register_stripe_webhook(
                    tenant_id,
                    &data.alias,
                    &sensitive_data,
                    &account_id,
                    url,
                    &data.api_publishable_key,
                )
                .await
                .map_err(|e| {
                    log::warn!(
                        "Auto-registering Stripe webhook for alias {} failed: {e:?}",
                        data.alias
                    );
                    ConnectorApiError::InvalidArgument(format!(
                        "Stripe webhook auto-registration failed: {}. Paste a webhook \
                         secret manually, or grant the API key the Webhook Endpoints \
                         (write) scope.",
                        e.current_context()
                    ))
                })?;
                sensitive_data.webhook_secret = registered.secret;
                sensitive_data.webhook_endpoint_id = Some(registered.endpoint_id.clone());
                auto_registered_endpoint_id = Some(registered.endpoint_id);
            } else {
                return Err(ConnectorApiError::MissingArgument(
                    "webhook_secret is required when auto_register_webhook_url is not provided"
                        .to_string(),
                )
                .into());
            }
        }

        let store_result = self
            .store
            .connect_stripe(
                actor,
                tenant_id,
                data.alias.clone(),
                data.api_publishable_key,
                sensitive_data.clone(),
                account_id,
            )
            .await;

        if let (Err(_), Some(endpoint_id)) = (&store_result, &auto_registered_endpoint_id) {
            // Persistence failed after we already created a live webhook
            // endpoint in the merchant's Stripe account: tear it down rather
            // than leaving it orphaned with its signing secret discarded.
            cleanup_orphaned_stripe_webhook(tenant_id, &data.alias, &sensitive_data, endpoint_id)
                .await;
        }

        let res = store_result.map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(ConnectStripeResponse {
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

    /// Register a Stancer merchant account. `GET /v2/ping` is the lightest
    /// call that fails on a bad/revoked secret key, so we ping before
    /// persisting. No webhook registration: Stancer has no webhook mechanism.
    async fn connect_stancer(
        &self,
        request: Request<ConnectStancerRequest>,
    ) -> Result<Response<ConnectStancerResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let data = req.data.ok_or(ConnectorApiError::MissingArgument(
            "Missing stancer data".to_string(),
        ))?;

        let sensitive_data = mapping::connectors::stancer_data_to_domain(&data)?;

        StancerClient::new()
            .ping(&SecretString::from(sensitive_data.api_secret_key.clone()))
            .await
            .map_err(|e| ConnectorApiError::InvalidArgument(format!("Invalid Stancer key: {e}")))?;

        let res = self
            .store
            .connect_stancer(actor, tenant_id, data.alias, sensitive_data)
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(ConnectStancerResponse {
            connector: mapping::connectors::connector_meta_to_server(&res),
        }))
    }

    /// Register a GoCardless merchant account. Mirrors `connect_stripe` —
    /// validates the proto payload, runs it through the mapping layer, and
    /// asks the store to persist the (encrypted) connector. The frontend
    /// modal asks for the merchant's access token + webhook secret directly
    /// because GoCardless does not expose a programmatic webhook-endpoint
    /// API (those are managed in the dashboard).
    ///
    /// Method name matches tonic's snake_case split of `ConnectGoCardless`.
    async fn connect_go_cardless(
        &self,
        request: Request<ConnectGoCardlessRequest>,
    ) -> Result<Response<ConnectGoCardlessResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let data = req.data.ok_or(ConnectorApiError::MissingArgument(
            "Missing gocardless data".to_string(),
        ))?;

        let (public, sensitive) = mapping::connectors::gocardless_data_to_domain(&data)?;

        self.services
            .validate_gocardless_credentials(&sensitive, public.is_sandbox())
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        let res = self
            .store
            .connect_gocardless(tenant_id, data.alias, public, sensitive)
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(ConnectGoCardlessResponse {
            connector: mapping::connectors::connector_meta_to_server(&res),
        }))
    }
}

/// Validate the URL Stripe will POST webhooks to before registering it: it must
/// parse and use https, with http tolerated only for localhost (local dev). No
/// host allowlisting beyond that.
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

/// Auto-register a Stripe webhook endpoint via the Stripe API.
///
/// Builds a transient `Connector` domain struct (just enough for the
/// adapter to read the API key out of the sensitive blob) and calls
/// `WebhookOps::register_webhook`. The endpoint subscribes to the full
/// event set the adapter knows how to parse — Payments, Mandates, Refunds,
/// Disputes. Returns `(endpoint_id, secret)` — the secret is unwrapped to a
/// plain String because the caller stores it on `StripeSensitiveData`,
/// which gets encrypted-at-rest before being persisted.
async fn auto_register_stripe_webhook(
    tenant_id: common_domain::ids::TenantId,
    alias: &str,
    sensitive_data: &meteroid_store::domain::connectors::StripeSensitiveData,
    account_id: &str,
    webhook_url: &str,
    publishable_key: &str,
) -> Result<
    RegisteredWebhookFlat,
    error_stack::Report<meteroid_store::adapters::payment::ConnectorError>,
> {
    use chrono::Utc;
    use common_domain::ids::{BaseId, ConnectorId};
    use meteroid_store::adapters::payment::events::NormalizedEventSubscription;
    use meteroid_store::adapters::payment::{StripeConnector, WebhookOps};
    use meteroid_store::domain::connectors::{
        Connector, ProviderData, ProviderSensitiveData, StripePublicData,
    };
    use meteroid_store::domain::enums::{ConnectorProviderEnum, ConnectorTypeEnum};
    use secrecy::ExposeSecret;

    // The adapter only reads `sensitive` + `tenant_id` + `id`; the other
    // fields are placeholders. `id` is fresh because the connector hasn't
    // been persisted yet — used only for the idempotency-key derivation.
    let transient = Connector {
        id: ConnectorId::new(),
        created_at: Utc::now().naive_utc(),
        tenant_id,
        alias: alias.to_string(),
        connector_type: ConnectorTypeEnum::PaymentProvider,
        provider: ConnectorProviderEnum::Stripe,
        data: Some(ProviderData::Stripe(StripePublicData {
            api_publishable_key: publishable_key.to_string(),
            account_id: account_id.to_string(),
        })),
        sensitive: Some(ProviderSensitiveData::Stripe(sensitive_data.clone())),
    };

    let registered = StripeConnector::new()
        .register_webhook(
            &transient,
            webhook_url,
            &[
                NormalizedEventSubscription::Payments,
                NormalizedEventSubscription::Mandates,
                NormalizedEventSubscription::Refunds,
                NormalizedEventSubscription::Disputes,
            ],
        )
        .await?;

    Ok(RegisteredWebhookFlat {
        endpoint_id: registered.endpoint_id,
        secret: registered.secret.expose_secret().to_string(),
    })
}

struct RegisteredWebhookFlat {
    endpoint_id: String,
    secret: String,
}

/// Best-effort cleanup of a Stripe webhook endpoint we auto-registered but
/// never got to attach to a persisted connector (e.g. a duplicate-alias
/// insert failure). Logs and swallows its own errors — the caller has
/// already failed with the real error and must not have it masked by a
/// cleanup failure.
async fn cleanup_orphaned_stripe_webhook(
    tenant_id: common_domain::ids::TenantId,
    alias: &str,
    sensitive_data: &meteroid_store::domain::connectors::StripeSensitiveData,
    endpoint_id: &str,
) {
    use chrono::Utc;
    use common_domain::ids::{BaseId, ConnectorId};
    use meteroid_store::adapters::payment::{StripeConnector, WebhookOps};
    use meteroid_store::domain::connectors::{Connector, ProviderSensitiveData};
    use meteroid_store::domain::enums::{ConnectorProviderEnum, ConnectorTypeEnum};

    let transient = Connector {
        id: ConnectorId::new(),
        created_at: Utc::now().naive_utc(),
        tenant_id,
        alias: alias.to_string(),
        connector_type: ConnectorTypeEnum::PaymentProvider,
        provider: ConnectorProviderEnum::Stripe,
        data: None,
        sensitive: Some(ProviderSensitiveData::Stripe(sensitive_data.clone())),
    };

    if let Err(e) = StripeConnector::new()
        .unregister_webhook(&transient, endpoint_id)
        .await
    {
        log::warn!(
            "Failed to clean up orphaned Stripe webhook endpoint {endpoint_id} for alias \
             {alias} after connector persistence failure: {e:?}"
        );
    }
}
