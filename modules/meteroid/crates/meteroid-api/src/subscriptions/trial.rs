//! A subscription's trial for the web (R-153): read whole, extended, ended today and previewed.
use chrono::NaiveDate;
use common_domain::ids::{SubscriptionId, TenantId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_app::conversions::ProtoConv;
use meteroid_grpc::meteroid::api::subscriptions::v1 as proto2;
use meteroid_grpc::meteroid::api::trials::v1 as proto;
use meteroid_grpc::meteroid::api::trials::v1::subscription_trials_service_server::{
    SubscriptionTrialsService, SubscriptionTrialsServiceServer,
};
use meteroid_store::domain::SubscriptionDetails;
use meteroid_store::domain::customer_payment_methods::ResolvedPaymentMethod;
use meteroid_store::domain::subscription_changes::PlanChangeMode;
use meteroid_store::domain::trials::{
    SubscriptionTrial, TrialEnd, TrialEndAction, TrialMove, TrialOffer, TrialOutcome, TrialStatus,
    TrialWithoutCard,
};
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use meteroid_store::{Services, Store};
use tonic::{Request, Response, Status};

use crate::plans::mapping::plans::{at_end_to_proto, without_card_to_proto};
use crate::subscriptions::error::SubscriptionApiError;

fn store_status(report: error_stack::Report<meteroid_store::errors::StoreError>) -> Status {
    SubscriptionApiError::from(report).into()
}

pub(crate) fn status_to_proto(status: TrialStatus) -> proto2::trial_config::Status {
    use proto2::trial_config::Status as Proto;
    match status {
        TrialStatus::Active => Proto::Active,
        TrialStatus::Converted => Proto::Converted,
        TrialStatus::Cancelled => Proto::Cancelled,
        TrialStatus::Switched => Proto::Switched,
        TrialStatus::AwaitingPayment => Proto::AwaitingPayment,
        TrialStatus::AwaitingDecision => Proto::AwaitingDecision,
    }
}

/// The trial's end action as recorded, the plan of a switch read from its scheduled switch, else
/// its offer's.
fn end_action_of(
    trial: &SubscriptionTrial,
    offer: Option<&TrialOffer>,
    details: &SubscriptionDetails,
) -> TrialEndAction {
    match trial.end_action {
        TrialEnd::Convert => TrialEndAction::Convert,
        TrialEnd::Manual => TrialEndAction::Manual,
        TrialEnd::Cancel => TrialEndAction::Cancel,
        TrialEnd::Fallback => {
            let scheduled = details
                .pending_events
                .iter()
                .filter(|event| event.scheduled_date == trial.until)
                .find_map(|event| event.new_plan_version_id);
            let offered = offer.and_then(|offer| match offer.end_action {
                TrialEndAction::Fallback { plan_version_id } => Some(plan_version_id),
                _ => None,
            });
            match scheduled.or(offered) {
                Some(plan_version_id) => TrialEndAction::Fallback { plan_version_id },
                None => TrialEndAction::Convert,
            }
        }
    }
}

async fn has_payment_method(
    store: &Store,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> Result<bool, Status> {
    let resolved = store
        .resolve_payment_method_for_subscription(tenant_id, subscription_id)
        .await
        .map_err(store_status)?;
    Ok(!matches!(resolved, ResolvedPaymentMethod::NotConfigured))
}

/// The opening invoice billed from `on`, as the checkout prices a new subscription.
async fn opening_total(
    services: &Services,
    details: &SubscriptionDetails,
    on: NaiveDate,
) -> Result<i64, Status> {
    let mut from = details.clone();
    from.subscription.billing_start_date = Some(on);
    Ok(services
        .compute_new_subscription_invoice(&from)
        .await
        .map_err(store_status)?
        .total)
}

fn estimate(amount: i64, currency: &str, on: NaiveDate) -> proto2::FirstChargeEstimate {
    proto2::FirstChargeEstimate {
        amount,
        currency: currency.to_owned(),
        on: on.as_proto(),
    }
}

/// The subscription's trial read whole; none without one.
pub(crate) async fn trial_config(
    store: &Store,
    services: &Services,
    tenant_id: TenantId,
    details: &SubscriptionDetails,
) -> Result<Option<proto2::TrialConfig>, Status> {
    let Some(mut config) = details
        .trial_config
        .clone()
        .map(super::mapping::subscriptions::trial_config_to_proto)
    else {
        return Ok(None);
    };
    let Some((trial, offer)) = services
        .subscription_trial(tenant_id, details.subscription.id)
        .await
        .map_err(store_status)?
    else {
        return Ok(Some(config));
    };
    let status = trial.status();
    let end_action = end_action_of(&trial, offer.as_ref(), details);
    let billing_start = if trial.free { trial.until } else { trial.from };
    config.status = status_to_proto(status) as i32;
    config.ended_on = trial.ended_at.map(|at| at.date().as_proto());
    config.at_end = Some(at_end_to_proto(end_action));
    config.without_card = Some(without_card_to_proto(trial.without_card));
    config.has_payment_method =
        has_payment_method(store, tenant_id, details.subscription.id).await?;
    config.first_charge_estimate =
        if status == TrialStatus::Active && end_action == TrialEndAction::Convert {
            match opening_total(services, details, billing_start).await {
                Ok(total) => Some(estimate(
                    total,
                    &details.subscription.currency,
                    billing_start,
                )),
                Err(status) => {
                    log::warn!(
                        "The first charge of subscription {}'s trial is not estimated: {status:?}",
                        details.subscription.id
                    );
                    None
                }
            }
        } else {
            None
        };
    Ok(Some(config))
}

pub fn service(
    store: Store,
    services: Services,
) -> SubscriptionTrialsServiceServer<TrialsComponents> {
    SubscriptionTrialsServiceServer::new(TrialsComponents { store, services })
}

pub struct TrialsComponents {
    store: Store,
    services: Services,
}

impl TrialsComponents {
    async fn moved(
        &self,
        actor: common_domain::actor::Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        to: TrialMove,
    ) -> Result<Option<proto2::TrialConfig>, Status> {
        self.services
            .move_trial(actor, tenant_id, subscription_id, to)
            .await
            .map_err(store_status)?;
        let details = self
            .store
            .get_subscription_details(tenant_id, subscription_id)
            .await
            .map_err(store_status)?;
        trial_config(&self.store, &self.services, tenant_id, &details).await
    }
}

#[tonic::async_trait]
impl SubscriptionTrialsService for TrialsComponents {
    #[tracing::instrument(skip_all)]
    async fn extend_trial(
        &self,
        request: Request<proto::ExtendTrialRequest>,
    ) -> Result<Response<proto::ExtendTrialResponse>, Status> {
        use proto::extend_trial_request::To;
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let inner = request.into_inner();
        let to = match inner.to {
            Some(To::UntilDate(day)) => TrialMove::Until(NaiveDate::from_proto(day)?),
            Some(To::Days(days)) if days > 0 => TrialMove::Days(days),
            _ => return Err(Status::invalid_argument("the trial's new end")),
        };
        let trial = self
            .moved(
                actor,
                tenant_id,
                SubscriptionId::from_proto(inner.subscription_id)?,
                to,
            )
            .await?;
        Ok(Response::new(proto::ExtendTrialResponse { trial }))
    }

    #[tracing::instrument(skip_all)]
    async fn end_trial(
        &self,
        request: Request<proto::EndTrialRequest>,
    ) -> Result<Response<proto::EndTrialResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let subscription_id = SubscriptionId::from_proto(request.into_inner().subscription_id)?;
        let trial = self
            .moved(actor, tenant_id, subscription_id, TrialMove::Now)
            .await?;
        Ok(Response::new(proto::EndTrialResponse { trial }))
    }

    #[tracing::instrument(skip_all)]
    async fn preview_end_trial(
        &self,
        request: Request<proto::PreviewEndTrialRequest>,
    ) -> Result<Response<proto::PreviewEndTrialResponse>, Status> {
        let tenant_id = request.tenant()?;
        let subscription_id = SubscriptionId::from_proto(request.into_inner().subscription_id)?;
        let details = self
            .store
            .get_subscription_details(tenant_id, subscription_id)
            .await
            .map_err(store_status)?;
        let (trial, offer) = self
            .services
            .subscription_trial(tenant_id, subscription_id)
            .await
            .map_err(store_status)?
            .ok_or_else(|| Status::failed_precondition("the subscription has no trial"))?;
        if !matches!(
            trial.status(),
            TrialStatus::Active | TrialStatus::AwaitingDecision
        ) {
            return Err(Status::failed_precondition(format!(
                "the trial ended on {}",
                trial.until
            )));
        }
        let today = common_clock::now().date_naive().max(trial.from);
        let without_method = details.subscription.charge_automatically
            && !has_payment_method(&self.store, tenant_id, subscription_id).await?;
        let action = match end_action_of(&trial, offer.as_ref(), &details) {
            TrialEndAction::Convert | TrialEndAction::Manual if without_method => {
                match trial.without_card {
                    TrialWithoutCard::AwaitPayment => TrialEndAction::Convert,
                    TrialWithoutCard::Cancel => TrialEndAction::Cancel,
                    TrialWithoutCard::Switch { plan_version_id } => {
                        TrialEndAction::Fallback { plan_version_id }
                    }
                }
            }
            TrialEndAction::Manual => TrialEndAction::Convert,
            other => other,
        };
        let currency = details.subscription.currency.clone();
        let (outcome, invoice) = match action {
            TrialEndAction::Convert | TrialEndAction::Manual => (
                if without_method {
                    TrialOutcome::AwaitingPayment
                } else {
                    TrialOutcome::Converted
                },
                Some(estimate(
                    opening_total(&self.services, &details, today).await?,
                    &currency,
                    today,
                )),
            ),
            TrialEndAction::Cancel => (TrialOutcome::Cancelled, None),
            TrialEndAction::Fallback { plan_version_id } => {
                let full = self
                    .services
                    .preview_plan_change_in_full(
                        subscription_id,
                        tenant_id,
                        plan_version_id,
                        Vec::new(),
                        Some(PlanChangeMode::Immediate),
                    )
                    .await
                    .map_err(store_status)?;
                (
                    TrialOutcome::Switched,
                    full.outcome
                        .invoice
                        .map(|invoice| estimate(invoice.total, &currency, today)),
                )
            }
        };
        let outcome = match outcome {
            TrialOutcome::Converted => proto2::trial_config::Status::Converted,
            TrialOutcome::Cancelled => proto2::trial_config::Status::Cancelled,
            TrialOutcome::Switched => proto2::trial_config::Status::Switched,
            TrialOutcome::AwaitingPayment => proto2::trial_config::Status::AwaitingPayment,
            TrialOutcome::AwaitingDecision => proto2::trial_config::Status::AwaitingDecision,
        };
        Ok(Response::new(proto::PreviewEndTrialResponse {
            outcome: outcome as i32,
            effective_date: today.as_proto(),
            invoice,
        }))
    }
}
