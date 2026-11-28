//! The catalog of offers and plans' start offers (R-170). An offer is a `trial_offer` row and its
//! grants; a discount or credit grant owns a coupon row of its own, archived, which billing and
//! the credit engine read as they read any coupon.
use std::collections::HashMap;

use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CouponId, OfferGrantId, OfferId, PlanId, TenantId};
use diesel_models::PgConn;
use diesel_models::add_ons::AddOnRow;
use diesel_models::coupons::{CouponRow, CouponRowNew, CouponStatusRowPatch};
use diesel_models::offers::{OfferApplicationRow, OfferGrantRow};
use diesel_models::plans::PlanRow;
use diesel_models::trial_offers::TrialOfferRow;
use error_stack::Report;
use rust_decimal::Decimal;

use super::{db, invalid};
use crate::StoreResult;
use crate::domain::coupons::{
    Coupon, CouponDiscount, fixed_amounts_to_row, validate_fixed_amounts,
};
use crate::domain::offers::{
    GrantDefinition, GrantDuration, Offer, OfferGrant, OfferGrantNew, OfferNew, OfferPatch,
    OfferTrial, PlanStartOffer, StartOfferChange, grant_of_row, grant_row,
};
use crate::domain::trials::{
    MAX_TRIAL_DAYS, TRIAL_IS_FREE, TrialEligibility, TrialEndAction, TrialTerms, TrialWithoutCard,
    end_action_of,
};
use crate::errors::{StoreError, TrialRefusal};

/// The offers of `rows` with their live grants.
pub(crate) async fn offers_of_rows(
    conn: &mut PgConn,
    tenant_id: TenantId,
    rows: Vec<TrialOfferRow>,
) -> StoreResult<Vec<Offer>> {
    let ids: Vec<OfferId> = rows.iter().map(|row| row.id).collect();
    let mut grants: HashMap<OfferId, Vec<OfferGrant>> = HashMap::new();
    let grant_rows = db(OfferGrantRow::list_live(conn, &tenant_id, &ids).await)?;
    for grant in grants_of_rows(conn, tenant_id, grant_rows).await? {
        grants.entry(grant.offer_id).or_default().push(grant);
    }
    rows.into_iter()
        .map(|row| {
            let of = grants.remove(&row.id).unwrap_or_default();
            offer_of_row(row, of)
        })
        .collect()
}

/// Grants by id, those an edit replaced included.
pub(crate) async fn grants_by_ids_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    ids: &[OfferGrantId],
) -> StoreResult<HashMap<OfferGrantId, OfferGrant>> {
    let rows = db(OfferGrantRow::list_by_ids(conn, &tenant_id, ids).await)?;
    Ok(grants_of_rows(conn, tenant_id, rows)
        .await?
        .into_iter()
        .map(|grant| (grant.id, grant))
        .collect())
}

async fn grants_of_rows(
    conn: &mut PgConn,
    tenant_id: TenantId,
    rows: Vec<OfferGrantRow>,
) -> StoreResult<Vec<OfferGrant>> {
    let coupon_ids: Vec<CouponId> = rows.iter().filter_map(|row| row.coupon_id).collect();
    let coupons: HashMap<CouponId, Coupon> = if coupon_ids.is_empty() {
        HashMap::new()
    } else {
        let rows = db(CouponRow::list_by_ids(conn, &coupon_ids, &tenant_id).await)?;
        crate::repositories::coupons::coupons_with_scope(conn, rows)
            .await?
            .into_iter()
            .map(|coupon| (coupon.id, coupon))
            .collect()
    };
    rows.into_iter()
        .map(|row| {
            let coupon = row.coupon_id.and_then(|id| coupons.get(&id));
            grant_of_row(row, coupon)
        })
        .collect()
}

fn offer_of_row(row: TrialOfferRow, grants: Vec<OfferGrant>) -> StoreResult<Offer> {
    let bad = |what: &str| invalid(format!("offer {}: {what}", row.id));
    let trial = match row.duration_days {
        None => None,
        Some(days) => Some(OfferTrial {
            days: u32::try_from(days).unwrap_or(1),
            free: row.free,
            trial_plan_id: row.trial_plan_id,
            card_required: row.card_required,
            end_action: end_action_of(&row.end_action, row.fallback_plan_version_id)
                .ok_or_else(|| bad("unknown end action"))?,
            without_card: TrialWithoutCard::parse(
                &row.without_card,
                row.without_card_plan_version_id,
            )
            .ok_or_else(|| bad("unknown end without a card"))?,
        }),
    };
    Ok(Offer {
        id: row.id,
        name: row.name,
        description: row.description,
        trial,
        grants,
        eligibility: TrialEligibility::parse(&row.eligibility_scope)
            .ok_or_else(|| bad("unknown eligibility"))?,
        standalone: row.standalone,
        archived_at: row.archived_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

pub(crate) async fn find_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    id: OfferId,
) -> StoreResult<Offer> {
    let row = db(TrialOfferRow::find(conn, &tenant_id, id).await)?
        .ok_or_else(|| Report::new(StoreError::ValueNotFound(format!("offer {id}"))))?;
    offers_of_rows(conn, tenant_id, vec![row])
        .await?
        .pop()
        .ok_or_else(|| Report::new(StoreError::ValueNotFound(format!("offer {id}"))))
}

/// Whether the plan's offer gives grants, or no trial: the plan's trial is then edited as part
/// of its start offer, its grants kept.
pub(crate) async fn gives_more_than_a_trial_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    offer_id: OfferId,
) -> StoreResult<bool> {
    let offer = find_tx(conn, tenant_id, offer_id).await?;
    Ok(offer.trial.is_none() || !offer.grants.is_empty())
}

/// The start offer of each of `plans` that names one, never archived, with the plans sharing it.
pub(crate) async fn plan_start_offers_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    plans: &[PlanId],
) -> StoreResult<HashMap<PlanId, PlanStartOffer>> {
    let rows = db(PlanRow::list_by_ids(conn, plans, tenant_id).await)?;
    let named: Vec<(PlanId, OfferId)> = rows
        .iter()
        .filter_map(|plan| plan.trial_offer_id.map(|offer| (plan.id, offer)))
        .collect();
    let offer_ids: Vec<OfferId> = named.iter().map(|(_, offer)| *offer).collect();
    let offer_rows = db(TrialOfferRow::list_by_ids(conn, &tenant_id, &offer_ids).await)?;
    let offers: HashMap<OfferId, Offer> = offers_of_rows(conn, tenant_id, offer_rows)
        .await?
        .into_iter()
        .filter(|offer| offer.archived_at.is_none())
        .map(|offer| (offer.id, offer))
        .collect();
    let sharing =
        crate::services::trials::catalog::plans_of_tx(conn, tenant_id, &offer_ids).await?;
    Ok(named
        .into_iter()
        .filter_map(|(plan, offer)| {
            let offer = offers.get(&offer)?.clone();
            let plans = sharing
                .get(&offer.id)
                .cloned()
                .unwrap_or_else(|| vec![plan]);
            Some((plan, PlanStartOffer { offer, plans }))
        })
        .collect())
}

/// An offer's trial checked: free, 1 to 3,650 days, its plans the tenant's and published where
/// it switches.
async fn checked_trial_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    trial: &OfferTrial,
) -> StoreResult<()> {
    if !trial.free {
        return Err(TrialRefusal::Billed.report(TRIAL_IS_FREE));
    }
    if !(1..=MAX_TRIAL_DAYS).contains(&trial.days) {
        return Err(invalid(format!("a trial lasts 1 to {MAX_TRIAL_DAYS} days")));
    }
    if let Some(plan) = trial.trial_plan_id {
        db(PlanRow::find_by_id(conn, plan, tenant_id).await)
            .map_err(|_| invalid(format!("no plan {plan} for the trial to give")))?;
    }
    if let TrialEndAction::Fallback { plan_version_id } = trial.end_action {
        crate::services::trials::catalog::published_tx(conn, tenant_id, plan_version_id).await?;
    }
    if let TrialWithoutCard::Switch { plan_version_id } = trial.without_card {
        crate::services::trials::catalog::published_tx(conn, tenant_id, plan_version_id).await?;
    }
    Ok(())
}

/// The offer's row: its trial's columns, the defaults when it has none.
#[allow(clippy::too_many_arguments)]
fn offer_row(
    id: OfferId,
    tenant_id: TenantId,
    name: &str,
    description: Option<&str>,
    trial: Option<&OfferTrial>,
    eligibility: TrialEligibility,
    standalone: bool,
    created_at: chrono::NaiveDateTime,
) -> TrialOfferRow {
    let end_action = trial.map_or(TrialEndAction::Convert, |t| t.end_action);
    let without_card = trial.map_or(TrialWithoutCard::AwaitPayment, |t| t.without_card);
    TrialOfferRow {
        id,
        tenant_id,
        name: name.trim().to_string(),
        code: None,
        duration_days: trial.map(|t| i32::try_from(t.days).unwrap_or(1)),
        free: trial.is_none_or(|t| t.free),
        trial_plan_id: trial.and_then(|t| t.trial_plan_id),
        end_action: end_action.as_str().to_string(),
        fallback_plan_version_id: match end_action {
            TrialEndAction::Fallback { plan_version_id } => Some(plan_version_id),
            _ => None,
        },
        eligibility_scope: eligibility.as_str().to_string(),
        archived_at: None,
        created_at,
        updated_at: common_clock::now().naive_utc(),
        card_required: trial.is_none_or(|t| t.card_required),
        without_card: without_card.as_str().to_string(),
        without_card_plan_version_id: without_card.plan_version_id(),
        standalone,
        description: description
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_string),
    }
}

fn checked_name(name: &str, description: Option<&str>) -> StoreResult<()> {
    if name.trim().is_empty() || name.chars().count() > 200 {
        return Err(invalid("an offer is named, in 200 characters at most"));
    }
    if description.is_some_and(|d| d.chars().count() > 1000) {
        return Err(invalid(
            "an offer's description is 1,000 characters at most",
        ));
    }
    Ok(())
}

/// The coupon row a discount or credit grant owns: no code anyone redeems (archived, under an
/// id-derived code until codes can be NULL), the discount and scope billing reads.
async fn grant_coupon_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    label: &str,
    grant: &OfferGrantNew,
) -> StoreResult<Option<CouponId>> {
    let (discount, fixed_amounts, add_on_ids, duration_months) = match &grant.definition {
        GrantDefinition::Discount(discount) => {
            match &discount.discount {
                CouponDiscount::Percentage(rate)
                    if *rate <= Decimal::ZERO || *rate > Decimal::ONE_HUNDRED =>
                {
                    return Err(invalid("a percentage discount is above 0 and at most 100"));
                }
                CouponDiscount::Fixed { amount, .. } if *amount <= Decimal::ZERO => {
                    return Err(invalid("a fixed discount is a positive amount"));
                }
                CouponDiscount::Credit { .. } => {
                    return Err(invalid("a discount grant takes a percentage or an amount"));
                }
                _ => {}
            }
            validate_fixed_amounts(&discount.discount, &discount.fixed_amounts)
                .map_err(Report::new)?;
            let months = match grant.window.duration {
                GrantDuration::Months(months) => Some(i32::try_from(months).unwrap_or(i32::MAX)),
                _ => None,
            };
            (
                discount.discount.clone(),
                fixed_amounts_to_row(&discount.fixed_amounts)?,
                discount.add_on_ids.clone(),
                months,
            )
        }
        GrantDefinition::Credit(credit) => {
            let discount = CouponDiscount::Credit {
                currency: credit.currency.clone(),
                amount: credit.amount,
                expires_in_days: match grant.window.duration {
                    GrantDuration::Days(days) => Some(days),
                    _ => None,
                },
                scope: credit.scope,
            };
            discount.validate().map_err(Report::new)?;
            (discount, None, Vec::new(), None)
        }
        GrantDefinition::Entitlement(_) | GrantDefinition::AddOn(_) => return Ok(None),
    };
    let id = CouponId::new();
    let row = CouponRowNew {
        id,
        code: crate::domain::coupons::grant_code(id),
        description: label.chars().take(200).collect(),
        tenant_id,
        discount: (&discount).try_into()?,
        reusable: true,
        fixed_amounts,
        duration_months,
        ..CouponRowNew::default()
    };
    db(row.insert(conn).await)?;
    db(CouponStatusRowPatch {
        id,
        tenant_id,
        archived_at: Some(Some(common_clock::now().naive_utc())),
        disabled: None,
    }
    .patch(conn)
    .await)?;
    crate::repositories::coupons::replace_coupon_add_ons(conn, tenant_id, id, &add_on_ids).await?;
    Ok(Some(id))
}

/// The offer's grants written, in order, each checked against its kind's windows (R-171).
async fn write_grants_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    offer_id: OfferId,
    label: &str,
    grants: &[OfferGrantNew],
) -> StoreResult<()> {
    let now = common_clock::now().naive_utc();
    let mut rows = Vec::with_capacity(grants.len());
    for (seq, grant) in grants.iter().enumerate() {
        grant
            .window
            .validate(grant.definition.kind())
            .map_err(Report::new)?;
        match &grant.definition {
            GrantDefinition::Entitlement(entitlement) => {
                db(diesel_models::entitlements::FeatureRow::find_by_id(
                    conn,
                    entitlement.feature_id,
                    tenant_id,
                )
                .await)
                .map_err(|_| invalid(format!("no feature {} to give", entitlement.feature_id)))?;
            }
            GrantDefinition::AddOn(add_on) => {
                if add_on.quantity < 1 {
                    return Err(invalid("an add-on grant gives at least one"));
                }
                db(AddOnRow::get_by_id(conn, tenant_id, add_on.add_on_id).await)
                    .map_err(|_| invalid(format!("no add-on {} to give", add_on.add_on_id)))?;
            }
            GrantDefinition::Discount(_) | GrantDefinition::Credit(_) => {}
        }
        let coupon = grant_coupon_tx(conn, tenant_id, label, grant).await?;
        rows.push(grant_row(
            tenant_id,
            offer_id,
            u32::try_from(seq).unwrap_or(u32::MAX),
            grant,
            coupon,
            now,
        )?);
    }
    db(OfferGrantRow::insert_batch(conn, &rows).await)
}

fn same_grants(current: &[OfferGrant], new: &[OfferGrantNew]) -> bool {
    current.len() == new.len()
        && current
            .iter()
            .zip(new)
            .all(|(c, n)| c.window == n.window && c.definition == n.definition)
}

/// The grants as they would be written again.
pub(crate) fn as_new(grants: &[OfferGrant]) -> Vec<OfferGrantNew> {
    grants
        .iter()
        .map(|grant| OfferGrantNew {
            window: grant.window,
            definition: grant.definition.clone(),
        })
        .collect()
}

pub(crate) async fn create_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    offer: OfferNew,
    standalone: bool,
) -> StoreResult<Offer> {
    checked_name(&offer.name, offer.description.as_deref())?;
    if offer.trial.is_none() && offer.grants.is_empty() {
        return Err(invalid("an offer gives a trial, grants or both"));
    }
    if let Some(trial) = &offer.trial {
        checked_trial_tx(conn, tenant_id, trial).await?;
    }
    let now = common_clock::now().naive_utc();
    let row = db(offer_row(
        OfferId::new(),
        tenant_id,
        &offer.name,
        offer.description.as_deref(),
        offer.trial.as_ref(),
        offer.eligibility,
        standalone,
        now,
    )
    .insert(conn)
    .await)?;
    write_grants_tx(conn, tenant_id, row.id, &row.name, &offer.grants).await?;
    find_tx(conn, tenant_id, row.id).await
}

/// A catalog offer edited, for applications to come: subscriptions it was applied to keep what
/// they were given (its replaced grants stay theirs).
pub(crate) async fn update_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    id: OfferId,
    patch: OfferPatch,
) -> StoreResult<Offer> {
    let current = find_tx(conn, tenant_id, id).await?;
    if !current.standalone {
        return Err(invalid(
            "this offer is a plan's or a coupon's own: it is edited there",
        ));
    }
    rewrite_tx(conn, tenant_id, current, patch).await
}

async fn rewrite_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    current: Offer,
    patch: OfferPatch,
) -> StoreResult<Offer> {
    if current.archived_at.is_some() {
        return Err(invalid("an archived offer is not edited"));
    }
    let name = patch.name.unwrap_or_else(|| current.name.clone());
    let description = patch
        .description
        .unwrap_or_else(|| current.description.clone());
    let trial = patch.trial.unwrap_or(current.trial);
    let eligibility = patch.eligibility.unwrap_or(current.eligibility);
    checked_name(&name, description.as_deref())?;
    let grants_given = match &patch.grants {
        Some(grants) => !grants.is_empty(),
        None => !current.grants.is_empty(),
    };
    if trial.is_none() && !grants_given {
        return Err(invalid("an offer gives a trial, grants or both"));
    }
    if let Some(trial) = &trial
        && current.trial.as_ref() != Some(trial)
    {
        checked_trial_tx(conn, tenant_id, trial).await?;
    }
    db(offer_row(
        current.id,
        tenant_id,
        &name,
        description.as_deref(),
        trial.as_ref(),
        eligibility,
        current.standalone,
        current.created_at,
    )
    .update_offer(conn)
    .await)?;
    if let Some(grants) = patch.grants
        && !same_grants(&current.grants, &grants)
    {
        db(OfferGrantRow::remove_live(
            conn,
            &tenant_id,
            current.id,
            common_clock::now().naive_utc(),
        )
        .await)?;
        write_grants_tx(conn, tenant_id, current.id, &name, &grants).await?;
    }
    find_tx(conn, tenant_id, current.id).await
}

/// Archived, a catalog offer is given no more and is no plan's start offer; the plans it was.
pub(crate) async fn archive_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    id: OfferId,
) -> StoreResult<Vec<PlanId>> {
    let offer = find_tx(conn, tenant_id, id).await?;
    if !offer.standalone {
        return Err(invalid(
            "this offer is a plan's or a coupon's own: it is removed there",
        ));
    }
    db(TrialOfferRow::archive(conn, &tenant_id, id).await)?;
    db(TrialOfferRow::unset_on_plans(conn, &tenant_id, id).await)
}

/// The plan's start offer written, in the caller's transaction (R-170): its own offer edited in
/// place when no other plan names it, else a new one of its own (given once per product family);
/// a catalog offer named; or none, its own offer then archived.
pub(crate) async fn set_plan_start_offer_tx(
    internal: &crate::store::StoreInternal,
    conn: &mut PgConn,
    actor: &Actor,
    tenant_id: TenantId,
    plan_id: PlanId,
    change: StartOfferChange,
) -> StoreResult<()> {
    let plan = db(PlanRow::find_by_id(conn, plan_id, tenant_id).await)?;
    let current = match plan.trial_offer_id {
        Some(id) => Some(find_tx(conn, tenant_id, id).await?),
        None => None,
    };
    let shared = match &current {
        Some(offer) => crate::services::trials::catalog::plans_of_tx(conn, tenant_id, &[offer.id])
            .await?
            .get(&offer.id)
            .is_some_and(|plans| plans.iter().any(|p| *p != plan_id)),
        None => false,
    };
    let own = current
        .clone()
        .filter(|offer| !offer.standalone && !shared && offer.archived_at.is_none());
    match change {
        StartOfferChange::None => {
            if current.is_none() {
                return Ok(());
            }
            db(TrialOfferRow::set_for_plan(conn, &tenant_id, plan_id, None).await)?;
            if let Some(own) = &own {
                db(TrialOfferRow::archive(conn, &tenant_id, own.id).await)?;
            }
        }
        StartOfferChange::Catalog(offer_id) => {
            if current.as_ref().is_some_and(|offer| offer.id == offer_id) {
                return Ok(());
            }
            let offer = find_tx(conn, tenant_id, offer_id).await?;
            if !offer.standalone || offer.archived_at.is_some() {
                return Err(invalid(format!("no catalog offer {offer_id} to give")));
            }
            db(TrialOfferRow::set_for_plan(conn, &tenant_id, plan_id, Some(offer_id)).await)?;
            if let Some(own) = &own {
                db(TrialOfferRow::archive(conn, &tenant_id, own.id).await)?;
            }
        }
        StartOfferChange::Inline {
            trial,
            grants,
            eligibility,
        } => {
            let current_trial = current.as_ref().and_then(|o| o.trial.map(|t| t.terms()));
            let trial = trial.map(|terms| {
                OfferTrial::from_terms(crate::services::trials::catalog::normalized(
                    terms,
                    plan_id,
                    current_trial.as_ref(),
                ))
            });
            if trial.is_none() && grants.is_empty() {
                return Box::pin(set_plan_start_offer_tx(
                    internal,
                    conn,
                    actor,
                    tenant_id,
                    plan_id,
                    StartOfferChange::None,
                ))
                .await;
            }
            match own {
                Some(own) => {
                    if own.trial == trial
                        && same_grants(&own.grants, &grants)
                        && eligibility.is_none_or(|e| e == own.eligibility)
                    {
                        return Ok(());
                    }
                    rewrite_tx(
                        conn,
                        tenant_id,
                        own,
                        OfferPatch {
                            trial: Some(trial),
                            grants: Some(grants),
                            eligibility,
                            ..OfferPatch::default()
                        },
                    )
                    .await?;
                }
                None => {
                    let plan_name: String = plan.name.chars().take(200).collect();
                    let offer = create_tx(
                        conn,
                        tenant_id,
                        OfferNew {
                            name: if trial.is_some() {
                                format!("{plan_name} trial")
                            } else {
                                format!("{plan_name} start offer")
                            },
                            description: None,
                            trial,
                            grants,
                            eligibility: eligibility.unwrap_or_else(|| {
                                current
                                    .as_ref()
                                    .map_or(TrialEligibility::ProductFamily, |o| o.eligibility)
                            }),
                        },
                        false,
                    )
                    .await?;
                    db(
                        TrialOfferRow::set_for_plan(conn, &tenant_id, plan_id, Some(offer.id))
                            .await,
                    )?;
                }
            }
        }
    }
    crate::services::trials::catalog::audit_tx(internal, conn, actor, tenant_id, plan_id).await
}

/// The plan editor's trial written on a start offer with grants: its grants kept.
pub(crate) async fn set_plan_trial_keeping_grants_tx(
    internal: &crate::store::StoreInternal,
    conn: &mut PgConn,
    actor: &Actor,
    tenant_id: TenantId,
    plan_id: PlanId,
    terms: Option<TrialTerms>,
) -> StoreResult<()> {
    let plan = db(PlanRow::find_by_id(conn, plan_id, tenant_id).await)?;
    let grants = match plan.trial_offer_id {
        Some(id) => as_new(&find_tx(conn, tenant_id, id).await?.grants),
        None => Vec::new(),
    };
    set_plan_start_offer_tx(
        internal,
        conn,
        actor,
        tenant_id,
        plan_id,
        StartOfferChange::Inline {
            trial: terms,
            grants,
            eligibility: None,
        },
    )
    .await
}

/// Who uses each of `offers`: the plans naming it, the coupons giving it, the retention offers.
pub(crate) async fn used_by_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    offers: &[OfferId],
) -> StoreResult<HashMap<OfferId, crate::domain::offers::OfferUse>> {
    let mut used: HashMap<OfferId, crate::domain::offers::OfferUse> = HashMap::new();
    for (offer, plans) in
        crate::services::trials::catalog::plans_of_tx(conn, tenant_id, offers).await?
    {
        used.entry(offer).or_default().plans = plans;
    }
    for coupon in db(CouponRow::list_by_offers(conn, tenant_id, offers).await)? {
        if let Some(offer) = coupon.offer_id {
            used.entry(offer)
                .or_default()
                .coupons
                .push(crate::domain::offers::OfferCoupon {
                    id: coupon.id,
                    code: coupon.code.unwrap_or_default(),
                });
        }
    }
    for (offer, count) in db(OfferApplicationRow::count_by_offers(conn, &tenant_id, offers).await)?
    {
        used.entry(offer).or_default().applications = u64::try_from(count).unwrap_or(0);
    }
    for (offer, retention) in
        db(diesel_models::query::offers::retention_offers_of(conn, tenant_id, offers).await)?
    {
        used.entry(offer)
            .or_default()
            .retention
            .push(common_domain::ids::RetentionOfferId::from(retention));
    }
    Ok(used)
}

/// The offer each of `coupons` gives, wrapping the coupons old pods made since (R1 of ADR 0019).
pub(crate) async fn offers_of_coupons_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    coupons: &[CouponId],
) -> StoreResult<HashMap<CouponId, OfferId>> {
    if coupons.is_empty() {
        return Ok(HashMap::new());
    }
    db(TrialOfferRow::wrap_coupons(conn, coupons).await)?;
    Ok(db(CouponRow::list_by_ids(conn, coupons, &tenant_id).await)?
        .into_iter()
        .filter_map(|row| row.offer_id.map(|offer| (row.id, offer)))
        .collect())
}
