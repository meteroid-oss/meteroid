use cornucopia_async::Params;
use meteroid_repository as db;
use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::{
    api::services::utils::{parse_uuid, uuid_gen},
    db::DbService,
    parse_uuid,
};

use super::mapping;
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::components::v1::fee::r#type::Fee;
use meteroid_grpc::meteroid::api::components::v1::{
    price_components_service_server::PriceComponentsService, CreatePriceComponentRequest,
    CreatePriceComponentResponse, EditPriceComponentRequest, EditPriceComponentResponse,
    EmptyResponse, ListPriceComponentRequest, ListPriceComponentResponse,
    RemovePriceComponentRequest,
};

#[tonic::async_trait]
impl PriceComponentsService for DbService {
    #[tracing::instrument(skip_all)]
    async fn list_price_components(
        &self,
        request: Request<ListPriceComponentRequest>,
    ) -> Result<Response<ListPriceComponentResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let components = super::ext::list_price_components(
            parse_uuid!(&req.plan_version_id)?,
            tenant_id,
            &connection,
        )
        .await?;

        let response = ListPriceComponentResponse { components };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn create_price_component(
        &self,
        request: Request<CreatePriceComponentRequest>,
    ) -> Result<Response<CreatePriceComponentResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let mut connection = self.get_connection().await?;
        let transaction = self.get_transaction(&mut connection).await?;

        let metric_id = req
            .fee_type
            .as_ref()
            .and_then(|f| f.fee.as_ref())
            .and_then(|f| match f {
                Fee::Rate(_) => None,
                Fee::SlotBased(_) => None,
                Fee::Capacity(c) => c.metric.as_ref().and_then(|m| parse_uuid!(m.id).ok()),
                Fee::UsageBased(u) => u.metric.as_ref().and_then(|m| parse_uuid!(m.id).ok()),
                Fee::Recurring(_) => None,
                Fee::OneTime(_) => None,
            });

        let serialized_fee = req
            .fee_type
            .ok_or_else(|| Status::invalid_argument("Missing fee_type"))
            .and_then(|fee_type| {
                serde_json::to_value(&fee_type).map_err(|e| {
                    Status::invalid_argument(format!("Failed to serialize fee_type: {}", e))
                })
            })?;

        let id = uuid_gen::v7();
        db::price_components::upsert_price_component()
            .params(
                &transaction,
                &db::price_components::UpsertPriceComponentParams {
                    id,
                    plan_version_id: parse_uuid!(&req.plan_version_id)?,
                    tenant_id,
                    name: &req.name,
                    fee: &serialized_fee,
                    product_item_id: req
                        .product_item_id
                        .map(|product_item_id| parse_uuid!(product_item_id))
                        .transpose()?,
                    billable_metric_id: metric_id,
                },
            )
            .await
            .map_err(|e| {
                Status::internal("Unable to create price component")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let component = db::price_components::get_price_component()
            .params(
                &transaction,
                &db::price_components::GetPriceComponentParams {
                    component_id: id,
                    tenant_id,
                },
            )
            .one()
            .await
            .map_err(|e| {
                Status::internal("Unable to get component")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        transaction.commit().await.map_err(|e| {
            Status::internal("Failed to commit transaction")
                .set_source(Arc::new(e))
                .clone()
        })?;

        let response = mapping::components::db_to_server(component)?;

        Ok(Response::new(CreatePriceComponentResponse {
            component: Some(response),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn edit_price_component(
        &self,
        request: Request<EditPriceComponentRequest>,
    ) -> Result<Response<EditPriceComponentResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let mut connection = self.get_connection().await?;
        let transaction = self.get_transaction(&mut connection).await?;

        let req_component = req
            .component
            .ok_or_else(|| Status::invalid_argument("Missing component"))?;

        let fee_type = req_component
            .fee_type
            .ok_or_else(|| Status::invalid_argument("Missing fee_type"))?;

        let metric_id = fee_type.fee.as_ref().and_then(|f| match f {
            Fee::Rate(_) => None,
            Fee::SlotBased(_) => None,
            Fee::Capacity(c) => c.metric.as_ref().and_then(|m| parse_uuid!(m.id).ok()),
            Fee::UsageBased(u) => u.metric.as_ref().and_then(|m| parse_uuid!(m.id).ok()),
            Fee::Recurring(_) => None,
            Fee::OneTime(_) => None,
        });

        let serialized_fee = serde_json::to_value(fee_type).map_err(|e| {
            Status::invalid_argument(format!("Failed to serialize fee_type: {}", e))
        })?;

        let product_item = req_component.product_item;
        db::price_components::upsert_price_component()
            .params(
                &transaction,
                &db::price_components::UpsertPriceComponentParams {
                    id: parse_uuid!(&req_component.id)?,
                    plan_version_id: parse_uuid!(&req.plan_version_id)?,
                    tenant_id,
                    name: &req_component.name,
                    fee: &serialized_fee,
                    product_item_id: product_item
                        .map(|product_item| parse_uuid!(product_item.id))
                        .transpose()?,
                    billable_metric_id: metric_id,
                },
            )
            //.one()
            .await
            .map_err(|e| {
                Status::internal("Unable to edit price component")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let component = db::price_components::get_price_component()
            .params(
                &transaction,
                &db::price_components::GetPriceComponentParams {
                    component_id: parse_uuid!(&req_component.id)?,
                    tenant_id,
                },
            )
            .one()
            .await
            .map_err(|e| {
                Status::internal("Unable to get component")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        transaction.commit().await.map_err(|e| {
            Status::internal("Failed to commit transaction")
                .set_source(Arc::new(e))
                .clone()
        })?;

        let response = mapping::components::db_to_server(component)?;

        Ok(Response::new(EditPriceComponentResponse {
            component: Some(response),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn remove_price_component(
        &self,
        request: Request<RemovePriceComponentRequest>,
    ) -> Result<Response<EmptyResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        db::price_components::delete_price_component()
            .params(
                &connection,
                &db::price_components::DeletePriceComponentParams {
                    id: parse_uuid(&req.price_component_id, "price_component_id")?,
                    tenant_id,
                },
            )
            .await
            .map_err(|e| {
                Status::internal("Unable to remove price component")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        Ok(Response::new(EmptyResponse {}))
    }
}
