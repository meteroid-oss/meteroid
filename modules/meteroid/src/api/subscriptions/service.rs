use common_domain::ids::{CustomerId, PlanId, PriceComponentId, SubscriptionId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_store::repositories::subscriptions::slots::SubscriptionSlotsInterfaceAuto;
use tonic::{Request, Response, Status};

use meteroid_grpc::meteroid::api::subscriptions::v1::subscriptions_service_server::SubscriptionsService;

use meteroid_grpc::meteroid::api::subscriptions::v1::{
    CancelSubscriptionRequest, CancelSubscriptionResponse, CreateSubscriptionRequest,
    CreateSubscriptionResponse, CreateSubscriptionsRequest, CreateSubscriptionsResponse,
    GetSlotsValueRequest, GetSlotsValueResponse, ListSubscriptionsRequest,
    ListSubscriptionsResponse, SubscriptionDetails, SyncToHubspotRequest, SyncToHubspotResponse,
    UpdateSlotsRequest, UpdateSlotsResponse,
};

use crate::api::shared::conversions::ProtoConv;
use crate::api::subscriptions::error::SubscriptionApiError;
use crate::api::subscriptions::{SubscriptionServiceComponents, mapping};
use crate::api::utils::PaginationExt;
use meteroid_store::repositories::SubscriptionInterface;
use meteroid_store::repositories::subscriptions::{
    CancellationEffectiveAt, SubscriptionInterfaceAuto,
};

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
            .services
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
            .services
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
                SubscriptionId::from_proto(inner.subscription_id)?,
            )
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        let subscription = mapping::subscriptions::details_domain_to_proto(subscription)?;

        Ok(Response::new(subscription))
    }

    #[tracing::instrument(skip_all)]
    async fn list_subscriptions(
        &self,
        request: Request<ListSubscriptionsRequest>,
    ) -> Result<Response<ListSubscriptionsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let customer_id = CustomerId::from_proto_opt(inner.customer_id.as_ref())?;
        let plan_id = PlanId::from_proto_opt(inner.plan_id)?;

        let status_filter: Vec<_> = inner
            .status
            .into_iter()
            .filter_map(|s| {
                meteroid_grpc::meteroid::api::subscriptions::v1::SubscriptionStatus::try_from(s)
                    .ok()
            })
            .flat_map(mapping::subscriptions::map_proto_status_to_domain)
            .collect();

        let res = self
            .store
            .list_subscriptions(
                tenant_id,
                customer_id,
                plan_id,
                if status_filter.is_empty() {
                    None
                } else {
                    Some(status_filter)
                },
                inner.pagination.into_domain(),
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
            pagination_meta: inner
                .pagination
                .into_response(res.total_pages, res.total_results),
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

        let subscription_id = SubscriptionId::from_proto(inner.subscription_id.clone())?;
        let price_component_id = PriceComponentId::from_proto(inner.price_component_id.clone())?;

        use meteroid_grpc::meteroid::api::subscriptions::v1::SlotUpgradeBillingMode as ProtoMode;
        use meteroid_store::domain::SlotUpgradeBillingMode;

        // default to Optimistic
        let billing_mode = match inner.billing_mode() {
            ProtoMode::SlotOptimistic => SlotUpgradeBillingMode::Optimistic,
            ProtoMode::SlotOnCheckout => SlotUpgradeBillingMode::OnCheckout,
            ProtoMode::SlotOnInvoicePaid => SlotUpgradeBillingMode::OnInvoicePaid,
        };

        let result = self
            .services
            .update_subscription_slots(
                tenant_id,
                subscription_id,
                price_component_id,
                inner.delta,
                billing_mode,
            )
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        Ok(Response::new(UpdateSlotsResponse {
            current_value: result.new_slot_count as u32,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_slots_value(
        &self,
        request: Request<GetSlotsValueRequest>,
    ) -> Result<Response<GetSlotsValueResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(inner.subscription_id)?;

        let slots = self
            .store
            .get_active_slots_value(tenant_id, subscription_id, inner.unit, None)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

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

        use meteroid_grpc::meteroid::api::subscriptions::v1::cancel_subscription_request::EffectiveAt;

        let effective_at = match inner.effective_at {
            Some(EffectiveAt::Immediate(_)) => {
                // For now, immediate cancellation maps to cancelling today, we could do sync
                CancellationEffectiveAt::Date(chrono::Utc::now().date_naive())
            }
            Some(EffectiveAt::Date(date)) => {
                let date = chrono::NaiveDate::from_proto(date)?;
                CancellationEffectiveAt::Date(date)
            }
            Some(EffectiveAt::BillingPeriodEnd(_)) | None => {
                CancellationEffectiveAt::EndOfBillingPeriod
            }
        };

        let subscription = self
            .services
            .cancel_subscription(
                SubscriptionId::from_proto(inner.subscription_id)?,
                tenant_id,
                inner.reason,
                effective_at,
                actor,
            )
            .await
            .map_err(|err| {
                SubscriptionApiError::StoreError(
                    "Failed to cancel subscription".to_string(),
                    Box::new(err.into_error()),
                )
            })?;

        mapping::subscriptions::domain_to_proto(subscription).map(|s| {
            Response::new(CancelSubscriptionResponse {
                subscription: Some(s),
            })
        })
    }

    #[tracing::instrument(skip_all)]
    async fn sync_to_hubspot(
        &self,
        request: Request<SyncToHubspotRequest>,
    ) -> Result<Response<SyncToHubspotResponse>, Status> {
        let tenant_id = request.tenant()?;

        let req = request.into_inner();

        let subscription_ids = req
            .subscription_ids
            .into_iter()
            .map(SubscriptionId::from_proto)
            .collect::<Result<Vec<_>, _>>()?;

        self.store
            .sync_subscriptions_to_hubspot(tenant_id, subscription_ids)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        Ok(Response::new(SyncToHubspotResponse {}))
    }
}
