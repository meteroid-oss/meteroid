//! Offers and plans' trials. A plan's trial is the offer it names, edited from the plan in place
//! when it is the plan's alone, else given one of its own (R-153).
use std::collections::HashMap;

use common_domain::actor::Actor;
use common_domain::ids::{AliasOr, BaseId, PlanId, TenantId, TrialOfferId};
use diesel_models::PgConn;
use diesel_models::plans::PlanRow;
use diesel_models::trial_offers::TrialOfferRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

use super::{db, invalid};
use crate::StoreResult;
use crate::domain::entity_activity::{Activity, ActivityType, AuditInput, EntityType};
use crate::domain::trials::{
    MAX_TRIAL_DAYS, PlanTrial, TrialEligibility, TrialEndAction, TrialOffer, TrialOfferNew,
    TrialOfferPatch, TrialTerms,
};
use crate::errors::StoreError;
use crate::services::Services;

pub(crate) async fn find_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    id_or_code: &AliasOr<TrialOfferId>,
) -> StoreResult<TrialOffer> {
    let row = match id_or_code {
        AliasOr::Id(id) => db(TrialOfferRow::find(conn, &tenant_id, *id).await)?,
        AliasOr::Alias(code) => db(TrialOfferRow::find_by_code(conn, &tenant_id, code).await)?,
    };
    TrialOffer::try_from(
        row.ok_or_else(|| Report::new(StoreError::ValueNotFound("trial offer".to_string())))?,
    )
}

pub(crate) async fn plans_of_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    offers: &[TrialOfferId],
) -> StoreResult<HashMap<TrialOfferId, Vec<PlanId>>> {
    let mut plans: HashMap<TrialOfferId, Vec<PlanId>> = HashMap::new();
    for (offer, plan) in db(TrialOfferRow::plans_of(conn, &tenant_id, offers).await)? {
        plans.entry(offer).or_default().push(plan);
    }
    Ok(plans)
}

/// The trial of each of `plans` that has one, with the plans sharing its offer.
pub(crate) async fn plan_trials_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    plans: &[PlanId],
) -> StoreResult<HashMap<PlanId, PlanTrial>> {
    let offers = db(TrialOfferRow::of_plans(conn, &tenant_id, plans).await)?;
    let ids: Vec<_> = offers.iter().map(|(_, row)| row.id).collect();
    let sharing = plans_of_tx(conn, tenant_id, &ids).await?;
    offers
        .into_iter()
        .map(|(plan, row)| {
            let plans = sharing.get(&row.id).cloned().unwrap_or_else(|| vec![plan]);
            Ok((
                plan,
                PlanTrial {
                    offer: TrialOffer::try_from(row)?,
                    plans,
                },
            ))
        })
        .collect()
}

impl Services {
    pub(in crate::services) async fn create_trial_offer(
        &self,
        tenant_id: TenantId,
        offer: TrialOfferNew,
    ) -> StoreResult<TrialOffer> {
        let mut conn = self.store.get_conn().await?;
        let row = offer_row(&mut conn, tenant_id, TrialOfferId::new(), offer, None).await?;
        TrialOffer::try_from(db(row.insert(&mut conn).await)?)
    }

    /// The offer's terms edited: trials to come take them, those it started keep theirs.
    pub(in crate::services) async fn update_trial_offer(
        &self,
        tenant_id: TenantId,
        id: TrialOfferId,
        patch: TrialOfferPatch,
    ) -> StoreResult<TrialOffer> {
        let mut conn = self.store.get_conn().await?;
        update_offer_tx(&mut conn, tenant_id, id, patch).await
    }

    pub(in crate::services) async fn archive_trial_offer(
        &self,
        tenant_id: TenantId,
        id: TrialOfferId,
    ) -> StoreResult<Vec<PlanId>> {
        self.store
            .transaction(|conn| {
                async move {
                    db(TrialOfferRow::archive(conn, &tenant_id, id).await)?;
                    db(TrialOfferRow::unset_on_plans(conn, &tenant_id, id).await)
                }
                .scope_boxed()
            })
            .await
    }

    pub(in crate::services) async fn set_plan_trial_offer(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        plan_id: PlanId,
        offer: Option<TrialOfferId>,
    ) -> StoreResult<Option<PlanTrial>> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    set_plan_trial_offer_tx(
                        &self.store.internal,
                        conn,
                        actor,
                        tenant_id,
                        plan_id,
                        offer,
                    )
                    .await?;
                    Ok(plan_trials_tx(conn, tenant_id, &[plan_id])
                        .await?
                        .remove(&plan_id))
                }
                .scope_boxed()
            })
            .await
    }
}

/// The plan's trial set to a catalog offer, or removed, in the caller's transaction.
pub(crate) async fn set_plan_trial_offer_tx(
    internal: &crate::store::StoreInternal,
    conn: &mut PgConn,
    actor: &Actor,
    tenant_id: TenantId,
    plan_id: PlanId,
    offer: Option<TrialOfferId>,
) -> StoreResult<()> {
    db(PlanRow::find_by_id(conn, plan_id, tenant_id).await)?;
    if let Some(offer) = offer {
        let found = db(TrialOfferRow::find(conn, &tenant_id, offer).await)?;
        if found.is_none_or(|row| row.archived_at.is_some()) {
            return Err(invalid(format!("no trial offer {offer} to give")));
        }
    }
    db(TrialOfferRow::set_for_plan(conn, &tenant_id, plan_id, offer).await)?;
    audit_tx(internal, conn, actor, tenant_id, plan_id).await
}

async fn update_offer_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    id: TrialOfferId,
    patch: TrialOfferPatch,
) -> StoreResult<TrialOffer> {
    let current = find_tx(conn, tenant_id, &AliasOr::Id(id)).await?;
    if current.archived_at.is_some() {
        return Err(invalid("an archived trial offer is not edited"));
    }
    let row = offer_row(
        conn,
        tenant_id,
        id,
        current.patched(patch),
        current.code.as_deref(),
    )
    .await?;
    TrialOffer::try_from(db(row.update_terms(conn).await)?)
}

/// The plan's trial set to `terms`, in the caller's transaction: its offer edited in place when no
/// other plan names it, else a new offer of its own, the offer's other settings kept. `None`
/// removes it; an offer then named by no plan and without a code is archived.
pub(crate) async fn set_plan_trial_tx(
    internal: &crate::store::StoreInternal,
    conn: &mut PgConn,
    actor: &Actor,
    tenant_id: TenantId,
    plan_id: PlanId,
    terms: Option<TrialTerms>,
) -> StoreResult<()> {
    let plan = db(PlanRow::find_by_id(conn, plan_id, tenant_id).await)?;
    let current = match plan.trial_offer_id {
        Some(id) => Some(find_tx(conn, tenant_id, &AliasOr::Id(id)).await?),
        None => None,
    };
    let shared = match &current {
        Some(offer) => plans_of_tx(conn, tenant_id, &[offer.id])
            .await?
            .get(&offer.id)
            .is_some_and(|plans| plans.iter().any(|p| *p != plan_id)),
        None => false,
    };
    match (terms, current) {
        (None, None) => return Ok(()),
        (None, Some(offer)) => {
            db(TrialOfferRow::set_for_plan(conn, &tenant_id, plan_id, None).await)?;
            if !shared && offer.code.is_none() {
                db(TrialOfferRow::archive(conn, &tenant_id, offer.id).await)?;
            }
        }
        (Some(terms), Some(offer)) if offer.terms() == normalized(terms, plan_id) => {
            return Ok(());
        }
        (Some(terms), Some(offer)) if !shared && offer.archived_at.is_none() => {
            let terms = normalized(terms, plan_id);
            update_offer_tx(
                conn,
                tenant_id,
                offer.id,
                TrialOfferPatch {
                    duration_days: Some(terms.duration_days),
                    free: Some(terms.free),
                    trial_plan_id: Some(terms.trial_plan_id),
                    ..TrialOfferPatch::default()
                },
            )
            .await?;
        }
        (Some(terms), current) => {
            let terms = normalized(terms, plan_id);
            let (end_action, eligibility) = current
                .map(|offer| (offer.end_action, offer.eligibility))
                .unwrap_or((TrialEndAction::Convert, TrialEligibility::None));
            let row = offer_row(
                conn,
                tenant_id,
                TrialOfferId::new(),
                TrialOfferNew {
                    name: format!("{} trial", plan.name.chars().take(200).collect::<String>()),
                    code: None,
                    duration_days: terms.duration_days,
                    free: terms.free,
                    trial_plan_id: terms.trial_plan_id,
                    end_action,
                    eligibility,
                },
                None,
            )
            .await?;
            let offer = db(row.insert(conn).await)?;
            db(TrialOfferRow::set_for_plan(conn, &tenant_id, plan_id, Some(offer.id)).await)?;
        }
    }
    audit_tx(internal, conn, actor, tenant_id, plan_id).await
}

/// A trial giving the plan's own features gives none of another.
fn normalized(terms: TrialTerms, plan_id: PlanId) -> TrialTerms {
    TrialTerms {
        trial_plan_id: terms.trial_plan_id.filter(|id| *id != plan_id),
        ..terms
    }
}

async fn audit_tx(
    internal: &crate::store::StoreInternal,
    conn: &mut PgConn,
    actor: &Actor,
    tenant_id: TenantId,
    plan_id: PlanId,
) -> StoreResult<()> {
    let activity = Activity::new(
        ActivityType::EntityUpdated,
        EntityType::Plan,
        plan_id.as_uuid(),
    )
    .with_metadata(serde_json::json!({ "changes": [{ "field": "trial" }] }));
    internal
        .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
        .await
}

/// The row an offer is stored as, its terms checked; `own_code`, an edited offer's code, is not
/// taken by itself.
async fn offer_row(
    conn: &mut PgConn,
    tenant_id: TenantId,
    id: TrialOfferId,
    offer: TrialOfferNew,
    own_code: Option<&str>,
) -> StoreResult<TrialOfferRow> {
    if offer.name.trim().is_empty() || !(1..=MAX_TRIAL_DAYS).contains(&offer.duration_days) {
        return Err(invalid(format!(
            "a trial offer is named and lasts 1 to {MAX_TRIAL_DAYS} days"
        )));
    }
    if let Some(plan) = offer.trial_plan_id {
        db(PlanRow::find_by_id(conn, plan, tenant_id).await)
            .map_err(|_| invalid(format!("no plan {plan} for the trial to give")))?;
    }
    let fallback = match &offer.end_action {
        TrialEndAction::Fallback { plan_version_id } => {
            if !offer.free {
                return Err(invalid(
                    "a trial falling back to another plan is free: the switch is on the billing start",
                ));
            }
            let version = db(PlanRow::get_with_version(conn, *plan_version_id, tenant_id).await)?
                .version
                .ok_or_else(|| invalid("no such plan version to fall back to"))?;
            if version.is_draft_version {
                return Err(invalid("a trial falls back to a published plan version"));
            }
            Some(*plan_version_id)
        }
        _ => None,
    };
    let code = offer
        .code
        .as_deref()
        .map(str::trim)
        .filter(|code| !code.is_empty());
    if let Some(code) = code.filter(|code| Some(*code) != own_code) {
        crate::domain::catalog_codes::validate_code(code, "tro_").map_err(Report::new)?;
        if db(TrialOfferRow::find_by_code(conn, &tenant_id, code).await)?.is_some() {
            return Err(Report::new(StoreError::DuplicateValue {
                entity: "trial offer",
                key: Some(code.to_string()),
            }));
        }
    }
    let now = common_clock::now().naive_utc();
    Ok(TrialOfferRow {
        id,
        tenant_id,
        name: offer.name.trim().to_string(),
        code: code.map(str::to_string),
        duration_days: i32::try_from(offer.duration_days).unwrap_or(1),
        free: offer.free,
        trial_plan_id: offer.trial_plan_id,
        end_action: offer.end_action.as_str().to_string(),
        fallback_plan_version_id: fallback,
        eligibility_scope: offer.eligibility.as_str().to_string(),
        archived_at: None,
        created_at: now,
        updated_at: now,
    })
}
