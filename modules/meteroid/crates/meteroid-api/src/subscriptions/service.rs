use common_domain::ids::{
    BaseId, BillableMetricId, CustomerId, OrganizationId, PlanId, PlanVersionId, PriceComponentId,
    SubscriptionId,
};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_app::features::{minimum_commitments_open, minimum_commitments_refused};
use meteroid_store::repositories::subscriptions::slots::SubscriptionSlotsInterfaceAuto;
use tonic::{Request, Response, Status};

use meteroid_grpc::meteroid::api::subscriptions::v1::subscriptions_service_server::SubscriptionsService;

use meteroid_grpc::meteroid::api::subscriptions::v1::{
    ActivateSubscriptionRequest, ActivateSubscriptionResponse, ApplyAmendmentRequest,
    ApplyAmendmentResponse, CancelAmendmentRequest, CancelAmendmentResponse,
    CancelPlanChangeRequest, CancelPlanChangeResponse, CancelScheduledEventRequest,
    CancelScheduledEventResponse, CancelSlotTransactionRequest, CancelSlotTransactionResponse,
    CancelSubscriptionRequest, CancelSubscriptionResponse, CreateSubscriptionRequest,
    CreateSubscriptionResponse, CreateSubscriptionsRequest, CreateSubscriptionsResponse,
    GenerateCheckoutTokenRequest, GenerateCheckoutTokenResponse, GetSlotsValueRequest,
    GetSlotsValueResponse, GetSubscriptionComponentUsageRequest,
    GetSubscriptionComponentUsageResponse, GetSubscriptionMinimumProgressRequest,
    GetSubscriptionMinimumProgressResponse, GetSubscriptionReceivablesRequest,
    GetSubscriptionReceivablesResponse, GetSubscriptionUsageRequest, GetSubscriptionUsageResponse,
    GetUpcomingInvoiceRequest, GetUpcomingInvoiceResponse, ListSlotTransactionsRequest,
    ListSlotTransactionsResponse, ListSubscriptionsRequest, ListSubscriptionsResponse,
    ListUnattributedUsageRequest, ListUnattributedUsageResponse, MrrChange,
    PreviewAmendmentRequest, PreviewAmendmentResponse, PreviewCreateSubscriptionRequest,
    PreviewCreateSubscriptionResponse, PreviewPlanChangeRequest, PreviewPlanChangeResponse,
    PreviewSlotUpdateRequest, PreviewSlotUpdateResponse, SchedulePlanChangeRequest,
    SchedulePlanChangeResponse, SubscriptionDetails, SubscriptionMinimumProgress,
    SubscriptionReceivables, SyncToHubspotRequest, SyncToHubspotResponse, UpdateSlotsRequest,
    UpdateSlotsResponse, UpdateSubscriptionRequest, UpdateSubscriptionResponse,
};

use crate::subscriptions::error::SubscriptionApiError;
use crate::subscriptions::{SubscriptionServiceComponents, mapping};
use common_domain::rbac::Permission;
use meteroid_app::conversions::{AsProtoOpt, FromProtoOpt, ProtoConv};
use meteroid_grpc::meteroid::api::subscriptions::v1::{
    OverdueInvoice, PendingInvoiceCredit, UnfilteredOverlap,
};
use meteroid_grpc_mapping::utils::PaginationExt;
use meteroid_grpc_mapping::utils::caller_permissions;
use meteroid_store::domain::credit_outlook::UpcomingChange;
use meteroid_store::repositories::SubscriptionInterface;
use meteroid_store::repositories::plans::PlansInterfaceAuto;
use meteroid_store::repositories::subscriptions::{
    CancellationEffectiveAt, SubscriptionInterfaceAuto,
};

fn added_component_to_grpc(
    a: meteroid_store::domain::subscription_changes::AddedComponent,
) -> meteroid_grpc::meteroid::api::subscriptions::v1::PlanChangeAddedComponent {
    meteroid_grpc::meteroid::api::subscriptions::v1::PlanChangeAddedComponent {
        name: a.name,
        fee: Some(mapping::price_components::subscription_fee_to_grpc(
            &a.fee,
            a.period.as_billing_period_opt().unwrap_or_default(),
        )),
        period: mapping::price_components::subscription_fee_billing_period_to_grpc(a.period).into(),
    }
}

fn removed_component_to_grpc(
    r: meteroid_store::domain::subscription_changes::RemovedComponent,
) -> meteroid_grpc::meteroid::api::subscriptions::v1::PlanChangeRemovedComponent {
    meteroid_grpc::meteroid::api::subscriptions::v1::PlanChangeRemovedComponent {
        name: r.name,
        current_fee: Some(mapping::price_components::subscription_fee_to_grpc(
            &r.current_fee,
            r.current_period.as_billing_period_opt().unwrap_or_default(),
        )),
        current_period: mapping::price_components::subscription_fee_billing_period_to_grpc(
            r.current_period,
        )
        .into(),
    }
}

impl SubscriptionServiceComponents {
    /// Taking the plan's minimum or dropping it stays open; an override introduces one.
    async fn ensure_minimum_overrides_allowed(
        &self,
        organization_id: OrganizationId,
        creates: &[meteroid_store::domain::CreateSubscription],
    ) -> Result<(), Status> {
        if creates.iter().any(|c| c.minimum_override.is_some())
            && !minimum_commitments_open(self.feature_resolver.as_ref(), organization_id).await
        {
            return Err(minimum_commitments_refused());
        }
        Ok(())
    }
}

#[tonic::async_trait]
impl SubscriptionsService for SubscriptionServiceComponents {
    async fn create_subscription(
        &self,
        request: Request<CreateSubscriptionRequest>,
    ) -> Result<Response<CreateSubscriptionResponse>, Status> {
        let tenant_id = request.tenant()?;
        let organization_id = request.organization()?;
        let actor = request.actor_typed()?;

        let inner = request.into_inner();

        let subscription = inner
            .subscription
            .ok_or(SubscriptionApiError::InvalidArgument(
                "No subscription provided".to_string(),
            ))?;

        let subscription = mapping::subscriptions::create_proto_to_domain(subscription)?;
        self.ensure_minimum_overrides_allowed(organization_id, std::slice::from_ref(&subscription))
            .await?;

        let created = self
            .services
            .insert_subscription(actor, subscription, tenant_id)
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
        let organization_id = request.organization()?;
        let actor = request.actor_typed()?;

        let inner = request.into_inner();

        let subscriptions = inner
            .subscriptions
            .into_iter()
            .map(mapping::subscriptions::create_proto_to_domain)
            .collect::<Result<Vec<_>, _>>()?;
        self.ensure_minimum_overrides_allowed(organization_id, &subscriptions)
            .await?;

        let inserted = self
            .services
            .insert_subscription_batch(actor, subscriptions, tenant_id)
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
    async fn preview_create_subscription(
        &self,
        request: Request<PreviewCreateSubscriptionRequest>,
    ) -> Result<Response<PreviewCreateSubscriptionResponse>, Status> {
        let tenant_id = request.tenant()?;
        let organization_id = request.organization()?;
        let inner = request.into_inner();

        let subscription = inner
            .subscription
            .ok_or(SubscriptionApiError::InvalidArgument(
                "No subscription provided".to_string(),
            ))?;

        let create = mapping::subscriptions::create_proto_to_domain(subscription)?;
        // The preview answers what the create would, refusal included.
        self.ensure_minimum_overrides_allowed(organization_id, std::slice::from_ref(&create))
            .await?;

        let (content, details) = self
            .services
            .preview_create_subscription(&create, tenant_id)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        let overlaps = self
            .services
            .unfiltered_overlaps(tenant_id, &details)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;
        let mut unfiltered_overlaps = Vec::with_capacity(overlaps.len());
        for (subscription_id, metric_ids) in overlaps {
            let sibling = self
                .store
                .get_subscription(tenant_id, subscription_id)
                .await
                .map_err(Into::<SubscriptionApiError>::into)?;
            unfiltered_overlaps.push(UnfilteredOverlap {
                subscription_id: subscription_id.as_proto(),
                plan_name: sibling.plan_name,
                metric_ids: metric_ids.iter().map(|id| id.as_proto()).collect(),
            });
        }

        let invoice = mapping::upcoming::create_preview_to_upcoming_proto(content, &details);

        Ok(Response::new(PreviewCreateSubscriptionResponse {
            invoice: Some(invoice),
            unfiltered_overlaps,
        }))
    }

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
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

    #[tracing::instrument(skip_all, fields(customer_id = request.get_ref().customer_id.as_deref()))]
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
                inner.search,
                meteroid_grpc_mapping::filters::map_filters(inner.filters)?,
                inner.order_by,
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

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
    async fn update_slots(
        &self,
        request: Request<UpdateSlotsRequest>,
    ) -> Result<Response<UpdateSlotsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;

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
                actor,
            )
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        Ok(Response::new(UpdateSlotsResponse {
            current_value: result.new_slot_count as u32,
        }))
    }

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
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

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
    async fn cancel_subscription(
        &self,
        request: Request<CancelSubscriptionRequest>,
    ) -> Result<Response<CancelSubscriptionResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor_typed = request.actor_typed()?;
        let inner = request.into_inner();

        use meteroid_grpc::meteroid::api::subscriptions::v1::cancel_subscription_request::EffectiveAt;

        let effective_at = match inner.effective_at {
            Some(EffectiveAt::Immediate(_)) => CancellationEffectiveAt::Immediate,
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
                actor_typed,
                SubscriptionId::from_proto(inner.subscription_id)?,
                tenant_id,
                inner.reason,
                effective_at,
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

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
    async fn generate_checkout_token(
        &self,
        request: Request<GenerateCheckoutTokenRequest>,
    ) -> Result<Response<GenerateCheckoutTokenResponse>, Status> {
        use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;

        let tenant_id = request.tenant()?;
        let delegated = request.delegated();

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
            None,
            None,
            delegated,
        )
        .map_err(Into::<SubscriptionApiError>::into)?;

        Ok(Response::new(GenerateCheckoutTokenResponse { token }))
    }

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
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

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
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

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
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

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
    async fn update_subscription(
        &self,
        request: Request<UpdateSubscriptionRequest>,
    ) -> Result<Response<UpdateSubscriptionResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(&inner.subscription_id)?;

        let patch = mapping::subscriptions::update_request_to_patch(subscription_id, &inner)?;

        let updated = self
            .store
            .patch_subscription(tenant_id, patch, actor)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        let proto_subscription = mapping::subscriptions::domain_to_proto(updated)?;

        Ok(Response::new(UpdateSubscriptionResponse {
            subscription: Some(proto_subscription),
        }))
    }

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
    async fn schedule_plan_change(
        &self,
        request: Request<SchedulePlanChangeRequest>,
    ) -> Result<Response<SchedulePlanChangeResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor_typed = request.actor_typed()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(inner.subscription_id)?;
        let new_plan_version_id = PlanVersionId::from_proto(inner.new_plan_version_id)?;
        let component_params =
            mapping::plan_change::map_component_parameterizations(inner.parameterized_components)?;
        let mode = mapping::plan_change::map_apply_mode(inner.apply_mode);

        match mode {
            meteroid_store::domain::subscription_changes::PlanChangeMode::Immediate => {
                let result = self
                    .services
                    .apply_plan_change_immediate(
                        actor_typed.clone(),
                        subscription_id,
                        tenant_id,
                        new_plan_version_id,
                        component_params,
                    )
                    .await
                    .map_err(Into::<SubscriptionApiError>::into)?;

                Ok(Response::new(SchedulePlanChangeResponse {
                    event_id: None,
                    effective_date: result.effective_date.to_string(),
                    invoice_id: result.adjustment_invoice_id.map(|id| id.to_string()),
                }))
            }
            meteroid_store::domain::subscription_changes::PlanChangeMode::EndOfPeriod => {
                let event = self
                    .services
                    .schedule_plan_change(
                        actor_typed,
                        subscription_id,
                        tenant_id,
                        new_plan_version_id,
                        component_params,
                    )
                    .await
                    .map_err(Into::<SubscriptionApiError>::into)?;

                Ok(Response::new(SchedulePlanChangeResponse {
                    event_id: Some(event.id.to_string()),
                    effective_date: event.scheduled_time.date().to_string(),
                    invoice_id: None,
                }))
            }
        }
    }

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
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
        let mode = mapping::plan_change::map_apply_mode(inner.apply_mode);

        let result = self
            .services
            .preview_plan_change(
                subscription_id,
                tenant_id,
                new_plan_version_id,
                component_params,
                Some(mode),
            )
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        let preview = result.preview;

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
            proration: result
                .proration
                .as_ref()
                .map(mapping::plan_change::proration_summary_to_grpc),
            change_direction: mapping::plan_change::change_direction_to_string(
                result.change_direction,
            ),
            immediate_allowed: result.immediate_allowed,
        }))
    }

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
    async fn cancel_plan_change(
        &self,
        request: Request<CancelPlanChangeRequest>,
    ) -> Result<Response<CancelPlanChangeResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(inner.subscription_id)?;

        self.services
            .cancel_plan_change(actor, subscription_id, tenant_id)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        Ok(Response::new(CancelPlanChangeResponse {}))
    }

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
    async fn cancel_scheduled_event(
        &self,
        request: Request<CancelScheduledEventRequest>,
    ) -> Result<Response<CancelScheduledEventResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let inner = request.into_inner();

        let event_id = common_domain::ids::ScheduledEventId::from_proto(inner.event_id)?;
        let subscription_id =
            common_domain::ids::SubscriptionId::from_proto(inner.subscription_id)?;

        self.services
            .cancel_scheduled_event(actor, event_id, subscription_id, tenant_id)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        Ok(Response::new(CancelScheduledEventResponse {}))
    }

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
    async fn preview_amendment(
        &self,
        request: Request<PreviewAmendmentRequest>,
    ) -> Result<Response<PreviewAmendmentResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(inner.subscription_id)?;
        let amendment = mapping::amendment::map_amendment(
            inner.apply_mode,
            inner.component_changes,
            inner.add_on_changes,
        )?;

        let result = self
            .services
            .preview_amendment(subscription_id, tenant_id, amendment)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        // Needed to render the invoice previews (currency, period, plan name, net terms).
        let details = self
            .store
            .get_subscription_details(tenant_id, subscription_id)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        let preview = result.preview;

        Ok(Response::new(PreviewAmendmentResponse {
            added_components: preview
                .component_added
                .into_iter()
                .map(added_component_to_grpc)
                .collect(),
            removed_components: preview
                .component_removed
                .into_iter()
                .map(removed_component_to_grpc)
                .collect(),
            added_add_ons: preview
                .addon_added
                .into_iter()
                .map(added_component_to_grpc)
                .collect(),
            removed_add_ons: preview
                .addon_removed
                .into_iter()
                .map(removed_component_to_grpc)
                .collect(),
            effective_date: preview.effective_date.to_string(),
            proration: result
                .proration
                .as_ref()
                .map(mapping::plan_change::proration_summary_to_grpc),
            change_direction: mapping::plan_change::change_direction_to_string(
                result.change_direction,
            ),
            adjustment_invoice: result.adjustment_invoice.map(|c| {
                let period =
                    result
                        .adjustment_period
                        .clone()
                        .unwrap_or(meteroid_store::domain::Period {
                            start: preview.effective_date,
                            end: preview.effective_date,
                        });
                mapping::upcoming::computed_content_to_dated_proto(
                    c,
                    &details,
                    period.start,
                    (period.start, period.end),
                )
            }),
            credit_note: result
                .credit_note
                .map(|c| mapping::upcoming::computed_content_to_upcoming_proto(c, &details)),
            next_invoice: result.next_invoice.map(|c| {
                let after = meteroid_store::domain::SubscriptionDetails {
                    subscription: result.subscription_after.clone(),
                    ..details.clone()
                };
                mapping::upcoming::computed_content_to_upcoming_proto(c, &after)
            }),
            mrr: Some(MrrChange {
                before_cents: result.mrr_before_cents,
                after_cents: result.mrr_after_cents,
                delta_cents: result.mrr_after_cents - result.mrr_before_cents,
            }),
        }))
    }

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
    async fn apply_amendment(
        &self,
        request: Request<ApplyAmendmentRequest>,
    ) -> Result<Response<ApplyAmendmentResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(inner.subscription_id)?;
        let amendment = mapping::amendment::map_amendment(
            inner.apply_mode,
            inner.component_changes,
            inner.add_on_changes,
        )?;

        match amendment.apply_mode {
            meteroid_store::domain::subscription_changes::PlanChangeMode::Immediate => {
                let result = self
                    .services
                    .apply_amendment_immediate(actor, subscription_id, tenant_id, amendment)
                    .await
                    .map_err(Into::<SubscriptionApiError>::into)?;

                Ok(Response::new(ApplyAmendmentResponse {
                    event_id: None,
                    effective_date: result.effective_date.to_string(),
                    invoice_id: result.adjustment_invoice_id.map(|id| id.to_string()),
                }))
            }
            meteroid_store::domain::subscription_changes::PlanChangeMode::EndOfPeriod => {
                let event = self
                    .services
                    .schedule_amendment(actor, subscription_id, tenant_id, amendment)
                    .await
                    .map_err(Into::<SubscriptionApiError>::into)?;

                Ok(Response::new(ApplyAmendmentResponse {
                    event_id: Some(event.id.to_string()),
                    effective_date: event.scheduled_time.date().to_string(),
                    invoice_id: None,
                }))
            }
        }
    }

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
    async fn cancel_amendment(
        &self,
        request: Request<CancelAmendmentRequest>,
    ) -> Result<Response<CancelAmendmentResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(inner.subscription_id)?;

        self.services
            .cancel_amendment(actor, subscription_id, tenant_id)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        Ok(Response::new(CancelAmendmentResponse {}))
    }

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
    async fn get_upcoming_invoice(
        &self,
        request: Request<GetUpcomingInvoiceRequest>,
    ) -> Result<Response<GetUpcomingInvoiceResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(inner.subscription_id)?;

        let details = self
            .store
            .get_subscription_details(tenant_id, subscription_id)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        let forecast = self
            .services
            .upcoming_invoice_forecast(&details)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        let new_plan_name = match &forecast.change {
            Some(UpcomingChange::PlanChange { plan_version_id }) => Some(
                self.store
                    .get_plan_by_version_id(*plan_version_id, tenant_id)
                    .await
                    .map_err(Into::<SubscriptionApiError>::into)?
                    .plan
                    .name,
            ),
            _ => None,
        };
        let invoice =
            mapping::upcoming::forecast_to_upcoming_proto(forecast, &details, new_plan_name)?;

        Ok(Response::new(GetUpcomingInvoiceResponse {
            invoice: Some(invoice),
        }))
    }

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
    async fn get_subscription_usage(
        &self,
        request: Request<GetSubscriptionUsageRequest>,
    ) -> Result<Response<GetSubscriptionUsageResponse>, Status> {
        let tenant_id = request.tenant()?;
        let subscription_id = SubscriptionId::from_proto(request.into_inner().subscription_id)?;

        let usage = self
            .services
            .get_subscription_usage(tenant_id, subscription_id, chrono::Utc::now().naive_utc())
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        Ok(Response::new(mapping::usage::usage_to_server(&usage)))
    }

    #[tracing::instrument(skip_all, fields(customer_id = %request.get_ref().customer_id))]
    async fn list_unattributed_usage(
        &self,
        request: Request<ListUnattributedUsageRequest>,
    ) -> Result<Response<ListUnattributedUsageResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();
        let customer_id = CustomerId::from_proto(&inner.customer_id)?;
        let (default_from, default_to) =
            meteroid_store::domain::unattributed_default_range(chrono::Utc::now().date_naive());
        let from = chrono::NaiveDate::from_proto_opt(inner.from)?.unwrap_or(default_from);
        let to = chrono::NaiveDate::from_proto_opt(inner.to)?.unwrap_or(default_to);

        let usage = self
            .services
            .unattributed_usage(
                tenant_id,
                customer_id,
                from.and_time(chrono::NaiveTime::MIN),
                to.and_time(chrono::NaiveTime::MIN),
            )
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        Ok(Response::new(ListUnattributedUsageResponse {
            from: from.as_proto(),
            to: to.as_proto(),
            usage: usage
                .into_iter()
                .map(mapping::subscriptions::unattributed_usage_to_proto)
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
    async fn get_subscription_component_usage(
        &self,
        request: Request<GetSubscriptionComponentUsageRequest>,
    ) -> Result<Response<GetSubscriptionComponentUsageResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(inner.subscription_id)?;
        let metric_id = BillableMetricId::from_proto(inner.metric_id)?;

        let details = self
            .store
            .get_subscription_details(tenant_id, subscription_id)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        let period = meteroid_grpc_mapping::shared::usage::resolve_usage_period(
            inner.start_date.as_ref(),
            inner.end_date.as_ref(),
            &details.subscription,
        )?;

        let usage = self
            .services
            .get_subscription_component_usage(&details, metric_id, period)
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;

        Ok(Response::new(mapping::upcoming::windowed_usage_to_proto(
            usage,
        )))
    }

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
    async fn get_subscription_minimum_progress(
        &self,
        request: Request<GetSubscriptionMinimumProgressRequest>,
    ) -> Result<Response<GetSubscriptionMinimumProgressResponse>, Status> {
        let tenant_id = request.tenant()?;
        let subscription_id = SubscriptionId::from_proto(request.into_inner().subscription_id)?;
        let progress = self
            .services
            .subscription_minimum_progress(
                tenant_id,
                subscription_id,
                chrono::Utc::now().naive_utc(),
            )
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;
        Ok(Response::new(GetSubscriptionMinimumProgressResponse {
            progress: progress.map(|p| SubscriptionMinimumProgress {
                currency: p.currency,
                period_start: p.period_start.format("%Y-%m-%d").to_string(),
                period_end: p.period_end.format("%Y-%m-%d").to_string(),
                minimum: p.minimum_minor,
                counted_to_date: p.counted_to_date_minor,
                projected: p.projected_minor,
                projected_shortfall: p.projected_shortfall_minor,
                elapsed: p.elapsed,
                scope_product_names: p.scope_product_names,
            }),
        }))
    }

    #[tracing::instrument(skip_all, fields(subscription_id = %request.get_ref().subscription_id))]
    async fn get_subscription_receivables(
        &self,
        request: Request<GetSubscriptionReceivablesRequest>,
    ) -> Result<Response<GetSubscriptionReceivablesResponse>, Status> {
        let tenant_id = request.tenant()?;
        // Invoice amounts follow the invoices area, as on the customer summary.
        if !caller_permissions(&request).contains(Permission::InvoicesRead) {
            return Ok(Response::new(GetSubscriptionReceivablesResponse {
                receivables: None,
            }));
        }
        let subscription_id = SubscriptionId::from_proto(request.into_inner().subscription_id)?;
        let r = self
            .services
            .subscription_receivables(tenant_id, subscription_id, chrono::Utc::now().naive_utc())
            .await
            .map_err(Into::<SubscriptionApiError>::into)?;
        Ok(Response::new(GetSubscriptionReceivablesResponse {
            receivables: Some(SubscriptionReceivables {
                currency: r.currency,
                open_amount: r.open_minor,
                open_count: r.open_count,
                overdue_amount: r.overdue_minor,
                overdue_count: r.overdue_count,
                next_due_at: r.next_due_at.as_proto(),
                oldest_overdue: r.oldest_overdue.map(|o| OverdueInvoice {
                    invoice_id: o.invoice_id.as_proto(),
                    invoice_number: o.invoice_number,
                    amount_due: o.amount_due_minor,
                    due_at: o.due_at.as_proto(),
                    last_failure_at: o.last_failure_at.as_proto(),
                    last_failure_reason: o.last_failure_reason,
                    next_retry_at: o.next_retry_at.as_proto(),
                }),
                last_failure_at: r.last_failure_at.as_proto(),
                last_failure_reason: r.last_failure_reason,
                last_failure_invoice_id: r.last_failure_invoice_id.map(|id| id.as_proto()),
                next_retry_at: r.next_retry_at.as_proto(),
                pending_credit: r
                    .pending_credit
                    .into_iter()
                    .map(|c| PendingInvoiceCredit {
                        invoice_id: c.invoice_id.as_proto(),
                        invoice_number: c.invoice_number,
                        currency: c.denomination,
                        credit_unit_id: c.credit_unit_id.map(|id| id.as_proto()),
                        amount: c.amount_minor,
                    })
                    .collect(),
            }),
        }))
    }
}
