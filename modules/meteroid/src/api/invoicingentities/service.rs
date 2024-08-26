use tonic::{Request, Response, Status};
use uuid::Uuid;

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::invoicingentities::v1::{
    invoicing_entities_service_server::InvoicingEntitiesService, CreateInvoicingEntityRequest,
    CreateInvoicingEntityResponse, ListInvoicingEntitiesRequest, ListInvoicingEntitiesResponse,
    UpdateInvoicingEntityRequest, UpdateInvoicingEntityResponse,
};
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;

use crate::api::invoicingentities::error::InvoicingEntitiesApiError;
use crate::api::shared::conversions::ProtoConv;

use super::{mapping, InvoicingEntitiesServiceComponents};

#[tonic::async_trait]
impl InvoicingEntitiesService for InvoicingEntitiesServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_invoicing_entities(
        &self,
        request: Request<ListInvoicingEntitiesRequest>,
    ) -> Result<Response<ListInvoicingEntitiesResponse>, Status> {
        let tenant = request.tenant()?;

        let invoicing_entities = self
            .store
            .list_invoicing_entities(tenant)
            .await
            .map_err(Into::<InvoicingEntitiesApiError>::into)?
            .into_iter()
            .map(mapping::invoicing_entities::domain_to_proto)
            .collect();

        let response = ListInvoicingEntitiesResponse {
            entities: invoicing_entities,
        };

        Ok(Response::new(response))
    }

    async fn create_invoicing_entity(
        &self,
        request: Request<CreateInvoicingEntityRequest>,
    ) -> Result<Response<CreateInvoicingEntityResponse>, Status> {
        let tenant = request.tenant()?;
        let organization = request.organization()?;

        let data = request
            .into_inner()
            .data
            .ok_or_else(|| Status::invalid_argument("Missing data"))?;

        let res = self
            .store
            .create_invoicing_entity(
                mapping::invoicing_entities::proto_to_domain(data),
                tenant,
                organization,
            )
            .await
            .map_err(Into::<InvoicingEntitiesApiError>::into)?;

        Ok(Response::new(CreateInvoicingEntityResponse {
            entity: Some(mapping::invoicing_entities::domain_to_proto(res)),
        }))
    }

    async fn update_invoicing_entity(
        &self,
        request: Request<UpdateInvoicingEntityRequest>,
    ) -> Result<Response<UpdateInvoicingEntityResponse>, Status> {
        let tenant = request.tenant()?;

        let req = request.into_inner();

        let data = req
            .data
            .ok_or_else(|| Status::invalid_argument("Missing data"))?;

        // TODO check if the account entity is used by any invoice

        let res = self
            .store
            .patch_invoicing_entity(
                mapping::invoicing_entities::proto_to_patch_domain(data, Uuid::from_proto(req.id)?),
                tenant,
            )
            .await
            .map_err(Into::<InvoicingEntitiesApiError>::into)?;

        Ok(Response::new(UpdateInvoicingEntityResponse {
            entity: Some(mapping::invoicing_entities::domain_to_proto(res)),
        }))
    }
}
