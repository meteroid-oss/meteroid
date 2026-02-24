use crate::api::portal::subscription::PortalSubscriptionServiceComponents;
use crate::api::portal::subscription::error::PortalSubscriptionApiError;
use crate::api::shared::conversions::ProtoConv;
use common_domain::ids::{PlanVersionId, SubscriptionId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::prices::v1 as prices_proto;
use meteroid_grpc::meteroid::api::subscriptions::v1 as sub_proto;
use meteroid_grpc::meteroid::portal::subscription::v1::portal_subscription_service_server::PortalSubscriptionService;
use meteroid_grpc::meteroid::portal::subscription::v1::*;
use meteroid_store::domain::enums::{BillingPeriodEnum, SubscriptionFeeBillingPeriod};
use meteroid_store::domain::plans::FullPlan;
use meteroid_store::domain::prices::Pricing;
use meteroid_store::domain::subscription_components::{SubscriptionComponent, SubscriptionFee};
use meteroid_store::repositories::PlansInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use tonic::{Request, Response, Status};

// ---------------------------------------------------------------------------
// Fee mapping helpers
// ---------------------------------------------------------------------------

fn map_billing_period(period: &BillingPeriodEnum) -> i32 {
    match period {
        BillingPeriodEnum::Monthly => sub_proto::SubscriptionFeeBillingPeriod::Monthly.into(),
        BillingPeriodEnum::Quarterly => sub_proto::SubscriptionFeeBillingPeriod::Quarterly.into(),
        BillingPeriodEnum::Semiannual => sub_proto::SubscriptionFeeBillingPeriod::Semiannual.into(),
        BillingPeriodEnum::Annual => sub_proto::SubscriptionFeeBillingPeriod::Yearly.into(),
    }
}

fn map_sub_billing_period(period: &SubscriptionFeeBillingPeriod) -> i32 {
    match period {
        SubscriptionFeeBillingPeriod::OneTime => {
            sub_proto::SubscriptionFeeBillingPeriod::OneTime.into()
        }
        SubscriptionFeeBillingPeriod::Monthly => {
            sub_proto::SubscriptionFeeBillingPeriod::Monthly.into()
        }
        SubscriptionFeeBillingPeriod::Quarterly => {
            sub_proto::SubscriptionFeeBillingPeriod::Quarterly.into()
        }
        SubscriptionFeeBillingPeriod::Semiannual => {
            sub_proto::SubscriptionFeeBillingPeriod::Semiannual.into()
        }
        SubscriptionFeeBillingPeriod::Annual => {
            sub_proto::SubscriptionFeeBillingPeriod::Yearly.into()
        }
    }
}

fn map_fee(fee: &SubscriptionFee, period: &SubscriptionFeeBillingPeriod) -> ComponentFee {
    let (fee_type, amount, unit) = match fee {
        SubscriptionFee::Rate { rate } => (prices_proto::FeeType::Rate, rate.to_string(), None),
        SubscriptionFee::OneTime { rate, .. } => {
            (prices_proto::FeeType::OneTime, rate.to_string(), None)
        }
        SubscriptionFee::Recurring { rate, .. } => (
            prices_proto::FeeType::ExtraRecurring,
            rate.to_string(),
            None,
        ),
        SubscriptionFee::Capacity { rate, .. } => {
            (prices_proto::FeeType::Capacity, rate.to_string(), None)
        }
        SubscriptionFee::Slot {
            unit_rate, unit, ..
        } => (
            prices_proto::FeeType::Slot,
            unit_rate.to_string(),
            Some(unit.clone()),
        ),
        SubscriptionFee::Usage { .. } => (prices_proto::FeeType::Usage, String::new(), None),
    };

    ComponentFee {
        fee_type: fee_type.into(),
        amount,
        cadence: map_sub_billing_period(period),
        unit,
    }
}

// ---------------------------------------------------------------------------
// Headline fee extraction
// ---------------------------------------------------------------------------

fn fee_type_priority(fee: &SubscriptionFee) -> Option<u8> {
    match fee {
        SubscriptionFee::Rate { .. } => Some(0),
        SubscriptionFee::Slot { .. } => Some(1),
        SubscriptionFee::Capacity { .. } => Some(2),
        _ => None,
    }
}

fn pick_headline_fee(components: &[SubscriptionComponent]) -> Option<ComponentFee> {
    components
        .iter()
        .filter_map(|c| fee_type_priority(&c.fee).map(|p| (p, c)))
        .min_by_key(|(p, _)| *p)
        .map(|(_, c)| map_fee(&c.fee, &c.period))
}

fn pricing_priority(pricing: &Pricing) -> Option<u8> {
    match pricing {
        Pricing::Rate { .. } => Some(0),
        Pricing::Slot { .. } => Some(1),
        Pricing::Capacity { .. } => Some(2),
        _ => None,
    }
}

/// Extract headline fee from a plan's price components for a given currency.
fn pick_plan_headline_fee(plan: &FullPlan, currency: &str) -> Option<ComponentFee> {
    // Collect (priority, fee_type, amount, cadence, unit) from v2 prices
    let mut candidates: Vec<(u8, i32, String, i32, Option<String>)> = Vec::new();

    for component in &plan.price_components {
        // v2 prices
        for price in &component.prices {
            if price.currency.eq_ignore_ascii_case(currency)
                && let Some(priority) = pricing_priority(&price.pricing)
            {
                let (fee_type, amount, unit) = match &price.pricing {
                    Pricing::Rate { rate } => (prices_proto::FeeType::Rate, rate.to_string(), None),
                    Pricing::Slot { unit_rate, .. } => {
                        // Try to get unit name from product
                        let unit_name = component
                            .product_id
                            .and_then(|pid| plan.products.get(&pid))
                            .map(|p| p.name.clone());
                        (
                            prices_proto::FeeType::Slot,
                            unit_rate.to_string(),
                            unit_name,
                        )
                    }
                    Pricing::Capacity { rate, .. } => {
                        (prices_proto::FeeType::Capacity, rate.to_string(), None)
                    }
                    _ => continue,
                };
                candidates.push((
                    priority,
                    fee_type.into(),
                    amount,
                    map_billing_period(&price.cadence),
                    unit,
                ));
            }
        }

        // v1 legacy pricing
        if let Some(legacy) = &component.legacy_pricing
            && legacy.currency.eq_ignore_ascii_case(currency)
        {
            for (cadence, pricing) in &legacy.pricing_entries {
                if let Some(priority) = pricing_priority(pricing) {
                    let (fee_type, amount, unit) = match pricing {
                        Pricing::Rate { rate } => {
                            (prices_proto::FeeType::Rate, rate.to_string(), None)
                        }
                        Pricing::Slot { unit_rate, .. } => {
                            let unit_name = match &legacy.fee_structure {
                                meteroid_store::domain::prices::FeeStructure::Slot {
                                    unit_name,
                                    ..
                                } => Some(unit_name.clone()),
                                _ => None,
                            };
                            (
                                prices_proto::FeeType::Slot,
                                unit_rate.to_string(),
                                unit_name,
                            )
                        }
                        Pricing::Capacity { rate, .. } => {
                            (prices_proto::FeeType::Capacity, rate.to_string(), None)
                        }
                        _ => continue,
                    };
                    candidates.push((
                        priority,
                        fee_type.into(),
                        amount,
                        map_billing_period(cadence),
                        unit,
                    ));
                }
            }
        }
    }

    candidates.into_iter().min_by_key(|(p, _, _, _, _)| *p).map(
        |(_, fee_type, amount, cadence, unit)| ComponentFee {
            fee_type,
            amount,
            cadence,
            unit,
        },
    )
}

#[tonic::async_trait]
impl PortalSubscriptionService for PortalSubscriptionServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn get_subscription_details(
        &self,
        request: Request<GetSubscriptionDetailsRequest>,
    ) -> Result<Response<GetSubscriptionDetailsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let customer_id = request.portal_resource()?.customer()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(&inner.subscription_id)?;

        let details = self
            .store
            .get_subscription_details(tenant_id, subscription_id)
            .await
            .map_err(Into::<PortalSubscriptionApiError>::into)?;

        // Verify customer owns this subscription
        if details.subscription.customer_id != customer_id {
            return Err(PortalSubscriptionApiError::Unauthorized.into());
        }

        // Check if the plan has self_service_rank set
        let plan = self
            .store
            .get_plan(
                details.subscription.plan_id,
                tenant_id,
                meteroid_store::domain::PlanVersionFilter::Active,
            )
            .await
            .map_err(Into::<PortalSubscriptionApiError>::into)?;

        let can_change_plan = plan.plan.self_service_rank.is_some();

        let headline_fee = pick_headline_fee(&details.price_components);

        let subscription = SubscriptionDetails {
            id: details.subscription.id.as_proto(),
            plan_name: details.subscription.plan_name.clone(),
            plan_id: details.subscription.plan_id.as_proto(),
            plan_version_id: details.subscription.plan_version_id.as_proto(),
            status: format!("{:?}", details.subscription.status),
            current_period_end: details
                .subscription
                .current_period_end
                .map(|d| d.as_proto()),
            headline_fee,
            currency: details.subscription.currency.clone(),
            can_change_plan,
            scheduled_plan_change: details
                .pending_plan_change
                .as_ref()
                .map(|pc| pc.new_plan_name.clone()),
            scheduled_plan_change_date: details
                .pending_plan_change
                .as_ref()
                .map(|pc| pc.effective_date.as_proto()),
        };

        Ok(Response::new(GetSubscriptionDetailsResponse {
            subscription: Some(subscription),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_available_plans(
        &self,
        request: Request<ListAvailablePlansRequest>,
    ) -> Result<Response<ListAvailablePlansResponse>, Status> {
        let tenant_id = request.tenant()?;
        let customer_id = request.portal_resource()?.customer()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(&inner.subscription_id)?;

        let details = self
            .store
            .get_subscription_details(tenant_id, subscription_id)
            .await
            .map_err(Into::<PortalSubscriptionApiError>::into)?;

        if details.subscription.customer_id != customer_id {
            return Err(PortalSubscriptionApiError::Unauthorized.into());
        }

        // Look up the full plan to get product_family_id and price_components for headline fee
        let plan = self
            .store
            .get_full_plan(
                details.subscription.plan_id,
                tenant_id,
                meteroid_store::domain::PlanVersionFilter::Active,
            )
            .await
            .map_err(Into::<PortalSubscriptionApiError>::into)?;

        let self_service_plans = self
            .store
            .list_self_service_plans(
                tenant_id,
                plan.plan.product_family_id,
                &details.subscription.currency,
                details.subscription.plan_id,
            )
            .await
            .map_err(Into::<PortalSubscriptionApiError>::into)?;

        let currency = &details.subscription.currency;

        // Build available plans list — include the current plan too if it has a rank
        let mut plans: Vec<AvailablePlan> = Vec::new();

        if let Some(rank) = plan.plan.self_service_rank {
            let headline_fee = pick_plan_headline_fee(&plan, currency);
            plans.push(AvailablePlan {
                plan_id: plan.plan.id.as_proto(),
                plan_version_id: details.subscription.plan_version_id.as_proto(),
                plan_name: plan.plan.name.clone(),
                description: plan.plan.description.clone(),
                self_service_rank: rank,
                is_current: true,
                headline_fee,
            });
        }

        for sp in self_service_plans {
            let full_plan = self
                .store
                .get_full_plan(
                    sp.plan_id,
                    tenant_id,
                    meteroid_store::domain::PlanVersionFilter::Active,
                )
                .await
                .map_err(Into::<PortalSubscriptionApiError>::into)?;

            let headline_fee = pick_plan_headline_fee(&full_plan, currency);

            plans.push(AvailablePlan {
                plan_id: sp.plan_id.as_proto(),
                plan_version_id: sp.plan_version_id.as_proto(),
                plan_name: sp.plan_name,
                description: sp.description,
                self_service_rank: sp.self_service_rank,
                is_current: false,
                headline_fee,
            });
        }

        plans.sort_by_key(|p| p.self_service_rank);

        Ok(Response::new(ListAvailablePlansResponse { plans }))
    }

    #[tracing::instrument(skip_all)]
    async fn preview_plan_change(
        &self,
        request: Request<PreviewPlanChangeRequest>,
    ) -> Result<Response<PreviewPlanChangeResponse>, Status> {
        let tenant_id = request.tenant()?;
        let customer_id = request.portal_resource()?.customer()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(&inner.subscription_id)?;
        let new_plan_version_id = PlanVersionId::from_proto(&inner.new_plan_version_id)?;

        // Verify ownership
        let details = self
            .store
            .get_subscription_details(tenant_id, subscription_id)
            .await
            .map_err(Into::<PortalSubscriptionApiError>::into)?;

        if details.subscription.customer_id != customer_id {
            return Err(PortalSubscriptionApiError::Unauthorized.into());
        }

        // Get target plan name
        let target_plan = self
            .store
            .get_plan_by_version_id(new_plan_version_id, tenant_id)
            .await
            .map_err(Into::<PortalSubscriptionApiError>::into)?;

        let preview = self
            .services
            .preview_plan_change(subscription_id, tenant_id, new_plan_version_id, vec![])
            .await
            .map_err(Into::<PortalSubscriptionApiError>::into)?;

        let mut component_changes: Vec<ComponentChangePreview> = Vec::new();
        for c in &preview.matched {
            component_changes.push(ComponentChangePreview {
                component_name: c.new_name.clone(),
                is_new: false,
                current_fee: Some(map_fee(&c.current_fee, &c.current_period)),
                new_fee: Some(map_fee(&c.new_fee, &c.new_period)),
            });
        }
        for c in &preview.added {
            component_changes.push(ComponentChangePreview {
                component_name: c.name.clone(),
                is_new: true,
                current_fee: None,
                new_fee: Some(map_fee(&c.fee, &c.period)),
            });
        }

        Ok(Response::new(PreviewPlanChangeResponse {
            preview: Some(PlanChangePreview {
                effective_date: preview.effective_date.as_proto(),
                component_changes,
            }),
            new_plan_name: target_plan.plan.name,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn confirm_plan_change(
        &self,
        request: Request<ConfirmPlanChangeRequest>,
    ) -> Result<Response<ConfirmPlanChangeResponse>, Status> {
        let tenant_id = request.tenant()?;
        let customer_id = request.portal_resource()?.customer()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(&inner.subscription_id)?;
        let new_plan_version_id = PlanVersionId::from_proto(&inner.new_plan_version_id)?;

        // Verify ownership
        let details = self
            .store
            .get_subscription_details(tenant_id, subscription_id)
            .await
            .map_err(Into::<PortalSubscriptionApiError>::into)?;

        if details.subscription.customer_id != customer_id {
            return Err(PortalSubscriptionApiError::Unauthorized.into());
        }

        let event = self
            .services
            .schedule_plan_change(subscription_id, tenant_id, new_plan_version_id, vec![])
            .await
            .map_err(Into::<PortalSubscriptionApiError>::into)?;

        Ok(Response::new(ConfirmPlanChangeResponse {
            scheduled_for: event.scheduled_time.as_proto(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn cancel_scheduled_plan_change(
        &self,
        request: Request<CancelScheduledPlanChangeRequest>,
    ) -> Result<Response<CancelScheduledPlanChangeResponse>, Status> {
        let tenant_id = request.tenant()?;
        let customer_id = request.portal_resource()?.customer()?;
        let inner = request.into_inner();

        let subscription_id = SubscriptionId::from_proto(&inner.subscription_id)?;

        // Verify ownership
        let details = self
            .store
            .get_subscription_details(tenant_id, subscription_id)
            .await
            .map_err(Into::<PortalSubscriptionApiError>::into)?;

        if details.subscription.customer_id != customer_id {
            return Err(PortalSubscriptionApiError::Unauthorized.into());
        }

        self.services
            .cancel_plan_change(subscription_id, tenant_id)
            .await
            .map_err(Into::<PortalSubscriptionApiError>::into)?;

        Ok(Response::new(CancelScheduledPlanChangeResponse {}))
    }
}
