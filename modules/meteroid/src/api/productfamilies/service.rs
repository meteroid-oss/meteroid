use common_domain::ids::ProductFamilyId;
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::productfamilies::v1::{
    CreateProductFamilyRequest, CreateProductFamilyResponse, GetProductFamilyByLocalIdRequest,
    GetProductFamilyByLocalIdResponse, ListProductFamiliesRequest, ListProductFamiliesResponse,
    product_families_service_server::ProductFamiliesService,
};
use meteroid_store::domain;
use meteroid_store::domain::{OrderByRequest, PaginationRequest};
use meteroid_store::repositories::ProductFamilyInterface;
use tonic::{Request, Response, Status};

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
            .list_product_families(
                *tenant_id,
                PaginationRequest {
                    per_page: Some(u32::MAX),
                    page: 0,
                },
                OrderByRequest::IdAsc,
                None,
            )
            .await
            .map_err(Into::<ProductFamilyApiError>::into)?
            .items
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
    async fn get_product_family_by_local_id(
        &self,
        request: Request<GetProductFamilyByLocalIdRequest>,
    ) -> Result<Response<GetProductFamilyByLocalIdResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let rs = if req.local_id.to_lowercase().as_str() == "default" {
            self.store.find_default_product_family(tenant_id)
        } else {
            self.store.find_product_family_by_id(
                ProductFamilyId::from_proto(req.local_id.as_str())?,
                tenant_id,
            )
        };

        let rs = rs
            .await
            .map_err(Into::<ProductFamilyApiError>::into)
            .map(|x| ProductFamilyWrapper::from(x).0)?;

        Ok(Response::new(GetProductFamilyByLocalIdResponse {
            product_family: Some(rs),
        }))
    }
}
