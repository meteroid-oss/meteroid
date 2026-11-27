//! The one resolution of a creation's trial (R-153): every entry point and every preview asks
//! through `resolve_trial_tx`.
use chrono::NaiveDate;
use common_domain::ids::{
    AliasOr, CustomerId, PlanId, PlanVersionId, ProductFamilyId, TenantId, TrialOfferId,
};
use diesel_models::PgConn;
use diesel_models::plans::PlanRow;

use super::{catalog, db, eligibility, invalid};
use crate::StoreResult;
use crate::domain::trials::{
    MAX_TRIAL_DAYS, ResolvedTrial, TrialEligibility, TrialEndAction, TrialLength, TrialOffer,
    TrialRequest,
};

/// The plan a creation is on, as its trial reads it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlanRef {
    pub plan_id: PlanId,
    pub product_family_id: ProductFamilyId,
    pub trial_offer_id: Option<TrialOfferId>,
}

impl PlanRef {
    pub(crate) async fn of_version_tx(
        conn: &mut PgConn,
        tenant_id: TenantId,
        plan_version_id: PlanVersionId,
    ) -> StoreResult<Self> {
        let plan = db(PlanRow::get_with_version(conn, plan_version_id, tenant_id).await)?.plan;
        Ok(PlanRef {
            plan_id: plan.id,
            product_family_id: plan.product_family_id,
            trial_offer_id: plan.trial_offer_id,
        })
    }
}

/// The trial a creation on `plan` billing from `billing_start` gets for `request`. With a
/// customer, an offer given once that the customer had gives none when asked by default, and is
/// refused when named (R-159); checked again under the customer's lock as the creation is written.
pub(crate) async fn resolve_trial_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    plan: &PlanRef,
    billing_start: NaiveDate,
    customer: Option<CustomerId>,
    request: &TrialRequest,
) -> StoreResult<Option<ResolvedTrial>> {
    let plan_trial = match plan.trial_offer_id {
        Some(id) => Some(catalog::find_tx(conn, tenant_id, &AliasOr::Id(id)).await?)
            .filter(|offer| offer.archived_at.is_none()),
        None => None,
    };
    match request {
        TrialRequest::None => Ok(None),
        TrialRequest::Plan => {
            let Some(offer) = plan_trial else {
                return Ok(None);
            };
            if let Some(customer) = customer
                && eligibility::given_tx(conn, tenant_id, customer, &offer, plan).await?
            {
                return Ok(None);
            }
            Ok(Some(of_offer(&offer, plan.plan_id)))
        }
        TrialRequest::Offer(id) => {
            let offer = catalog::find_tx(conn, tenant_id, &AliasOr::Id(*id))
                .await
                .ok()
                .filter(|offer| offer.archived_at.is_none())
                .ok_or_else(|| invalid(format!("no trial offer {id} to give")))?;
            if let Some(customer) = customer
                && eligibility::given_tx(conn, tenant_id, customer, &offer, plan).await?
            {
                return Err(already_had(&offer));
            }
            Ok(Some(of_offer(&offer, plan.plan_id)))
        }
        TrialRequest::Custom {
            length,
            free,
            trial_plan,
        } => {
            let days = match length {
                TrialLength::Days(0) => return Ok(None),
                TrialLength::Days(days) => *days,
                TrialLength::Until(until) => {
                    let days = (*until - billing_start).num_days();
                    if days <= 0 {
                        return Err(invalid(format!(
                            "a trial ends after its start, {billing_start}"
                        )));
                    }
                    u32::try_from(days).unwrap_or(u32::MAX)
                }
            };
            if days > MAX_TRIAL_DAYS {
                return Err(invalid(format!(
                    "a trial lasts {MAX_TRIAL_DAYS} days at most"
                )));
            }
            let trial_plan_id = trial_plan
                .unwrap_or_else(|| plan_trial.as_ref().and_then(|offer| offer.trial_plan_id))
                .filter(|id| *id != plan.plan_id);
            if let Some(id) = trial_plan_id {
                db(PlanRow::find_by_id(conn, id, tenant_id).await)
                    .map_err(|_| invalid(format!("no plan {id} for the trial to give")))?;
            }
            Ok(Some(ResolvedTrial {
                days,
                free: free.unwrap_or_else(|| plan_trial.as_ref().is_none_or(|offer| offer.free)),
                trial_plan_id,
                offer: None,
                end_action: TrialEndAction::Convert,
                eligibility: TrialEligibility::None,
            }))
        }
    }
}

fn of_offer(offer: &TrialOffer, plan_id: PlanId) -> ResolvedTrial {
    ResolvedTrial {
        days: offer.duration_days,
        free: offer.free,
        trial_plan_id: offer.trial_plan_id.filter(|id| *id != plan_id),
        offer: Some(offer.id),
        end_action: offer.end_action.clone(),
        eligibility: offer.eligibility,
    }
}

pub(super) fn already_had(offer: &TrialOffer) -> error_stack::Report<crate::errors::StoreError> {
    invalid(format!(
        "the customer has already had the trial of {}",
        offer.name
    ))
}

/// Whether a subscription created on `plan_id` asking no trial starts on a free one: its plan's
/// trial is free (a self-serve sign-up needs no payment method then).
pub(crate) async fn starts_free_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    plan_id: PlanId,
) -> StoreResult<bool> {
    Ok(catalog::plan_trials_tx(conn, tenant_id, &[plan_id])
        .await?
        .remove(&plan_id)
        .is_some_and(|trial| trial.offer.free))
}
