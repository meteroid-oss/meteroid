use tonic::{Request, Response, Status};

use common_grpc::middleware::server::auth::RequestExt;

use meteroid_grpc::meteroid::api::subscriptions::v1::subscriptions_service_server::SubscriptionsService;

use meteroid_grpc::meteroid::api::subscriptions::v1::{
    CancelSubscriptionRequest, CancelSubscriptionResponse, CreateSubscriptionRequest,
    CreateSubscriptionResponse, CreateSubscriptionsRequest, CreateSubscriptionsResponse,
    GetSlotsValueRequest, GetSlotsValueResponse, ListSubscriptionsRequest,
    ListSubscriptionsResponse, PaginationResponse, SubscriptionDetails, UpdateSlotsRequest,
    UpdateSlotsResponse,
};

use meteroid_store::domain;
use meteroid_store::domain::Identity;
use meteroid_store::repositories::subscriptions::{
    CancellationEffectiveAt, SubscriptionSlotsInterface,
};
use meteroid_store::repositories::SubscriptionInterface;

use crate::api::subscriptions::error::SubscriptionApiError;
use crate::api::subscriptions::{mapping, SubscriptionServiceComponents};
use crate::api::utils::{parse_uuid, parse_uuid_opt};

use crate::parse_uuid;

#[tonic::async_trait]
impl SubscriptionsService for SubscriptionServiceComponents {
    async fn create_subscription(
        &self,
        request: Request<CreateSubscriptionRequest>,
    ) -> Result<Response<CreateSubscriptionResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor()?;

        let inner = request.into_inner();

        let subscription = inner
            .subscription
            .ok_or(SubscriptionApiError::InvalidArgument(
                "No subscription provided".to_string(),
            ))?;

        let subscription = mapping::subscriptions::create_proto_to_domain(subscription, &actor)?;

        let created = self
            .store
            .insert_subscription(subscription, tenant_id)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        let res = mapping::subscriptions::created_domain_to_proto(created)?;

        Ok(Response::new(CreateSubscriptionResponse {
            subscription: Some(res),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn create_subscriptions(
        &self,
        request: Request<CreateSubscriptionsRequest>,
    ) -> Result<Response<CreateSubscriptionsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor()?;

        let inner = request.into_inner();

        let subscriptions = inner
            .subscriptions
            .into_iter()
            .map(|s| mapping::subscriptions::create_proto_to_domain(s, &actor))
            .collect::<Result<Vec<_>, _>>()?;

        let inserted = self
            .store
            .insert_subscription_batch(subscriptions, tenant_id)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        let res = inserted
            .into_iter()
            .map(mapping::subscriptions::created_domain_to_proto)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Response::new(CreateSubscriptionsResponse {
            subscriptions: res,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_subscription_details(
        &self,
        request: Request<
            meteroid_grpc::meteroid::api::subscriptions::v1::GetSubscriptionDetailsRequest,
        >,
    ) -> Result<Response<SubscriptionDetails>, Status> {
        let tenant_id = request.tenant()?;

        let inner = request.into_inner();

        let subscription = self
            .store
            .get_subscription_details(
                tenant_id,
                Identity::UUID(parse_uuid!(inner.subscription_id)?),
            )
            .await
            .map_err(Into::<SubscriptionApiError>::into)
            .map_err(Into::<Status>::into)
            .and_then(mapping::subscriptions::details_domain_to_proto)?;

        Ok(Response::new(subscription))
    }

    #[tracing::instrument(skip_all)]
    async fn list_subscriptions(
        &self,
        request: Request<ListSubscriptionsRequest>,
    ) -> Result<Response<ListSubscriptionsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let customer_id = parse_uuid_opt(&inner.customer_id, "customer_id")?;
        let plan_id = parse_uuid_opt(&inner.plan_id, "plan_id")?;

        let res = self
            .store
            .list_subscriptions(
                tenant_id,
                customer_id.map(Identity::UUID),
                plan_id.map(Identity::UUID),
                domain::PaginationRequest {
                    page: inner.pagination.as_ref().map(|p| p.page).unwrap_or(0),
                    per_page: inner.pagination.as_ref().map(|p| p.per_page),
                },
            )
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        let subscriptions: Vec<meteroid_grpc::meteroid::api::subscriptions::v1::Subscription> = res
            .items
            .into_iter()
            .map(mapping::subscriptions::domain_to_proto)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Response::new(ListSubscriptionsResponse {
            subscriptions,
            pagination: Some(PaginationResponse {
                total_pages: res.total_pages,
                total_items: res.total_results,
            }),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_slots(
        &self,
        request: Request<UpdateSlotsRequest>,
    ) -> Result<Response<UpdateSlotsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let _actor = request.actor()?;

        let inner = request.into_inner();

        let subscription_id = parse_uuid!(inner.subscription_id)?;
        let price_component_id = parse_uuid!(inner.price_component_id)?;

        let added = self
            .store
            .add_slot_transaction(tenant_id, subscription_id, price_component_id, inner.delta)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        Ok(Response::new(UpdateSlotsResponse {
            current_value: added as u32, // TODO
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_slots_value(
        &self,
        request: Request<GetSlotsValueRequest>,
    ) -> Result<Response<GetSlotsValueResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let subscription_id = parse_uuid!(inner.subscription_id)?;
        let price_component_id = parse_uuid!(inner.price_component_id)?;

        let slots = self
            .store
            .get_current_slots_value(tenant_id, subscription_id, price_component_id, None)
            .await
            .map_err(|err| {
                SubscriptionApiError::StoreError(
                    "Failed to retrieve current slots".to_string(),
                    Box::new(err.into_error()),
                )
            })?;

        Ok(Response::new(GetSlotsValueResponse {
            current_value: slots,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn cancel_subscription(
        &self,
        request: Request<CancelSubscriptionRequest>,
    ) -> Result<Response<CancelSubscriptionResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor()?;
        let inner = request.into_inner();

        let subscription = self
            .store
            .cancel_subscription(
                parse_uuid!(inner.subscription_id)?,
                inner.reason,
                CancellationEffectiveAt::EndOfBillingPeriod,
                domain::TenantContext { tenant_id, actor },
            )
            .await
            .map_err(|err| {
                SubscriptionApiError::StoreError(
                    "Failed to cancel subscription".to_string(),
                    Box::new(err.into_error()),
                )
            })?;

        mapping::subscriptions::domain_to_proto(subscription)
            .map(|s| {
                Response::new(CancelSubscriptionResponse {
                    subscription: Some(s),
                })
            })
            .map_err(Into::<Status>::into)
    }
}
