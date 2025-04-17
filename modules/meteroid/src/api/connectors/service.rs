use crate::api::connectors::error::ConnectorApiError;
use crate::api::connectors::{ConnectorsServiceComponents, mapping};
use crate::api::utils::parse_referer;
use common_domain::ids::ConnectorId;
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::connectors::v1::connectors_service_server::ConnectorsService;
use meteroid_grpc::meteroid::api::connectors::v1::{
    ConnectHubspotRequest, ConnectHubspotResponse, ConnectStripeRequest, ConnectStripeResponse,
    ConnectorTypeEnum, DisconnectConnectorRequest, DisconnectConnectorResponse,
    ListConnectorsRequest, ListConnectorsResponse, UpdateHubspotConnectorRequest,
    UpdateHubspotConnectorResponse,
};
use meteroid_oauth::model::OauthProvider;
use meteroid_store::domain::connectors::HubspotPublicData;
use meteroid_store::domain::oauth::{CrmData, OauthVerifierData};
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
                .map(|x| mapping::connectors::connector_meta_to_server(&x))
                .collect(),
        };

        Ok(Response::new(response))
    }

    async fn disconnect_connector(
        &self,
        request: Request<DisconnectConnectorRequest>,
    ) -> Result<Response<DisconnectConnectorResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let connector_id: ConnectorId = ConnectorId::from_proto(&req.id)?;

        self.store
            .delete_connector(connector_id, tenant_id)
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(DisconnectConnectorResponse {}))
    }

    async fn connect_stripe(
        &self,
        request: Request<ConnectStripeRequest>,
    ) -> Result<Response<ConnectStripeResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let data = req.data.ok_or(ConnectorApiError::MissingArgument(
            "Missing stripe data".to_string(),
        ))?;

        let sensitive_data = mapping::connectors::stripe_data_to_domain(&data);

        let res = self
            .store
            .connect_stripe(
                tenant_id,
                data.alias,
                data.api_publishable_key,
                sensitive_data,
            )
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(ConnectStripeResponse {
            connector: Some(mapping::connectors::connector_meta_to_server(&res)),
        }))
    }

    async fn connect_hubspot(
        &self,
        request: Request<ConnectHubspotRequest>,
    ) -> Result<Response<ConnectHubspotResponse>, Status> {
        let tenant_id = request.tenant()?;

        let referer = parse_referer(&request)?;

        let url = self
            .store
            .oauth_auth_url(
                OauthProvider::Hubspot,
                OauthVerifierData::Crm(CrmData { tenant_id, referer }),
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

        let data = req.data.ok_or(ConnectorApiError::MissingArgument(
            "Missing hubspot data".to_string(),
        ))?;

        let connector = self
            .store
            .update_hubspot_connector(
                connector_id,
                tenant_id,
                HubspotPublicData {
                    auto_sync: data.auto_sync,
                },
            )
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(UpdateHubspotConnectorResponse {
            connector: Some(mapping::connectors::connector_to_server(&connector)),
        }))
    }
}
