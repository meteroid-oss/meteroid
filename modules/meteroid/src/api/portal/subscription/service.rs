use crate::api::portal::subscription::PortalSubscriptionServiceComponents;
use crate::api::portal::subscription::error::PortalSubscriptionApiError;
use crate::api::shared::conversions::ProtoConv;
use common_domain::ids::{BaseId, PlanVersionId, SubscriptionId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::portal::subscription::v1::portal_subscription_service_server::PortalSubscriptionService;
use meteroid_grpc::meteroid::portal::subscription::v1::*;
use meteroid_store::domain::subscription_changes::{
    AddedComponent, MatchedComponent, RemovedComponent,
};
use meteroid_store::domain::subscription_components::SubscriptionFee;
use meteroid_store::repositories::{PlansInterface, SubscriptionInterface};
use tonic::{Request, Response, Status};
use meteroid_store::domain::UsagePricingModel;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;

fn format_fee(fee: &SubscriptionFee) -> String {
    match fee {
        SubscriptionFee::Rate { rate } => format!("{}/period", rate),
        SubscriptionFee::OneTime { rate, quantity } => format!("{} x {}", rate, quantity),
        SubscriptionFee::Slot {
            unit_rate,
            initial_slots,
            ..
        } => format!("{}/slot x {} slots", unit_rate, initial_slots),
        SubscriptionFee::Capacity {
            rate, included, ..
        } => format!("{} ({} included)", rate, included),
        SubscriptionFee::Usage { model, .. } => format!("usage-based ({})", match model {
            UsagePricingModel::PerUnit { .. } => "per unit".to_string(),
            UsagePricingModel::Tiered { .. } => "tiered".to_string(),
            UsagePricingModel::Volume { .. } => "volume".to_string(),
            UsagePricingModel::Package { .. } => "package".to_string(),
            UsagePricingModel::Matrix { .. } => "matrix".to_string(),
        }),
        SubscriptionFee::Recurring { rate, quantity, .. } => format!("{} x {}", rate, quantity),
    }
}

fn map_matched_component(c: &MatchedComponent) -> ComponentChangePreview {
    ComponentChangePreview {
        component_name: c.new_name.clone(),
        action: "matched".to_string(),
        old_value: Some(format_fee(&c.current_fee)),
        new_value: Some(format_fee(&c.new_fee)),
    }
}

fn map_added_component(c: &AddedComponent) -> ComponentChangePreview {
    ComponentChangePreview {
        component_name: c.name.clone(),
        action: "added".to_string(),
        old_value: None,
        new_value: Some(format_fee(&c.fee)),
    }
}

fn map_removed_component(c: &RemovedComponent) -> ComponentChangePreview {
    ComponentChangePreview {
        component_name: c.name.clone(),
        action: "removed".to_string(),
        old_value: Some(format_fee(&c.current_fee)),
        new_value: None,
    }
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

        let subscription = SubscriptionDetails {
            id: details.subscription.id.as_proto(),
            plan_name: details.subscription.plan_name.clone(),
            plan_id: details.subscription.plan_id.as_proto(),
            plan_version_id: details.subscription.plan_version_id.as_proto(),
            status: format!("{:?}", details.subscription.status),
            current_period_end: details.subscription.current_period_end.map(|d| d.as_proto()),
            mrr_cents: details.subscription.mrr_cents,
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

        // Look up the plan to get product_family_id
        let plan = self
            .store
            .get_plan(
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

        // Build available plans list — include the current plan too if it has a rank
        let mut plans: Vec<AvailablePlan> = Vec::new();

        if let Some(rank) = plan.plan.self_service_rank {
            plans.push(AvailablePlan {
                plan_id: plan.plan.id.as_proto(),
                plan_version_id: details.subscription.plan_version_id.as_proto(),
                plan_name: plan.plan.name.clone(),
                description: plan.plan.description.clone(),
                self_service_rank: rank,
                is_current: true,
            });
        }

        for sp in self_service_plans {
            plans.push(AvailablePlan {
                plan_id: sp.plan_id.as_proto(),
                plan_version_id: sp.plan_version_id.as_proto(),
                plan_name: sp.plan_name,
                description: sp.description,
                self_service_rank: sp.self_service_rank,
                is_current: false,
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
            component_changes.push(map_matched_component(c));
        }
        for c in &preview.added {
            component_changes.push(map_added_component(c));
        }
        for c in &preview.removed {
            component_changes.push(map_removed_component(c));
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
