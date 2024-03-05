use cornucopia_async::Params;
use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::components::v1::fee::r#type::Fee;
use meteroid_grpc::meteroid::api::components::v1::{
    price_components_service_server::PriceComponentsService, CreatePriceComponentRequest,
    CreatePriceComponentResponse, EditPriceComponentRequest, EditPriceComponentResponse,
    EmptyResponse, ListPriceComponentRequest, ListPriceComponentResponse,
    RemovePriceComponentRequest,
};
use meteroid_repository as db;

use crate::api::services::pricecomponents::error::PriceComponentServiceError;
use crate::eventbus::Event;
use crate::{
    api::services::utils::{parse_uuid, uuid_gen},
    parse_uuid,
};

use super::{mapping, PriceComponentServiceComponents};

#[tonic::async_trait]
impl PriceComponentsService for PriceComponentServiceComponents {
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
        let actor = request.actor()?;
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
            .ok_or_else(|| PriceComponentServiceError::MissingArgument("fee_type".to_string()))
            .and_then(|fee_type| {
                serde_json::to_value(&fee_type).map_err(|e| {
                    PriceComponentServiceError::SerializationError(
                        "Failed to serialize fee_type".to_string(),
                        e,
                    )
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
                PriceComponentServiceError::DatabaseError(
                    "unable to create price component".to_string(),
                    e,
                )
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
                PriceComponentServiceError::DatabaseError("unable to get component".to_string(), e)
            })?;

        transaction.commit().await.map_err(|e| {
            PriceComponentServiceError::DatabaseError("failed to commit transaction".to_string(), e)
        })?;

        let response = mapping::components::db_to_server(component.clone())?;

        let _ = self
            .eventbus
            .publish(Event::price_component_created(
                actor,
                component.id,
                tenant_id,
            ))
            .await;

        Ok(Response::new(CreatePriceComponentResponse {
            component: Some(response),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn edit_price_component(
        &self,
        request: Request<EditPriceComponentRequest>,
    ) -> Result<Response<EditPriceComponentResponse>, Status> {
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let mut connection = self.get_connection().await?;
        let transaction = self.get_transaction(&mut connection).await?;

        let req_component = req
            .component
            .ok_or_else(|| PriceComponentServiceError::MissingArgument("component".to_string()))?;

        let fee_type = req_component
            .fee_type
            .ok_or_else(|| PriceComponentServiceError::MissingArgument("fee_type".to_string()))?;

        let metric_id = fee_type.fee.as_ref().and_then(|f| match f {
            Fee::Rate(_) => None,
            Fee::SlotBased(_) => None,
            Fee::Capacity(c) => c.metric.as_ref().and_then(|m| parse_uuid!(m.id).ok()),
            Fee::UsageBased(u) => u.metric.as_ref().and_then(|m| parse_uuid!(m.id).ok()),
            Fee::Recurring(_) => None,
            Fee::OneTime(_) => None,
        });

        let serialized_fee = serde_json::to_value(fee_type).map_err(|e| {
            PriceComponentServiceError::SerializationError(
                "failed to serialize fee_type".to_string(),
                e,
            )
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
                PriceComponentServiceError::DatabaseError(
                    "unable to edit price component".to_string(),
                    e,
                )
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
                PriceComponentServiceError::DatabaseError("unable to get component".to_string(), e)
            })?;

        transaction.commit().await.map_err(|e| {
            PriceComponentServiceError::DatabaseError("failed to commit transaction".to_string(), e)
        })?;

        let response = mapping::components::db_to_server(component.clone())?;

        let _ = self
            .eventbus
            .publish(Event::price_component_edited(
                actor,
                component.id,
                tenant_id,
            ))
            .await;

        Ok(Response::new(EditPriceComponentResponse {
            component: Some(response),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn remove_price_component(
        &self,
        request: Request<RemovePriceComponentRequest>,
    ) -> Result<Response<EmptyResponse>, Status> {
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let connection = self.get_connection().await?;

        let price_component_id = parse_uuid(&req.price_component_id, "price_component_id")?;

        db::price_components::delete_price_component()
            .params(
                &connection,
                &db::price_components::DeletePriceComponentParams {
                    id: price_component_id.clone(),
                    tenant_id,
                },
            )
            .await
            .map_err(|e| {
                PriceComponentServiceError::DatabaseError(
                    "Unable to remove price component".to_string(),
                    e,
                )
            })?;

        let _ = self
            .eventbus
            .publish(Event::price_component_removed(
                actor,
                price_component_id,
                tenant_id,
            ))
            .await;

        Ok(Response::new(EmptyResponse {}))
    }
}
