use cornucopia_async::Params;
use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::productfamilies::v1::{
    product_families_service_server::ProductFamiliesService, CreateProductFamilyRequest,
    CreateProductFamilyResponse, GetProductFamilyByExternalIdRequest,
    GetProductFamilyByExternalIdResponse, ListProductFamiliesRequest, ListProductFamiliesResponse,
};
use meteroid_repository as db;

use crate::api::services::productfamilies::error::ProductFamilyServiceError;
use crate::api::services::utils::uuid_gen;
use crate::eventbus::Event;

use super::{mapping, ProductFamilyServiceComponents};

#[tonic::async_trait]
impl ProductFamiliesService for ProductFamilyServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_product_families(
        &self,
        request: Request<ListProductFamiliesRequest>,
    ) -> Result<Response<ListProductFamiliesResponse>, Status> {
        let connection = self.get_connection().await?;

        let families = db::products::list_product_families()
            .bind(&connection, &request.tenant()?)
            .all()
            .await
            .map_err(|e| {
                ProductFamilyServiceError::DatabaseError(
                    "unable to list product families".to_string(),
                    e,
                )
            })?;

        let result = families
            .into_iter()
            .map(mapping::product_family::db_to_server)
            .collect();

        Ok(Response::new(ListProductFamiliesResponse {
            product_families: result,
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
        let client = self.pool.get().await.unwrap();

        let params = db::products::CreateProductFamilyParams {
            id: uuid_gen::v7(),
            name: req.name,
            external_id: req.external_id,
            tenant_id,
        };

        let product_family = db::products::create_product_family()
            .params(&client, &params)
            .one()
            .await
            .map_err(|e| {
                ProductFamilyServiceError::DatabaseError(
                    "unable to create product family".to_string(),
                    e,
                )
            })?;

        let rs = mapping::product_family::db_to_server(product_family.clone());

        let _ = self
            .eventbus
            .publish(Event::product_family_created(
                actor,
                product_family.id,
                tenant_id,
            ))
            .await;

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
        let client = self.pool.get().await.unwrap();

        let params = db::products::GetProductFamilyByExternalIdParams {
            external_id: req.external_id,
            tenant_id,
        };

        let product_family = db::products::get_product_family_by_external_id()
            .params(&client, &params)
            .one()
            .await
            .map_err(|e| {
                ProductFamilyServiceError::DatabaseError(
                    "unable to get product family by api name".to_string(),
                    e,
                )
            })?;

        let rs = mapping::product_family::db_to_server(product_family);
        Ok(Response::new(GetProductFamilyByExternalIdResponse {
            product_family: Some(rs),
        }))
    }
}
