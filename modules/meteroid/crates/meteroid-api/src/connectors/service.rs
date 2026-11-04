use crate::connectors::error::ConnectorApiError;
use crate::connectors::{ConnectorsServiceComponents, mapping};
use common_domain::ids::ConnectorId;
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::connectors::v1::connectors_service_server::ConnectorsService;
use meteroid_grpc::meteroid::api::connectors::v1::{
    ConnectHubspotRequest, ConnectHubspotResponse, ConnectPaymentProviderRequest,
    ConnectPaymentProviderResponse, ConnectPennylaneRequest, ConnectPennylaneResponse,
    ConnectorIdRequest, ConnectorResponse, ConnectorTypeEnum, CreatePaymentProviderDraftRequest,
    CreatePaymentProviderDraftResponse, DeleteConnectorResponse, DisconnectConnectorRequest,
    DisconnectConnectorResponse, GetConnectorUsageResponse, ListConnectorsRequest,
    ListConnectorsResponse, RenameConnectorRequest, UpdateHubspotConnectorRequest,
    UpdateHubspotConnectorResponse, UpdatePaymentProviderCredentialsRequest,
    UpdatePaymentProviderCredentialsResponse,
};
use meteroid_oauth::model::OauthProvider;
use meteroid_store::domain::connectors::{Connector, HubspotPublicData};
use meteroid_store::domain::oauth::{ConnectHubspotData, ConnectPennylaneData, OauthVerifierData};
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::oauth::OauthInterface;
use secrecy::ExposeSecret;
use tonic::{Request, Response, Status};

impl ConnectorsServiceComponents {
    fn to_server(
        &self,
        connector: &Connector,
    ) -> Option<meteroid_grpc::meteroid::api::connectors::v1::Connector> {
        mapping::connectors::connector_to_server(connector, self.store.settings.webhook_base_url())
    }
}

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
                .iter()
                .filter_map(|x| self.to_server(x))
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

    async fn create_payment_provider_draft(
        &self,
        request: Request<CreatePaymentProviderDraftRequest>,
    ) -> Result<Response<CreatePaymentProviderDraftResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let provider = mapping::connectors::payment_provider_from_server(&req.provider())?;
        let connector = self
            .store
            .create_payment_provider_draft(tenant_id, req.alias, provider)
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(CreatePaymentProviderDraftResponse {
            connector: self.to_server(&connector),
        }))
    }

    async fn update_payment_provider_credentials(
        &self,
        request: Request<UpdatePaymentProviderCredentialsRequest>,
    ) -> Result<Response<UpdatePaymentProviderCredentialsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let req = request.into_inner();

        let id = ConnectorId::from_proto(&req.id)?;
        let credentials = req.credentials.ok_or(ConnectorApiError::MissingArgument(
            "Missing provider credentials".to_string(),
        ))?;
        let credentials = mapping::connectors::credentials_update_to_domain(credentials)?;

        let connector = self
            .store
            .update_payment_provider_credentials(actor, tenant_id, id, credentials)
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(UpdatePaymentProviderCredentialsResponse {
            connector: self.to_server(&connector),
        }))
    }

    async fn activate_connector(
        &self,
        request: Request<ConnectorIdRequest>,
    ) -> Result<Response<ConnectorResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let id = ConnectorId::from_proto(&request.into_inner().id)?;

        let connector = self
            .store
            .activate_connector(actor, tenant_id, id)
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(ConnectorResponse {
            connector: self.to_server(&connector),
        }))
    }

    async fn disable_connector(
        &self,
        request: Request<ConnectorIdRequest>,
    ) -> Result<Response<ConnectorResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let id = ConnectorId::from_proto(&request.into_inner().id)?;

        let connector = self
            .store
            .disable_connector(actor, tenant_id, id)
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(ConnectorResponse {
            connector: self.to_server(&connector),
        }))
    }

    async fn enable_connector(
        &self,
        request: Request<ConnectorIdRequest>,
    ) -> Result<Response<ConnectorResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let id = ConnectorId::from_proto(&request.into_inner().id)?;

        let connector = self
            .store
            .enable_connector(actor, tenant_id, id)
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(ConnectorResponse {
            connector: self.to_server(&connector),
        }))
    }

    async fn delete_connector(
        &self,
        request: Request<ConnectorIdRequest>,
    ) -> Result<Response<DeleteConnectorResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let id = ConnectorId::from_proto(&request.into_inner().id)?;

        self.store
            .delete_connector(actor, id, tenant_id)
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(DeleteConnectorResponse {}))
    }

    async fn rename_connector(
        &self,
        request: Request<RenameConnectorRequest>,
    ) -> Result<Response<ConnectorResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let id = ConnectorId::from_proto(&req.id)?;

        let connector = self
            .store
            .rename_draft_connector(tenant_id, id, req.alias)
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(ConnectorResponse {
            connector: self.to_server(&connector),
        }))
    }

    async fn check_connector(
        &self,
        request: Request<ConnectorIdRequest>,
    ) -> Result<Response<ConnectorResponse>, Status> {
        let tenant_id = request.tenant()?;
        let id = ConnectorId::from_proto(&request.into_inner().id)?;

        let connector = self
            .store
            .check_connector(tenant_id, id)
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(ConnectorResponse {
            connector: self.to_server(&connector),
        }))
    }

    async fn get_connector_usage(
        &self,
        request: Request<ConnectorIdRequest>,
    ) -> Result<Response<GetConnectorUsageResponse>, Status> {
        let tenant_id = request.tenant()?;
        let id = ConnectorId::from_proto(&request.into_inner().id)?;

        let usage = self
            .store
            .connector_usage(tenant_id, id)
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(GetConnectorUsageResponse {
            usage: Some(mapping::connectors::connector_usage_to_server(&usage)),
        }))
    }

    /// One call for API clients: draft, verified credentials, webhook, activation.
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
        let (alias, credentials) = mapping::connectors::credentials_to_domain(credentials)?;

        let res = self
            .store
            .connect_payment_provider(actor, tenant_id, alias, credentials)
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

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
            connector: self.to_server(&connector),
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
