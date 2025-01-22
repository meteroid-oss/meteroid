use crate::api::connectors::error::ConnectorApiError;
use crate::api::connectors::{mapping, ConnectorsServiceComponents};
use crate::{api::utils::parse_uuid, parse_uuid};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::connectors::v1::connectors_service_server::ConnectorsService;
use meteroid_grpc::meteroid::api::connectors::v1::{
    ConnectStripeRequest, ConnectStripeResponse, ConnectorTypeEnum, DisconnectConnectorRequest,
    DisconnectConnectorResponse, ListConnectorsRequest, ListConnectorsResponse,
};
use meteroid_store::repositories::connectors::ConnectorsInterface;
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

        let connector_id = parse_uuid!(&req.id)?;

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
            .connect_stripe(tenant_id, data.alias, sensitive_data)
            .await
            .map_err(Into::<ConnectorApiError>::into)?;

        Ok(Response::new(ConnectStripeResponse {
            connector: Some(mapping::connectors::connector_meta_to_server(&res)),
        }))
    }
}
