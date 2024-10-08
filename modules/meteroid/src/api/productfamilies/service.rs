use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::productfamilies::v1::{
    product_families_service_server::ProductFamiliesService, CreateProductFamilyRequest,
    CreateProductFamilyResponse, GetProductFamilyByExternalIdRequest,
    GetProductFamilyByExternalIdResponse, ListProductFamiliesRequest, ListProductFamiliesResponse,
};
use meteroid_store::domain;
use meteroid_store::repositories::ProductFamilyInterface;

use crate::api::productfamilies::error::ProductFamilyApiError;
use crate::api::productfamilies::mapping::product_family::ProductFamilyWrapper;

use super::ProductFamilyServiceComponents;

#[tonic::async_trait]
impl ProductFamiliesService for ProductFamilyServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_product_families(
        &self,
        request: Request<ListProductFamiliesRequest>,
    ) -> Result<Response<ListProductFamiliesResponse>, Status> {
        let tenant_id = &request.tenant()?;

        let families = self
            .store
            .list_product_families(*tenant_id)
            .await
            .map_err(Into::<ProductFamilyApiError>::into)?
            .into_iter()
            .map(|x| ProductFamilyWrapper::from(x).0)
            .collect();

        Ok(Response::new(ListProductFamiliesResponse {
            product_families: families,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn create_product_family(
        &self,
        request: Request<CreateProductFamilyRequest>,
    ) -> Result<Response<CreateProductFamilyResponse>, Status> {
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let rs = self
            .store
            .insert_product_family(
                domain::ProductFamilyNew {
                    name: req.name,
                    external_id: req.external_id,
                    tenant_id,
                },
                Some(actor),
            )
            .await
            .map_err(Into::<ProductFamilyApiError>::into)
            .map(|x| ProductFamilyWrapper::from(x).0)?;

        Ok(Response::new(CreateProductFamilyResponse {
            product_family: Some(rs),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_product_family_by_external_id(
        &self,
        request: Request<GetProductFamilyByExternalIdRequest>,
    ) -> Result<Response<GetProductFamilyByExternalIdResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let rs = self
            .store
            .find_product_family_by_external_id(req.external_id.as_str(), tenant_id)
            .await
            .map_err(Into::<ProductFamilyApiError>::into)
            .map(|x| ProductFamilyWrapper::from(x).0)?;

        Ok(Response::new(GetProductFamilyByExternalIdResponse {
            product_family: Some(rs),
        }))
    }
}
