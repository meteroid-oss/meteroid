use common_domain::ids::{
    BaseId, CustomerId, PlanId, PlanVersionId, PriceComponentId, SubscriptionId,
};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_store::repositories::subscriptions::slots::SubscriptionSlotsInterfaceAuto;
use tonic::{Request, Response, Status};

use meteroid_grpc::meteroid::api::subscriptions::v1::subscriptions_service_server::SubscriptionsService;

use meteroid_grpc::meteroid::api::subscriptions::v1::{
    ActivateSubscriptionRequest, ActivateSubscriptionResponse, CancelPlanChangeRequest,
    CancelPlanChangeResponse, CancelScheduledEventRequest, CancelScheduledEventResponse,
    CancelSlotTransactionRequest, CancelSlotTransactionResponse, CancelSubscriptionRequest,
    CancelSubscriptionResponse, CreateSubscriptionRequest, CreateSubscriptionResponse,
    CreateSubscriptionsRequest, CreateSubscriptionsResponse, GenerateCheckoutTokenRequest,
    GenerateCheckoutTokenResponse, GetSlotsValueRequest, GetSlotsValueResponse,
    ListSlotTransactionsRequest, ListSlotTransactionsResponse, ListSubscriptionsRequest,
    ListSubscriptionsResponse, PreviewPlanChangeRequest, PreviewPlanChangeResponse,
    PreviewSlotUpdateRequest, PreviewSlotUpdateResponse, SchedulePlanChangeRequest,
    SchedulePlanChangeResponse, SubscriptionDetails, SyncToHubspotRequest, SyncToHubspotResponse,
    UpdateSlotsRequest, UpdateSlotsResponse, UpdateSubscriptionRequest, UpdateSubscriptionResponse,
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

    #[tracing::instrument(skip_all)]
    async fn generate_checkout_token(
        &self,
        request: Request<GenerateCheckoutTokenRequest>,
    ) -> Result<Response<GenerateCheckoutTokenResponse>, Status> {
        use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;

        let tenant_id = request.tenant()?;

        let req = request.into_inner();
        let subscription_id = SubscriptionId::from_proto(req.subscription_id)?;

        // Find the CheckoutSession linked to this subscription
        let session = self
            .store
            .get_checkout_session_by_subscription(tenant_id, subscription_id)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        // Generate the JWT token for the CheckoutSession
        let token = meteroid_store::jwt_claims::generate_portal_token(
            &self.jwt_secret,
            tenant_id,
            meteroid_store::jwt_claims::ResourceAccess::CheckoutSession(session.id),
        )
        .map_err(Into::<SubscriptionApiError>::into)?;

        Ok(Response::new(GenerateCheckoutTokenResponse { token }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_slot_transactions(
        &self,
        request: Request<ListSlotTransactionsRequest>,
    ) -> Result<Response<ListSlotTransactionsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(inner.subscription_id)?;

        let status = if let Some(status_int) = inner.status {
            use meteroid_grpc::meteroid::api::subscriptions::v1::SlotTransactionStatus;
            use meteroid_store::domain::SlotTransactionStatusEnum;

            let proto_status = SlotTransactionStatus::try_from(status_int)
                .map_err(|_| SubscriptionApiError::InvalidArgument("Invalid status".to_string()))?;

            Some(match proto_status {
                SlotTransactionStatus::SlotPending => SlotTransactionStatusEnum::Pending,
                SlotTransactionStatus::SlotActive => SlotTransactionStatusEnum::Active,
            })
        } else {
            None
        };

        let transactions = self
            .store
            .list_slot_transactions(tenant_id, subscription_id, inner.unit, status)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        let proto_transactions = transactions
            .into_iter()
            .map(|row| {
                let domain: meteroid_store::domain::slot_transactions::SlotTransaction = row.into();
                mapping::slot_transactions::domain_to_proto(domain)
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Response::new(ListSlotTransactionsResponse {
            transactions: proto_transactions,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn cancel_slot_transaction(
        &self,
        request: Request<CancelSlotTransactionRequest>,
    ) -> Result<Response<CancelSlotTransactionResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let transaction_id =
            common_domain::ids::SlotTransactionId::from_proto(inner.transaction_id)?;

        self.store
            .cancel_slot_transaction(tenant_id, transaction_id)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        Ok(Response::new(CancelSlotTransactionResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn preview_slot_update(
        &self,
        request: Request<PreviewSlotUpdateRequest>,
    ) -> Result<Response<PreviewSlotUpdateResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(inner.subscription_id)?;
        let price_component_id = PriceComponentId::from_proto(inner.price_component_id)?;

        let preview = self
            .services
            .preview_slot_update(tenant_id, subscription_id, price_component_id, inner.delta)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        Ok(Response::new(
            mapping::slot_transactions::preview_domain_to_proto(preview)?,
        ))
    }

    #[tracing::instrument(skip_all)]
    async fn activate_subscription(
        &self,
        request: Request<ActivateSubscriptionRequest>,
    ) -> Result<Response<ActivateSubscriptionResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(inner.subscription_id)?;

        // Activate the subscription
        let subscription = self
            .services
            .activate_subscription_manual(tenant_id, subscription_id)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        let proto_subscription = mapping::subscriptions::domain_to_proto(subscription)?;

        Ok(Response::new(ActivateSubscriptionResponse {
            subscription: Some(proto_subscription),
            invoice_id: None, // Invoice creation happens asynchronously via worker
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_subscription(
        &self,
        request: Request<UpdateSubscriptionRequest>,
    ) -> Result<Response<UpdateSubscriptionResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(&inner.subscription_id)?;

        let patch = mapping::subscriptions::update_request_to_patch(subscription_id, &inner)?;

        let updated = self
            .store
            .patch_subscription(tenant_id, patch)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        let proto_subscription = mapping::subscriptions::domain_to_proto(updated)?;

        Ok(Response::new(UpdateSubscriptionResponse {
            subscription: Some(proto_subscription),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn schedule_plan_change(
        &self,
        request: Request<SchedulePlanChangeRequest>,
    ) -> Result<Response<SchedulePlanChangeResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(inner.subscription_id)?;
        let new_plan_version_id = PlanVersionId::from_proto(inner.new_plan_version_id)?;
        let component_params =
            mapping::plan_change::map_component_parameterizations(inner.parameterized_components)?;

        let event = self
            .services
            .schedule_plan_change(
                subscription_id,
                tenant_id,
                new_plan_version_id,
                component_params,
            )
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        Ok(Response::new(SchedulePlanChangeResponse {
            event_id: event.id.to_string(),
            effective_date: event.scheduled_time.date().to_string(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn preview_plan_change(
        &self,
        request: Request<PreviewPlanChangeRequest>,
    ) -> Result<Response<PreviewPlanChangeResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(inner.subscription_id)?;
        let new_plan_version_id = PlanVersionId::from_proto(inner.new_plan_version_id)?;
        let component_params =
            mapping::plan_change::map_component_parameterizations(inner.parameterized_components)?;

        let preview = self
            .services
            .preview_plan_change(
                subscription_id,
                tenant_id,
                new_plan_version_id,
                component_params,
            )
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        Ok(Response::new(PreviewPlanChangeResponse {
            matched: preview
                .matched
                .into_iter()
                .map(|m| {
                    meteroid_grpc::meteroid::api::subscriptions::v1::PlanChangeMatchedComponent {
                        product_id: m.product_id.as_base62(),
                        current_name: m.current_name,
                        new_name: m.new_name,
                        current_fee: Some(mapping::price_components::subscription_fee_to_grpc(
                            &m.current_fee,
                            m.current_period.as_billing_period_opt().unwrap_or_default(),
                        )),
                        current_period:
                            mapping::price_components::subscription_fee_billing_period_to_grpc(
                                m.current_period,
                            )
                            .into(),
                        new_fee: Some(mapping::price_components::subscription_fee_to_grpc(
                            &m.new_fee,
                            m.new_period.as_billing_period_opt().unwrap_or_default(),
                        )),
                        new_period:
                            mapping::price_components::subscription_fee_billing_period_to_grpc(
                                m.new_period,
                            )
                            .into(),
                    }
                })
                .collect(),
            added: preview
                .added
                .into_iter()
                .map(|a| {
                    meteroid_grpc::meteroid::api::subscriptions::v1::PlanChangeAddedComponent {
                        name: a.name,
                        fee: Some(mapping::price_components::subscription_fee_to_grpc(
                            &a.fee,
                            a.period.as_billing_period_opt().unwrap_or_default(),
                        )),
                        period: mapping::price_components::subscription_fee_billing_period_to_grpc(
                            a.period,
                        )
                        .into(),
                    }
                })
                .collect(),
            removed: preview
                .removed
                .into_iter()
                .map(|r| {
                    meteroid_grpc::meteroid::api::subscriptions::v1::PlanChangeRemovedComponent {
                        name: r.name,
                        current_fee: Some(mapping::price_components::subscription_fee_to_grpc(
                            &r.current_fee,
                            r.current_period.as_billing_period_opt().unwrap_or_default(),
                        )),
                        current_period:
                            mapping::price_components::subscription_fee_billing_period_to_grpc(
                                r.current_period,
                            )
                            .into(),
                    }
                })
                .collect(),
            effective_date: preview.effective_date.to_string(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn cancel_plan_change(
        &self,
        request: Request<CancelPlanChangeRequest>,
    ) -> Result<Response<CancelPlanChangeResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(inner.subscription_id)?;

        self.services
            .cancel_plan_change(subscription_id, tenant_id)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        Ok(Response::new(CancelPlanChangeResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn cancel_scheduled_event(
        &self,
        request: Request<CancelScheduledEventRequest>,
    ) -> Result<Response<CancelScheduledEventResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let event_id = common_domain::ids::ScheduledEventId::from_proto(inner.event_id)?;

        self.services
            .cancel_scheduled_event(event_id, tenant_id)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        Ok(Response::new(CancelScheduledEventResponse {}))
    }
}
