//! The one resolution of a creation's offer and trial (R-170, R-172): every entry point asks
//! through `resolve_tx`, which gives the trial through `trials::resolve_trial_tx`.
use chrono::NaiveDate;
use common_domain::ids::{CustomerId, OfferId, TenantId};
use diesel_models::PgConn;
use diesel_models::offers::OfferApplicationRow;
use diesel_models::query::trial_offers::RedemptionScope;
use diesel_models::trial_offers::TrialRedemptionRow;

use super::{catalog, db, invalid};
use crate::StoreResult;
use crate::domain::offers::{Offer, OfferRequest, OfferSource, ResolvedOffer};
use crate::domain::trials::{ResolvedTrial, TrialEligibility, TrialRequest};
use crate::errors::OfferRefusal;
use crate::services::trials::PlanRef;

/// The trial and the offer a creation on `plan` billing from `billing_start` gets. The offer
/// asked by default (the plan's) gives nothing to a customer who already had it; named, or its
/// trial asked explicitly, it is refused then (R-173). A creation gets one trial: the chosen
/// offer's unless `trial` asks another (R-172).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn resolve_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    plan: &PlanRef,
    billing_start: NaiveDate,
    customer: Option<CustomerId>,
    offer: OfferRequest,
    trial: &TrialRequest,
    signed: bool,
) -> StoreResult<(Option<ResolvedTrial>, Option<ResolvedOffer>)> {
    let plan_offer = plan_offer_tx(conn, tenant_id, plan).await?;
    let (chosen, named) = match (offer, trial) {
        (OfferRequest::Offer(a), TrialRequest::Offer(b)) if a != *b => {
            return Err(invalid(
                "a creation is given one offer: its trial is that offer's",
            ));
        }
        (OfferRequest::Offer(id), _) => (Some(catalog_offer_tx(conn, tenant_id, id).await?), true),
        (OfferRequest::Plan, TrialRequest::Offer(id)) => {
            (Some(catalog_offer_tx(conn, tenant_id, *id).await?), true)
        }
        (OfferRequest::Plan, _) => (plan_offer.clone(), false),
        (OfferRequest::None, _) => (None, false),
    };
    let explicit = named || matches!(trial, TrialRequest::Plan);
    let chosen = match (chosen, customer) {
        (Some(offer), Some(customer)) => {
            if !given_tx(conn, tenant_id, customer, &offer, plan).await? {
                Some(offer)
            } else if explicit {
                return Err(OfferRefusal::AlreadyGiven.report(format!(
                    "the customer has already had this offer ({})",
                    offer.name
                )));
            } else {
                None
            }
        }
        (chosen, _) => chosen,
    };

    // The plan's trial stays the reference a custom trial's defaults are read from.
    let plan_trial = plan_offer
        .as_ref()
        .filter(|offer| offer.trial.is_some())
        .map(|offer| offer.id);
    let asked = match (trial, &chosen) {
        (TrialRequest::Default | TrialRequest::Offer(_), Some(offer)) if named => {
            match offer.trial {
                Some(_) => TrialRequest::Offer(offer.id),
                None => TrialRequest::None,
            }
        }
        (TrialRequest::Default, None) => TrialRequest::None,
        (asked, _) => asked.clone(),
    };
    let trial = crate::services::trials::resolve_trial_tx(
        conn,
        tenant_id,
        &PlanRef {
            trial_offer_id: plan_trial,
            ..*plan
        },
        billing_start,
        customer,
        &asked,
        signed,
    )
    .await?;

    let resolved = chosen
        .filter(|offer| !offer.grants.is_empty())
        .map(|offer| ResolvedOffer {
            offer_id: offer.id,
            source: if named {
                OfferSource::Operator
            } else {
                OfferSource::Plan
            },
            grants: offer.grants,
        });
    Ok((trial, resolved))
}

/// The plan's start offer, when live.
async fn plan_offer_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    plan: &PlanRef,
) -> StoreResult<Option<Offer>> {
    let Some(id) = plan.trial_offer_id else {
        return Ok(None);
    };
    Ok(Some(catalog::find_tx(conn, tenant_id, id).await?).filter(|o| o.archived_at.is_none()))
}

/// A live catalog offer named at a creation.
pub(crate) async fn catalog_offer_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    id: OfferId,
) -> StoreResult<Offer> {
    catalog::find_tx(conn, tenant_id, id)
        .await
        .ok()
        .filter(|offer| offer.standalone && offer.archived_at.is_none())
        .ok_or_else(|| OfferRefusal::Unavailable.report(format!("no offer {id} to give")))
}

/// Whether `customer` was already given what `offer` gives once (R-173): its trial as trials are
/// recorded (R-159), or the offer itself, or a plan's start offer on the same plan or family.
pub(crate) async fn given_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer: CustomerId,
    offer: &Offer,
    plan: &PlanRef,
) -> StoreResult<bool> {
    let scope = match offer.eligibility {
        TrialEligibility::None => return Ok(false),
        TrialEligibility::Offer => RedemptionScope::Offer(offer.id),
        TrialEligibility::Plan => RedemptionScope::Plan(plan.plan_id),
        TrialEligibility::ProductFamily => RedemptionScope::ProductFamily(plan.product_family_id),
    };
    if db(TrialRedemptionRow::count_given(conn, &tenant_id, customer, scope).await)? > 0 {
        return Ok(true);
    }
    let given = match scope {
        RedemptionScope::Offer(id) => {
            db(OfferApplicationRow::count_given(conn, &tenant_id, customer, id).await)?
        }
        scope => db(OfferApplicationRow::count_given_in_scope(
            conn, &tenant_id, customer, offer.id, scope,
        )
        .await)?,
    };
    Ok(given > 0)
}
