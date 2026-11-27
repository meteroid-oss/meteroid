//! What a customer is given once (R-159): every trial given at a creation is recorded with the
//! plan and family it starts on; an offer limited to itself, its plan or its family reads them.
use common_domain::actor::Actor;
use common_domain::ids::{CustomerId, TenantId};
use diesel_models::PgConn;
use diesel_models::query::trial_offers::RedemptionScope;
use diesel_models::subscriptions::SubscriptionRowNew;
use diesel_models::trial_offers::{TrialOfferRow, TrialRedemptionRow};

use super::resolve::{PlanRef, already_had};
use super::{db, invalid};
use crate::StoreResult;
use crate::domain::subscription_phases::{PhasePlan, PhaseTerms, SubscriptionPhaseInput};
use crate::domain::trials::{ResolvedTrial, TrialEligibility, TrialEndAction, TrialOffer};
use crate::services::Services;

fn scope_of(
    eligibility: TrialEligibility,
    offer: &TrialOffer,
    plan: &PlanRef,
) -> Option<RedemptionScope> {
    match eligibility {
        TrialEligibility::None => None,
        TrialEligibility::Offer => Some(RedemptionScope::Offer(offer.id)),
        TrialEligibility::Plan => Some(RedemptionScope::Plan(plan.plan_id)),
        TrialEligibility::ProductFamily => {
            Some(RedemptionScope::ProductFamily(plan.product_family_id))
        }
    }
}

/// Whether `customer` was already given what `offer` gives once on `plan`.
pub(super) async fn given_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer: CustomerId,
    offer: &TrialOffer,
    plan: &PlanRef,
) -> StoreResult<bool> {
    let Some(scope) = scope_of(offer.eligibility, offer, plan) else {
        return Ok(false);
    };
    Ok(db(TrialRedemptionRow::count_given(conn, &tenant_id, customer, scope).await)? > 0)
}

/// The trials of subscriptions just inserted recorded, under their customers' locks, and what
/// ends them written: the subscription's end date for one cancelling, a phase switching plan for
/// one falling back. One given once that the customer already had refuses the creation.
pub(in crate::services) async fn record_trials_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    trials: &[(&SubscriptionRowNew, &ResolvedTrial)],
) -> StoreResult<()> {
    for (subscription, trial) in trials {
        let tenant_id = subscription.tenant_id;
        let plan = PlanRef::of_version_tx(conn, tenant_id, subscription.plan_version_id).await?;
        if let Some(offer_id) = trial.offer
            && trial.eligibility != TrialEligibility::None
        {
            let offer = TrialOffer::try_from(
                db(TrialOfferRow::find(conn, &tenant_id, offer_id).await)?
                    .ok_or_else(|| invalid(format!("no trial offer {offer_id} to give")))?,
            )?;
            if given_tx(conn, tenant_id, subscription.customer_id, &offer, &plan).await? {
                return Err(already_had(&offer));
            }
        }
        db(TrialRedemptionRow::insert_batch(
            conn,
            &[TrialRedemptionRow {
                subscription_id: subscription.id,
                tenant_id,
                customer_id: subscription.customer_id,
                trial_offer_id: trial.offer,
                plan_id: plan.plan_id,
                product_family_id: plan.product_family_id,
                created_at: common_clock::now().naive_utc(),
            }],
        )
        .await)?;
        let until = trial.until(
            subscription
                .billing_start_date
                .unwrap_or(subscription.start_date),
        );
        match &trial.end_action {
            TrialEndAction::Convert => {}
            TrialEndAction::Cancel => {
                let end = subscription.end_date.map_or(until, |end| end.min(until));
                crate::services::facts::record_end_date_tx(
                    conn,
                    tenant_id,
                    subscription.id,
                    Some(end),
                    actor,
                )
                .await?;
            }
            TrialEndAction::Fallback { plan_version_id } => {
                let seq = crate::services::subscriptions::phases::list_phases_tx(
                    conn,
                    tenant_id,
                    subscription.id,
                )
                .await?
                .len();
                crate::services::subscriptions::phases::write_phase_tx(
                    services,
                    conn,
                    actor,
                    tenant_id,
                    subscription.id,
                    u32::try_from(seq).unwrap_or(u32::MAX),
                    &SubscriptionPhaseInput {
                        start_date: until,
                        label: Some("After the trial".to_string()),
                        terms: PhaseTerms {
                            plan: Some(PhasePlan {
                                plan_version_id: *plan_version_id,
                                option: None,
                                component_params: vec![],
                                name: None,
                            }),
                            ..PhaseTerms::default()
                        },
                    },
                )
                .await?;
            }
        }
    }
    Ok(())
}
