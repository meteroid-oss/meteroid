use common_domain::ids::ProductId;
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::prices::v1::{
    ListPricesByProductRequest, ListPricesByProductResponse, PreviewMatrixUpdateRequest,
    PreviewMatrixUpdateResponse, UpdateMatrixPricesRequest, UpdateMatrixPricesResponse,
    prices_service_server::PricesService,
};
use meteroid_store::repositories::prices::PriceInterface;
use tonic::{Request, Response, Status};

use crate::api::prices::error::PriceApiError;
use crate::api::prices::mapping::prices::{
    PriceWrapper, matrix_preview_from_proto, matrix_price_update_from_proto,
    matrix_update_preview_to_proto,
};

use super::PricesServiceComponents;

#[tonic::async_trait]
impl PricesService for PricesServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_prices_by_product(
        &self,
        request: Request<ListPricesByProductRequest>,
    ) -> Result<Response<ListPricesByProductResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let product_id = ProductId::from_proto(req.product_id)?;

        let prices = self
            .store
            .list_prices_by_product_id(product_id, tenant_id)
            .await
            .map_err(Into::<PriceApiError>::into)?
            .into_iter()
            .map(|p| PriceWrapper::from(p).0)
            .collect();

        Ok(Response::new(ListPricesByProductResponse { prices }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_matrix_prices(
        &self,
        request: Request<UpdateMatrixPricesRequest>,
    ) -> Result<Response<UpdateMatrixPricesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor()?;
        let req = request.into_inner();

        let product_id = ProductId::from_proto(req.product_id.clone())?;
        let update = matrix_price_update_from_proto(&req)?;

        let prices = self
            .services
            .update_matrix_prices(tenant_id, product_id, update, actor)
            .await
            .map_err(Into::<PriceApiError>::into)?
            .into_iter()
            .map(|p| PriceWrapper::from(p).0)
            .collect();

        Ok(Response::new(UpdateMatrixPricesResponse { prices }))
    }

    #[tracing::instrument(skip_all)]
    async fn preview_matrix_update(
        &self,
        request: Request<PreviewMatrixUpdateRequest>,
    ) -> Result<Response<PreviewMatrixUpdateResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let product_id = ProductId::from_proto(req.product_id.clone())?;
        let update = matrix_preview_from_proto(&req)?;

        let preview = self
            .services
            .preview_matrix_update(tenant_id, product_id, &update)
            .await
            .map_err(Into::<PriceApiError>::into)?;

        Ok(Response::new(matrix_update_preview_to_proto(preview)))
    }
}
