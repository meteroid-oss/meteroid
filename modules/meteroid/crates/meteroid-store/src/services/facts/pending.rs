//! Scheduled changes as the facts they become: a plan change resolved against its
//! target version, an amendment written as it is, each to the rows its event wrote when it
//! applied. A change scheduled now is written at once; a pending one is migrated (at deploy, or
//! when its event comes due), never applied.
use std::collections::{HashMap, HashSet};

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use common_domain::ids::{
    AddOnId, BaseId, PlanVersionId, PriceComponentId, PriceId, ProductId, ScheduledEventId,
    SlotTransactionId, SubscriptionAddOnId, SubscriptionId, SubscriptionPriceComponentId, TenantId,
};
use diesel_models::enums::BillingPeriodEnum as DbBillingPeriod;
use diesel_models::plan_component_prices::PlanComponentPriceRow;
use diesel_models::plan_version_add_ons::PlanVersionAddOnRow;
use diesel_models::plan_version_adjustments::{
    PlanVersionAdjustmentComponentRow, PlanVersionAdjustmentRow,
};
use diesel_models::plan_versions::PlanVersionRow;
use diesel_models::price_components::PriceComponentRow;
use diesel_models::prices::PriceRow;
use diesel_models::products::ProductRow;
use diesel_models::query::subscription_facts::ScheduledRows;
use diesel_models::scheduled_events::ScheduledEventRow;
use diesel_models::slot_transactions::SlotTransactionRow;
use diesel_models::subscription_add_ons::{SubscriptionAddOnRow, SubscriptionAddOnRowNew};
use diesel_models::subscription_adjustments::SubscriptionAdjustmentRow;
use diesel_models::subscription_components::{
    SubscriptionComponentRow, SubscriptionComponentRowNew,
};
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscription_facts::{
    SubscriptionEndingRow, SubscriptionPlanWindowRow, SubscriptionPlanWindowRowNew,
    SubscriptionTrialRow,
};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;
use serde::{Deserialize, Serialize};

use super::db;
use crate::StoreResult;
use crate::domain::SubscriptionStatusEnum;
use crate::domain::enums::{BillingPeriodEnum, ScheduledEventStatus};
use crate::domain::scheduled_events::{
    ComponentMapping, ResolvedAddOnInsert, ResolvedComponentInsert, ScheduledEvent,
    ScheduledEventData,
};
use crate::domain::slot_transactions::SlotTransactionNewInternal;
use crate::domain::subscription_add_ons::{SubscriptionAddOnNew, SubscriptionAddOnNewInternal};
use crate::domain::subscription_components::{
    SubscriptionComponentNew, SubscriptionComponentNewInternal, SubscriptionFee,
};
use crate::errors::StoreError;
use crate::services::ServicesEdge;
use crate::store::PgConn;

/// The rows a pending change writes when it applies on `on`, ids aside: rows it closes end on
/// `on`, rows it opens start on `on`.
#[derive(Debug, Clone)]
pub struct PendingFacts {
    pub event_id: ScheduledEventId,
    pub tenant_id: TenantId,
    pub subscription_id: SubscriptionId,
    pub on: NaiveDate,
    /// A plan change's window from `on`; an amendment keeps the plan.
    pub plan_window: Option<PlanWindowFact>,
    pub closes_components: Vec<SubscriptionPriceComponentId>,
    pub opens_components: Vec<SubscriptionComponentRowNew>,
    pub closes_add_ons: Vec<SubscriptionAddOnId>,
    pub opens_add_ons: Vec<SubscriptionAddOnRowNew>,
    /// The seat counts the new slot fees start from.
    pub seat_seeds: Vec<SeatSeed>,
    /// The seat counts a change sets on a seat fee it keeps, before the seats changed since it was
    /// scheduled (R-108).
    pub set_seats: Vec<SeatSeed>,
    /// When the change was scheduled, for one written on its date: what changed since carries.
    pub scheduled_at: Option<NaiveDateTime>,
    pub minimum: MinimumChange,
}

#[derive(Debug, Clone)]
pub struct PlanWindowFact {
    pub plan_version_id: PlanVersionId,
    /// `on`, or the start of a window already opened on or after it, which this one replaces.
    pub from_date: NaiveDate,
    pub cadence: BillingPeriodEnum,
    pub cadence_origin: NaiveDate,
    /// Where the trial ends once the change cuts it short.
    pub trial_until: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatSeed {
    pub unit: String,
    pub slots: i32,
}

/// The minimum from the change (`refresh_subscription_minimum_on_plan_change`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MinimumChange {
    /// None before or after, an operator's (a plan change keeps it), or an amendment.
    Kept,
    /// The plan's minimum ends: the target has none.
    Ended,
    /// The target version's minimum; `products` empty when it covers every component.
    Replaced {
        amount_subunit: i64,
        includes_all_components: bool,
        products: Vec<ProductId>,
    },
}

/// Why a pending change cannot be written as facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unresolvable {
    /// Neither an `ApplyPlanChange` nor an `ApplyAmendment`.
    NotAChange,
    /// Applied, cancelled, failed, claimed or migrated already.
    NotPending(ScheduledEventStatus),
    /// Another plan change landed first: applying this one does nothing.
    Superseded {
        expected: PlanVersionId,
        found: PlanVersionId,
    },
    /// Neither active nor trialing: applying it fails.
    SubscriptionNotActive(SubscriptionStatusEnum),
    /// A cancellation or the fixed term ends the subscription on or before the change.
    EndsBeforeTheChange(NaiveDateTime),
    TargetVersionGone(PlanVersionId),
    TargetVersionNotPublished(PlanVersionId),
    OtherCurrency {
        subscription: String,
        target: String,
    },
    /// A catalog component of the change left its plan version.
    ComponentGone(PriceComponentId),
    /// A price of the change is deleted, or no longer offered by its component.
    PriceGone(PriceId),
    ProductGone(ProductId),
    /// A fee row the change closes is no longer open.
    ComponentRowNotOpen(SubscriptionPriceComponentId),
    AddOnRowNotOpen(SubscriptionAddOnId),
    /// Writing it failed: the error, logged with the event.
    Failed(String),
}

pub type Resolution = Result<PendingFacts, Unresolvable>;

#[derive(Debug, Clone)]
pub enum Migration {
    Written(Box<PendingFacts>),
    /// Migrated by an earlier run: nothing written.
    AlreadyMigrated,
    /// A replica claimed it within the claim timeout, and may be applying it: left to it.
    Claimed,
}

/// When a pending change is written as facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Writing {
    /// Before its date (scheduled now, or at the deploy): its target must still offer each price
    /// it names. `leave_fresh_claims`: a change a replica claimed within the timeout is left.
    Ahead { leave_fresh_claims: bool },
    /// On its date, as the previous release applied it: each price as the event stored it.
    Due,
}

struct Context<'a> {
    event: &'a ScheduledEvent,
    writing: Writing,
    sub: &'a SubscriptionRow,
    on: NaiveDate,
    open_components: HashSet<SubscriptionPriceComponentId>,
    open_add_ons: Vec<(SubscriptionAddOnId, AddOnId)>,
}

impl Context<'_> {
    fn facts(&self) -> PendingFacts {
        PendingFacts {
            event_id: self.event.id,
            tenant_id: self.event.tenant_id,
            subscription_id: self.sub.id,
            on: self.on,
            plan_window: None,
            closes_components: Vec::new(),
            opens_components: Vec::new(),
            closes_add_ons: Vec::new(),
            opens_add_ons: Vec::new(),
            seat_seeds: Vec::new(),
            set_seats: Vec::new(),
            scheduled_at: (self.writing == Writing::Due).then_some(self.event.created_at),
            minimum: MinimumChange::Kept,
        }
    }
}

/// A catalog row a change names.
#[derive(Clone, Copy)]
struct Named {
    component: Option<PriceComponentId>,
    product: Option<ProductId>,
    price: Option<PriceId>,
}

/// The facts a pending change becomes, read and never written: the migration's dry run.
pub(crate) async fn resolve_pending_tx(
    conn: &mut PgConn,
    event: &ScheduledEvent,
    writing: Writing,
) -> StoreResult<Resolution> {
    let is_change = matches!(
        event.event_data,
        ScheduledEventData::ApplyPlanChange { .. } | ScheduledEventData::ApplyAmendment { .. }
    );
    if !is_change {
        return Ok(Err(Unresolvable::NotAChange));
    }
    // A straggler an old replica claimed is migrated too.
    if !matches!(
        event.status,
        ScheduledEventStatus::Pending | ScheduledEventStatus::Processing
    ) {
        return Ok(Err(Unresolvable::NotPending(event.status.clone())));
    }
    let sub =
        db(
            SubscriptionRow::get_subscription_by_id(conn, &event.tenant_id, event.subscription_id)
                .await,
        )?
        .subscription;
    let open_components = db(SubscriptionComponentRow::list_fact_rows(conn, sub.id).await)?
        .into_iter()
        .filter(|row| row.effective_to.is_none())
        .map(|row| row.id)
        .collect();
    let open_add_ons = db(SubscriptionAddOnRow::list_fact_rows(conn, sub.id).await)?
        .into_iter()
        .filter(|row| row.effective_to.is_none())
        .map(|row| (row.id, row.add_on_id))
        .collect();
    let ctx = Context {
        event,
        writing,
        sub: &sub,
        on: event.scheduled_time.date(),
        open_components,
        open_add_ons,
    };
    match &event.event_data {
        ScheduledEventData::ApplyPlanChange {
            source_plan_version_id,
            new_plan_version_id,
            component_mappings,
        } => {
            resolve_plan_change(
                conn,
                &ctx,
                *source_plan_version_id,
                *new_plan_version_id,
                component_mappings,
            )
            .await
        }
        ScheduledEventData::ApplyAmendment {
            component_close,
            component_insert,
            addon_close,
            addon_insert,
        } => {
            resolve_amendment(
                conn,
                &ctx,
                component_close,
                component_insert,
                addon_close,
                addon_insert,
            )
            .await
        }
        _ => Ok(Err(Unresolvable::NotAChange)),
    }
}

async fn resolve_plan_change(
    conn: &mut PgConn,
    ctx: &Context<'_>,
    source_plan_version_id: Option<PlanVersionId>,
    new_plan_version_id: PlanVersionId,
    mappings: &[ComponentMapping],
) -> StoreResult<Resolution> {
    let (sub, on, tenant_id) = (ctx.sub, ctx.on, ctx.event.tenant_id);
    if let Some(expected) = source_plan_version_id
        && sub.plan_version_id != expected
    {
        return Ok(Err(Unresolvable::Superseded {
            expected,
            found: sub.plan_version_id,
        }));
    }
    if let Some(why) = not_live_on(conn, sub, on).await? {
        return Ok(Err(why));
    }
    let Some(target) =
        db(
            PlanVersionRow::list_by_ids_and_tenant_id(conn, &[new_plan_version_id], tenant_id)
                .await,
        )?
        .into_iter()
        .next()
    else {
        return Ok(Err(Unresolvable::TargetVersionGone(new_plan_version_id)));
    };
    if target.is_draft_version {
        return Ok(Err(Unresolvable::TargetVersionNotPublished(
            new_plan_version_id,
        )));
    }
    if target.currency != sub.currency {
        return Ok(Err(Unresolvable::OtherCurrency {
            subscription: sub.currency.clone(),
            target: target.currency,
        }));
    }

    let components: HashSet<PriceComponentId> =
        db(PriceComponentRow::list_by_plan_version_id(conn, tenant_id, new_plan_version_id).await)?
            .into_iter()
            .map(|component| component.id)
            .collect();
    let component_ids: Vec<PriceComponentId> = components.iter().copied().collect();
    let offered: HashSet<(PriceComponentId, PriceId)> =
        db(PlanComponentPriceRow::list_by_component_ids(conn, &component_ids).await)?
            .into_iter()
            .map(|link| (link.plan_component_id, link.price_id))
            .collect();

    let mut named = Vec::new();
    for mapping in mappings {
        let closes = match mapping {
            ComponentMapping::Matched {
                current_component_id,
                target_component_id,
                product_id,
                price_id,
                ..
            } => {
                named.push(Named {
                    component: Some(*target_component_id),
                    product: Some(*product_id),
                    price: Some(*price_id),
                });
                Some(current_component_id)
            }
            ComponentMapping::Added {
                target_component_id,
                product_id,
                price_id,
                ..
            } => {
                named.push(Named {
                    component: Some(*target_component_id),
                    product: *product_id,
                    price: *price_id,
                });
                None
            }
            ComponentMapping::Removed {
                current_component_id,
            } => Some(current_component_id),
        };
        if let Some(row) = closes
            && !ctx.open_components.contains(row)
        {
            return Ok(Err(Unresolvable::ComponentRowNotOpen(*row)));
        }
    }
    let offered = (ctx.writing != Writing::Due).then_some(&offered);
    if let Some(why) = catalog_gone(conn, ctx, &named, &components, offered).await? {
        return Ok(Err(why));
    }

    let mut facts = ctx.facts();
    // The rows `process_apply_plan_change` writes, ids aside.
    for mapping in mappings {
        let internal = match mapping {
            ComponentMapping::Matched {
                current_component_id,
                target_component_id,
                product_id,
                price_id,
                name,
                fee,
                period,
                seats,
            } => {
                facts.closes_components.push(*current_component_id);
                if let (Some(slots), SubscriptionFee::Slot { unit, .. }) = (seats, fee) {
                    facts.set_seats.push(SeatSeed {
                        unit: unit.clone(),
                        slots: i32::try_from(*slots).unwrap_or(i32::MAX),
                    });
                }
                SubscriptionComponentNewInternal {
                    price_component_id: Some(*target_component_id),
                    product_id: Some(*product_id),
                    name: name.clone(),
                    period: *period,
                    fee: fee.clone(),
                    is_override: false,
                    price_id: Some(*price_id),
                    effective_from: on,
                }
            }
            ComponentMapping::Added {
                target_component_id,
                product_id,
                price_id,
                name,
                fee,
                period,
            } => {
                facts.seat_seeds.extend(seat_seed(fee, on));
                SubscriptionComponentNewInternal {
                    price_component_id: Some(*target_component_id),
                    product_id: *product_id,
                    name: name.clone(),
                    period: *period,
                    fee: fee.clone(),
                    is_override: false,
                    price_id: *price_id,
                    effective_from: on,
                }
            }
            ComponentMapping::Removed {
                current_component_id,
            } => {
                facts.closes_components.push(*current_component_id);
                continue;
            }
        };
        let row: SubscriptionComponentRowNew = SubscriptionComponentNew {
            subscription_id: sub.id,
            internal,
        }
        .try_into()?;
        facts.opens_components.push(row);
    }

    // Add-ons the target does not offer end with the old plan (`close_incompatible`).
    let offered_add_ons: HashSet<AddOnId> =
        db(
            PlanVersionAddOnRow::list_by_plan_version_id(conn, new_plan_version_id, tenant_id)
                .await,
        )?
        .into_iter()
        .map(|offer| offer.add_on_id)
        .collect();
    facts.closes_add_ons = ctx
        .open_add_ons
        .iter()
        .filter(|(_, add_on)| !offered_add_ons.contains(add_on))
        .map(|(row, _)| *row)
        .collect();

    facts.minimum = minimum_change(conn, sub.id, new_plan_version_id).await?;
    facts.plan_window = Some(plan_window(conn, sub, on, new_plan_version_id, mappings).await?);
    Ok(Ok(facts))
}

/// The window `record_plan_change_tx` opens for a change on `on`, read before it applies.
async fn plan_window(
    conn: &mut PgConn,
    sub: &SubscriptionRow,
    on: NaiveDate,
    plan_version_id: PlanVersionId,
    mappings: &[ComponentMapping],
) -> StoreResult<PlanWindowFact> {
    let cadence = ComponentMapping::derive_billing_period(mappings)
        .unwrap_or_else(|| BillingPeriodEnum::from(sub.period.clone()));
    let db_cadence: DbBillingPeriod = cadence.into();
    let trial = db(SubscriptionTrialRow::find(conn, sub.id).await)?;
    // `SubscriptionTrialRow::end_by`: a trial running past the change ends on it.
    let trial_until = trial.as_ref().and_then(|t| {
        let until = t.from_date.max(on);
        (t.until_date > until).then_some(until)
    });
    let (windows, from) = super::windows_before(conn, sub.id, on).await?;
    let cadence_origin = match (trial.as_ref(), trial_until) {
        (Some(t), Some(until)) if t.free => {
            origin_after_free_trial(conn, sub, &windows, from, &db_cadence, until).await?
        }
        _ => super::decided_origin(conn, sub, &windows, from, &db_cadence).await?,
    };
    Ok(PlanWindowFact {
        plan_version_id,
        from_date: from,
        cadence,
        cadence_origin,
        trial_until,
    })
}

/// [`super::decided_origin`] as it reads once the change has ended a free trial on `until`.
async fn origin_after_free_trial(
    conn: &mut PgConn,
    sub: &SubscriptionRow,
    before: &[SubscriptionPlanWindowRow],
    on: NaiveDate,
    cadence: &DbBillingPeriod,
    until: NaiveDate,
) -> StoreResult<NaiveDate> {
    if let Some(previous) = before.last()
        && super::months(&previous.cadence) == super::months(cadence)
    {
        return Ok(previous.cadence_origin);
    }
    if let Some(recorded) = db(SubscriptionEventRow::cadence_origin(conn, sub.id, on).await)? {
        return Ok(recorded);
    }
    let billing_start = sub.billing_start_date.unwrap_or(sub.current_period_start);
    Ok(super::origin_by_rule(
        until.max(billing_start),
        u32::from(sub.billing_day_anchor.unsigned_abs()),
        before,
        on,
        super::months(cadence),
    ))
}

/// What `refresh_subscription_minimum_on_plan_change` does to the minimum.
async fn minimum_change(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    target: PlanVersionId,
) -> StoreResult<MinimumChange> {
    let existing =
        db(SubscriptionAdjustmentRow::find_by_subscription_id(conn, &subscription_id).await)?;
    if existing.as_ref().is_some_and(|minimum| minimum.is_override) {
        return Ok(MinimumChange::Kept);
    }
    let Some(plan_minimum) =
        db(PlanVersionAdjustmentRow::find_by_plan_version_id(conn, &target).await)?
    else {
        return Ok(if existing.is_some() {
            MinimumChange::Ended
        } else {
            MinimumChange::Kept
        });
    };
    let products = if plan_minimum.includes_all_components {
        Vec::new()
    } else {
        db(PlanVersionAdjustmentComponentRow::list_by_plan_version_id(conn, &target).await)?
            .into_iter()
            .map(|scope| scope.product_id)
            .collect()
    };
    Ok(MinimumChange::Replaced {
        amount_subunit: plan_minimum.amount_subunit,
        includes_all_components: plan_minimum.includes_all_components,
        products,
    })
}

async fn resolve_amendment(
    conn: &mut PgConn,
    ctx: &Context<'_>,
    component_close: &[SubscriptionPriceComponentId],
    component_insert: &[ResolvedComponentInsert],
    addon_close: &[SubscriptionAddOnId],
    addon_insert: &[ResolvedAddOnInsert],
) -> StoreResult<Resolution> {
    let (sub, on) = (ctx.sub, ctx.on);
    if let Some(why) = not_live_on(conn, sub, on).await? {
        return Ok(Err(why));
    }
    if let Some(row) = component_close
        .iter()
        .find(|row| !ctx.open_components.contains(*row))
    {
        return Ok(Err(Unresolvable::ComponentRowNotOpen(*row)));
    }
    if let Some(row) = addon_close
        .iter()
        .find(|row| !ctx.open_add_ons.iter().any(|(open, _)| open == *row))
    {
        return Ok(Err(Unresolvable::AddOnRowNotOpen(*row)));
    }
    // An override names a component of the plan version it runs on.
    let components: HashSet<PriceComponentId> = db(PriceComponentRow::list_by_plan_version_id(
        conn,
        ctx.event.tenant_id,
        sub.plan_version_id,
    )
    .await)?
    .into_iter()
    .map(|component| component.id)
    .collect();
    let named: Vec<Named> = component_insert
        .iter()
        .map(|c| Named {
            component: c.price_component_id,
            product: c.product_id,
            price: c.price_id,
        })
        .chain(addon_insert.iter().map(|a| Named {
            component: None,
            product: a.product_id,
            price: a.price_id,
        }))
        .collect();
    if let Some(why) = catalog_gone(conn, ctx, &named, &components, None).await? {
        return Ok(Err(why));
    }

    let mut facts = ctx.facts();
    // The rows `process_apply_amendment` writes, ids aside.
    facts.closes_components = component_close.to_vec();
    facts.closes_add_ons = addon_close.to_vec();
    for c in component_insert {
        let mut row: SubscriptionComponentRowNew = SubscriptionComponentNew {
            subscription_id: sub.id,
            internal: SubscriptionComponentNewInternal {
                price_component_id: c.price_component_id,
                product_id: c.product_id,
                name: c.name.clone(),
                period: c.period,
                fee: c.fee.clone(),
                is_override: c.is_override,
                price_id: c.price_id,
                effective_from: on,
            },
        }
        .try_into()?;
        row.lineage_id = c.lineage_id;
        row.added_by_amendment = true;
        facts.opens_components.push(row);
        if !c.is_override {
            facts.seat_seeds.extend(seat_seed(&c.fee, on));
        }
    }
    for a in addon_insert {
        let mut row: SubscriptionAddOnRowNew = SubscriptionAddOnNew {
            subscription_id: sub.id,
            internal: SubscriptionAddOnNewInternal {
                add_on_id: a.add_on_id,
                name: a.name.clone(),
                period: a.period,
                fee: a.fee.clone(),
                product_id: a.product_id,
                price_id: a.price_id,
                quantity: a.quantity,
                effective_from: on,
            },
        }
        .try_into()?;
        row.lineage_id = a.lineage_id;
        row.added_by_amendment = true;
        facts.opens_add_ons.push(row);
        if a.lineage_id.is_none() {
            facts.seat_seeds.extend(seat_seed(&a.fee, on));
        }
    }
    Ok(Ok(facts))
}

/// Why the subscription will not take a change on `on`: not running, or ended by then.
async fn not_live_on(
    conn: &mut PgConn,
    sub: &SubscriptionRow,
    on: NaiveDate,
) -> StoreResult<Option<Unresolvable>> {
    let status = SubscriptionStatusEnum::from(sub.status.clone());
    if !matches!(
        status,
        SubscriptionStatusEnum::Active | SubscriptionStatusEnum::TrialActive
    ) {
        return Ok(Some(Unresolvable::SubscriptionNotActive(status)));
    }
    Ok(db(SubscriptionEndingRow::list_of(conn, sub.id).await)?
        .into_iter()
        .map(|ending| ending.at)
        .filter(|at| at.date() <= on)
        .min()
        .map(Unresolvable::EndsBeforeTheChange))
}

/// The first catalog row a change names that is gone, or priced in another currency. `offered`:
/// the prices each component of a target version offers.
async fn catalog_gone(
    conn: &mut PgConn,
    ctx: &Context<'_>,
    named: &[Named],
    components: &HashSet<PriceComponentId>,
    offered: Option<&HashSet<(PriceComponentId, PriceId)>>,
) -> StoreResult<Option<Unresolvable>> {
    let tenant_id = ctx.event.tenant_id;
    let price_ids: Vec<PriceId> = named.iter().filter_map(|n| n.price).collect();
    let product_ids: Vec<ProductId> = named.iter().filter_map(|n| n.product).collect();
    let prices: HashMap<PriceId, PriceRow> =
        db(PriceRow::list_by_ids(conn, &price_ids, tenant_id).await)?
            .into_iter()
            .map(|price| (price.id, price))
            .collect();
    let products: HashSet<ProductId> =
        db(ProductRow::list_by_ids(conn, &product_ids, tenant_id).await)?
            .into_iter()
            .map(|product| product.id)
            .collect();
    for n in named {
        if let Some(component) = n.component
            && !components.contains(&component)
        {
            return Ok(Some(Unresolvable::ComponentGone(component)));
        }
        if let Some(product) = n.product
            && !products.contains(&product)
        {
            return Ok(Some(Unresolvable::ProductGone(product)));
        }
        let Some(price_id) = n.price else {
            continue;
        };
        let Some(price) = prices.get(&price_id) else {
            return Ok(Some(Unresolvable::PriceGone(price_id)));
        };
        if let (Some(offered), Some(component)) = (offered, n.component)
            && !offered.contains(&(component, price_id))
        {
            return Ok(Some(Unresolvable::PriceGone(price_id)));
        }
        if price.currency != ctx.sub.currency {
            return Ok(Some(Unresolvable::OtherCurrency {
                subscription: ctx.sub.currency.clone(),
                target: price.currency.clone(),
            }));
        }
    }
    Ok(None)
}

fn seat_seed(fee: &SubscriptionFee, on: NaiveDate) -> Option<SeatSeed> {
    SlotTransactionNewInternal::from_fee(fee, on).map(|seed| SeatSeed {
        unit: seed.unit,
        slots: seed.prev_active_slots,
    })
}

/// What a count set for `at` holds beyond itself so the ledger reads it plus the seats of `unit`
/// changed since `scheduled_at` (now when `None`): those changed since and effective before `at`,
/// less those changed before and effective at `at`, which the ledger's reads add at `at` (R-108).
fn set_seats_carry(
    ledger: &[SlotTransactionRow],
    unit: &str,
    at: NaiveDateTime,
    scheduled_at: Option<NaiveDateTime>,
) -> i32 {
    ledger
        .iter()
        .filter(|row| row.unit == unit && row.delta != 0)
        .map(|row| {
            let since = scheduled_at.is_some_and(|scheduled| row.transaction_at > scheduled);
            match (since, row.effective_at.cmp(&at)) {
                (true, std::cmp::Ordering::Less) => row.delta,
                (false, std::cmp::Ordering::Equal) => -row.delta,
                _ => 0,
            }
        })
        .sum()
}

/// Seats `row` changed while a change setting its seats on a later day is pending carry into
/// that count, as the change's own reads would add them (R-108).
pub(crate) async fn carry_into_set_seats_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    row: &SlotTransactionRow,
) -> StoreResult<()> {
    if row.delta == 0 || row.status != diesel_models::enums::SlotTransactionStatusEnum::Active {
        return Ok(());
    }
    let mut seeds: Vec<SlotTransactionId> = Vec::new();
    for event in db(ScheduledEventRow::list_migrated_pending(
        conn,
        &tenant_id,
        row.subscription_id,
        &[diesel_models::enums::ScheduledEventTypeEnum::ApplyPlanChange],
    )
    .await)?
    {
        let Some(record) = event.written_facts else {
            continue;
        };
        let written: WrittenFacts = serde_json::from_value(record)
            .map_err(|error| Report::new(StoreError::SerdeError("written facts".into(), error)))?;
        seeds.extend(written.set_seats.into_iter().map(SlotTransactionId::from));
    }
    db(
        SlotTransactionRow::carry_into_seeds(conn, &seeds, &row.unit, row.effective_at, row.delta)
            .await,
    )?;
    Ok(())
}

/// What a change wrote as facts, kept on its MIGRATED record: withdrawn when the change is
/// cancelled before its date, and by the rollback script (ids as UUIDs, for SQL).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrittenFacts {
    pub on: NaiveDate,
    pub plan_window: Option<uuid::Uuid>,
    /// The open window `plan_window` replaced on its day, restored when the change is withdrawn.
    #[serde(default)]
    pub replaced_plan_window: Option<ReplacedWindow>,
    pub opened_components: Vec<uuid::Uuid>,
    pub closed_components: Vec<uuid::Uuid>,
    pub opened_add_ons: Vec<uuid::Uuid>,
    pub closed_add_ons: Vec<uuid::Uuid>,
    pub seat_seeds: Vec<uuid::Uuid>,
    /// Of `seat_seeds`, the counts set on a seat fee kept, into which seats changed before `on`
    /// carry.
    #[serde(default)]
    pub set_seats: Vec<uuid::Uuid>,
    /// The minimum it replaced or ended on `on`.
    pub minimum: bool,
    /// The trial's end before the change cut it.
    pub trial_until: Option<NaiveDate>,
    /// The pending trial end moved to `on`, and when it was due.
    pub end_trial: Option<(uuid::Uuid, NaiveDateTime)>,
}

/// A plan window a change's window replaced, as it was (its cadence as the database writes it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplacedWindow {
    pub id: uuid::Uuid,
    pub plan_version_id: uuid::Uuid,
    pub from_date: NaiveDate,
    pub cadence: String,
    pub cadence_origin: NaiveDate,
}

impl ReplacedWindow {
    fn of(row: &SubscriptionPlanWindowRow) -> Self {
        let cadence = match row.cadence {
            DbBillingPeriod::Monthly => "MONTHLY",
            DbBillingPeriod::Quarterly => "QUARTERLY",
            DbBillingPeriod::Semiannual => "SEMIANNUAL",
            DbBillingPeriod::Annual => "ANNUAL",
        };
        ReplacedWindow {
            id: row.id,
            plan_version_id: row.plan_version_id.as_uuid(),
            from_date: row.from_date,
            cadence: cadence.to_owned(),
            cadence_origin: row.cadence_origin,
        }
    }

    fn row(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> Option<SubscriptionPlanWindowRowNew> {
        let cadence = match self.cadence.as_str() {
            "MONTHLY" => DbBillingPeriod::Monthly,
            "QUARTERLY" => DbBillingPeriod::Quarterly,
            "SEMIANNUAL" => DbBillingPeriod::Semiannual,
            "ANNUAL" => DbBillingPeriod::Annual,
            _ => return None,
        };
        Some(SubscriptionPlanWindowRowNew {
            id: self.id,
            tenant_id,
            subscription_id,
            plan_version_id: self.plan_version_id.into(),
            from_date: self.from_date,
            cadence,
            cadence_origin: self.cadence_origin,
        })
    }
}

/// A plan window a change opened, and the one it replaced on its day.
pub(crate) struct OpenedWindow {
    pub id: uuid::Uuid,
    pub replaced: Option<ReplacedWindow>,
}

impl WrittenFacts {
    fn rows(&self, tenant_id: TenantId, subscription_id: SubscriptionId) -> ScheduledRows {
        ScheduledRows {
            plan_window: self.plan_window,
            replaced_plan_window: self
                .replaced_plan_window
                .as_ref()
                .and_then(|window| window.row(tenant_id, subscription_id)),
            opened_components: self
                .opened_components
                .iter()
                .map(|id| SubscriptionPriceComponentId::from(*id))
                .collect(),
            closed_components: self
                .closed_components
                .iter()
                .map(|id| SubscriptionPriceComponentId::from(*id))
                .collect(),
            opened_add_ons: self
                .opened_add_ons
                .iter()
                .map(|id| SubscriptionAddOnId::from(*id))
                .collect(),
            closed_add_ons: self
                .closed_add_ons
                .iter()
                .map(|id| SubscriptionAddOnId::from(*id))
                .collect(),
            seat_seeds: self
                .seat_seeds
                .iter()
                .map(|id| SlotTransactionId::from(*id))
                .collect(),
            minimum: self.minimum,
            trial_until: self.trial_until,
        }
    }
}

/// Writes a pending change as the facts it becomes and marks it `MIGRATED` with the rows it
/// wrote, under its customer's lock. A straggler an old replica claimed is migrated the same way,
/// unless it is left to a fresh claim; run again, it finds the event migrated and writes nothing.
pub(crate) async fn write_pending_as_facts_tx(
    conn: &mut PgConn,
    event: &ScheduledEvent,
    writing: Writing,
) -> StoreResult<Result<Migration, Unresolvable>> {
    let lock =
        db(SubscriptionRow::lock_subscription_for_update(conn, event.subscription_id).await)?;
    let current = load_event(conn, event.tenant_id, event.id).await?;
    if current.status == ScheduledEventStatus::Migrated {
        return Ok(Ok(Migration::AlreadyMigrated));
    }
    if writing
        == (Writing::Ahead {
            leave_fresh_claims: true,
        })
        && current.status == ScheduledEventStatus::Processing
        && current.updated_at
            > diesel_models::query::scheduled_events::stale_claims_before(
                common_clock::now().naive_utc(),
            )
    {
        return Ok(Ok(Migration::Claimed));
    }
    let facts = match resolve_pending_tx(conn, &current, writing).await? {
        Ok(facts) => facts,
        Err(why) => return Ok(Err(why)),
    };
    let written = write_facts(conn, &facts, &current).await?;
    let record = serde_json::to_value(&written)
        .map_err(|error| Report::new(StoreError::SerdeError("written facts".into(), error)))?;
    if !db(ScheduledEventRow::mark_as_migrated(
        conn,
        &current.tenant_id,
        &current.id,
        Some(record),
    )
    .await)?
    {
        return Err(Report::new(StoreError::FailedPrecondition(format!(
            "scheduled event {} left pending while it was migrated",
            current.id
        ))));
    }
    db(diesel_models::query::credit_inputs::bump_customer(conn, &lock).await)?;
    Ok(Ok(Migration::Written(Box::new(facts))))
}

async fn write_facts(
    conn: &mut PgConn,
    facts: &PendingFacts,
    event: &ScheduledEvent,
) -> StoreResult<WrittenFacts> {
    let on = facts.on;
    let mut written = WrittenFacts {
        on,
        closed_components: facts
            .closes_components
            .iter()
            .map(|id| id.as_uuid())
            .collect(),
        opened_components: facts
            .opens_components
            .iter()
            .map(|row| row.id.as_uuid())
            .collect(),
        closed_add_ons: facts.closes_add_ons.iter().map(|id| id.as_uuid()).collect(),
        opened_add_ons: facts
            .opens_add_ons
            .iter()
            .map(|row| row.id.as_uuid())
            .collect(),
        ..WrittenFacts::default()
    };
    super::close_components_tx(conn, &facts.closes_components, on).await?;
    if !facts.opens_components.is_empty() {
        db(
            SubscriptionComponentRow::insert_subscription_component_batch(
                conn,
                facts.opens_components.iter().collect(),
            )
            .await,
        )?;
    }
    super::close_add_ons_tx(conn, &facts.closes_add_ons, on).await?;
    if !facts.opens_add_ons.is_empty() {
        db(SubscriptionAddOnRow::insert_batch(conn, facts.opens_add_ons.iter().collect()).await)?;
    }
    for seed in &facts.seat_seeds {
        let at = on.and_time(NaiveTime::MIN);
        let seed = SlotTransactionNewInternal {
            id: SlotTransactionId::new(),
            unit: seed.unit.clone(),
            delta: 0,
            prev_active_slots: seed.slots,
            effective_at: at,
            transaction_at: at,
        };
        written.seat_seeds.push(seed.id.as_uuid());
        db(seed.into_row(facts.subscription_id).insert(conn).await)?;
    }
    if !facts.set_seats.is_empty() {
        let ledger = db(SlotTransactionRow::list_by_subscription_id(
            conn,
            facts.tenant_id,
            facts.subscription_id,
            None,
            Some(diesel_models::enums::SlotTransactionStatusEnum::Active),
        )
        .await)?;
        let at = on.and_time(NaiveTime::MIN);
        for set in &facts.set_seats {
            let seed = SlotTransactionNewInternal {
                id: SlotTransactionId::new(),
                unit: set.unit.clone(),
                delta: 0,
                prev_active_slots: set.slots
                    + set_seats_carry(&ledger, &set.unit, at, facts.scheduled_at),
                effective_at: at,
                transaction_at: at,
            };
            written.seat_seeds.push(seed.id.as_uuid());
            written.set_seats.push(seed.id.as_uuid());
            db(seed.into_row(facts.subscription_id).insert(conn).await)?;
        }
    }
    let Some(window) = &facts.plan_window else {
        if let Some(opened) =
            record_amendment_cadence_tx(conn, facts.tenant_id, facts.subscription_id, on).await?
        {
            written.plan_window = Some(opened.id);
            written.replaced_plan_window = opened.replaced;
        }
        return Ok(written);
    };
    crate::services::subscriptions::plan_change::refresh_subscription_minimum_on_plan_change(
        conn,
        facts.subscription_id,
        window.plan_version_id,
        on,
    )
    .await?;
    written.minimum = facts.minimum != MinimumChange::Kept;
    if window.trial_until.is_some()
        && let Some(trial) = db(SubscriptionTrialRow::find(conn, facts.subscription_id).await)?
    {
        written.trial_until = Some(trial.until_date);
        db(SubscriptionTrialRow::end_by(conn, facts.subscription_id, on).await)?;
        // The status is still the trial end event's: it moves to the change.
        if !trial.free {
            written.end_trial = db(ScheduledEventRow::move_pending_end_trial(
                conn,
                &facts.tenant_id,
                facts.subscription_id,
                on.and_time(NaiveTime::MIN),
            )
            .await)?
            .map(|(id, at)| (id.as_uuid(), at));
        }
    }
    let (opened, replaced) = db(SubscriptionPlanWindowRow::open_replacing(
        conn,
        SubscriptionPlanWindowRowNew {
            id: uuid::Uuid::now_v7(),
            tenant_id: event.tenant_id,
            subscription_id: facts.subscription_id,
            plan_version_id: window.plan_version_id,
            from_date: window.from_date,
            cadence: window.cadence.into(),
            cadence_origin: window.cadence_origin,
        },
    )
    .await)?;
    written.plan_window = Some(opened.id);
    written.replaced_plan_window = replaced.as_ref().map(ReplacedWindow::of);
    Ok(written)
}

/// An amendment moving the subscription onto the shortest cadence of the fees it bills from
/// `on`, as if created with them: the plan window of that cadence from then, its grid kept,
/// unless a plan switch moved it onto a grid of its own. Returns the window opened.
pub(crate) async fn record_amendment_cadence_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    on: NaiveDate,
) -> StoreResult<Option<OpenedWindow>> {
    let Some(window) =
        amendment_cadence_window_tx(conn, tenant_id, subscription_id, on, None).await?
    else {
        return Ok(None);
    };
    let (opened, replaced) = db(SubscriptionPlanWindowRow::open_replacing(
        conn,
        SubscriptionPlanWindowRowNew {
            id: uuid::Uuid::now_v7(),
            tenant_id,
            subscription_id,
            plan_version_id: window.plan_version_id,
            from_date: window.from_date,
            cadence: window.cadence.into(),
            cadence_origin: window.cadence_origin,
        },
    )
    .await)?;
    Ok(Some(OpenedWindow {
        id: opened.id,
        replaced: replaced.as_ref().map(ReplacedWindow::of),
    }))
}

/// The window [`record_amendment_cadence_tx`] opens for an amendment on `on`, read and never
/// written; `change`, the amendment's rows not yet written, read as if they were.
pub(crate) async fn amendment_cadence_window_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    on: NaiveDate,
    change: Option<&PendingFacts>,
) -> StoreResult<Option<PlanWindowFact>> {
    use crate::domain::enums::SubscriptionFeeBillingPeriod;
    use crate::services::invoice_lines::cadence::{CadenceSubject, cadence_runs_of};

    let sub = db(SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id).await)?
        .subscription;
    if sub.status != diesel_models::enums::SubscriptionStatusEnum::Active
        || sub.current_period_end.is_none()
    {
        return Ok(None);
    }
    let in_force =
        |from: NaiveDate, to: Option<NaiveDate>| from <= on && to.is_none_or(|to| on < to);
    let closes = |closed: bool, to: Option<NaiveDate>| {
        if closed && to.is_none() { Some(on) } else { to }
    };
    let cadences: Vec<SubscriptionFeeBillingPeriod> =
        db(SubscriptionComponentRow::list_fact_rows(conn, sub.id).await)?
            .into_iter()
            .filter(|row| {
                let closed = change.is_some_and(|c| c.closes_components.contains(&row.id));
                in_force(row.effective_from, closes(closed, row.effective_to))
            })
            .map(|row| row.period.into())
            .chain(
                db(SubscriptionAddOnRow::list_fact_rows(conn, sub.id).await)?
                    .into_iter()
                    .filter(|row| {
                        let closed = change.is_some_and(|c| c.closes_add_ons.contains(&row.id));
                        in_force(row.effective_from, closes(closed, row.effective_to))
                    })
                    .map(|row| row.period.into()),
            )
            .chain(change.into_iter().flat_map(|c| {
                c.opens_components
                    .iter()
                    .filter(|row| in_force(row.effective_from, None))
                    .map(|row| row.period.clone().into())
                    .chain(
                        c.opens_add_ons
                            .iter()
                            .filter(|row| in_force(row.effective_from, None))
                            .map(|row| row.period.clone().into()),
                    )
                    .collect::<Vec<SubscriptionFeeBillingPeriod>>()
            }))
            .collect();
    let Some(cadence) = crate::services::subscriptions::cadence::realigned_cadence(
        &BillingPeriodEnum::from(sub.period.clone()),
        sub.billing_start_date.unwrap_or(sub.current_period_start),
        u32::from(sub.billing_day_anchor.unsigned_abs()),
        &cadences,
        on,
    ) else {
        return Ok(None);
    };
    // A plan switch moved it onto a grid of its own.
    if !cadence_runs_of(conn, CadenceSubject::of_row(&sub), on, true)
        .await?
        .is_empty()
    {
        return Ok(None);
    }
    let Some(open) = db(SubscriptionPlanWindowRow::open_of(conn, sub.id).await)? else {
        return Ok(None);
    };
    Ok(Some(PlanWindowFact {
        plan_version_id: open.plan_version_id,
        from_date: on,
        cadence: cadence.period,
        cadence_origin: open.cadence_origin,
        trial_until: None,
    }))
}

/// Withdraws a change written as facts before its date: the rows it opened go, those it closed
/// reopen, a trial end it moved goes back. False when it took effect or was withdrawn already.
pub(crate) async fn withdraw_pending_tx(
    conn: &mut PgConn,
    event: &ScheduledEventRow,
    reason: &str,
) -> StoreResult<bool> {
    let Some(record) = event.written_facts.clone() else {
        return Ok(false);
    };
    let written: WrittenFacts = serde_json::from_value(record)
        .map_err(|error| Report::new(StoreError::SerdeError("written facts".into(), error)))?;
    if !db(ScheduledEventRow::withdraw_migrated(conn, &event.tenant_id, &event.id, reason).await)? {
        return Ok(false);
    }
    db(written
        .rows(event.tenant_id, event.subscription_id)
        .withdraw(conn, event.subscription_id, written.on)
        .await)?;
    if let Some((id, at)) = written.end_trial {
        db(ScheduledEventRow::reschedule_pending(
            conn,
            &event.tenant_id,
            ScheduledEventId::from(id),
            at,
        )
        .await)?;
    }
    Ok(true)
}

/// Every kind of change written as facts.
pub(crate) const CHANGES: &[diesel_models::enums::ScheduledEventTypeEnum] = &[
    diesel_models::enums::ScheduledEventTypeEnum::ApplyPlanChange,
    diesel_models::enums::ScheduledEventTypeEnum::ApplyAmendment,
];

/// Withdraws the subscription's changes of `types` written as facts and still to take effect;
/// returns how many.
pub(crate) async fn withdraw_pending_of_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    types: &[diesel_models::enums::ScheduledEventTypeEnum],
    reason: &str,
) -> StoreResult<usize> {
    let mut withdrawn = 0;
    for event in
        db(
            ScheduledEventRow::list_migrated_pending(conn, &tenant_id, subscription_id, types)
                .await,
        )?
    {
        if withdraw_pending_tx(conn, &event, reason).await? {
            withdrawn += 1;
        }
    }
    Ok(withdrawn)
}

/// A change requested now for a later date, written as facts at once: its event is recorded
/// MIGRATED with the rows it wrote, so it stays restorable and cancellable.
pub(crate) async fn schedule_as_facts_tx(
    store: &crate::Store,
    conn: &mut PgConn,
    change: crate::domain::scheduled_events::ScheduledEventNew,
) -> StoreResult<ScheduledEvent> {
    use crate::repositories::SubscriptionInterface;

    let event = store
        .schedule_events(conn, vec![change])
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| Report::new(StoreError::InsertError))?;
    if let Err(why) = write_pending_as_facts_tx(
        conn,
        &event,
        Writing::Ahead {
            leave_fresh_claims: false,
        },
    )
    .await?
    {
        return Err(Report::new(StoreError::InvalidArgument(format!(
            "the change cannot be scheduled: {why:?}"
        ))));
    }
    load_event(conn, event.tenant_id, event.id).await
}

async fn load_event(
    conn: &mut PgConn,
    tenant_id: TenantId,
    event_id: ScheduledEventId,
) -> StoreResult<ScheduledEvent> {
    ScheduledEvent::try_from(db(ScheduledEventRow::get_by_id(
        conn, event_id, &tenant_id,
    )
    .await)?)
}

/// What the deploy-time migration of pending changes did.
#[derive(Debug, Default, Clone)]
pub struct PendingMigration {
    pub written: usize,
    pub already: usize,
    /// Claimed by a replica since they were listed: left to it.
    pub claimed: usize,
    /// Each change left pending, and why.
    pub unresolved: Vec<(ScheduledEventId, Unresolvable)>,
}

impl ServicesEdge {
    /// The facts a pending scheduled change becomes, written by nothing (the migration's dry run).
    pub async fn resolve_pending_event(
        &self,
        tenant_id: TenantId,
        event_id: ScheduledEventId,
    ) -> StoreResult<Resolution> {
        let mut conn = self.store.get_conn().await?;
        let event = load_event(&mut conn, tenant_id, event_id).await?;
        resolve_pending_tx(
            &mut conn,
            &event,
            Writing::Ahead {
                leave_fresh_claims: false,
            },
        )
        .await
    }

    /// Writes a pending scheduled change as facts and marks it migrated, then derives the state.
    pub async fn write_pending_event_as_facts(
        &self,
        tenant_id: TenantId,
        event_id: ScheduledEventId,
    ) -> StoreResult<Result<Migration, Unresolvable>> {
        self.services
            .write_pending_event_as_facts(tenant_id, event_id)
            .await
    }

    /// Derives the subscription's state from its facts at `at`, under its customer's lock, as the
    /// controller does at its wake; the step it took.
    pub async fn derive_subscription_state(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        at: chrono::DateTime<chrono::Utc>,
    ) -> StoreResult<Option<billing_core::transition::Step>> {
        let services = &self.services;
        self.store
            .transaction(|conn| {
                async move {
                    let lock = db(SubscriptionRow::lock_customer_of(conn, subscription_id).await)?;
                    let step = services
                        .derive_state_tx(
                            conn,
                            &common_domain::actor::Actor::System,
                            tenant_id,
                            subscription_id,
                            at,
                        )
                        .await?;
                    db(diesel_models::query::credit_inputs::bump_customer(conn, &lock).await)?;
                    Ok(step)
                }
                .scope_boxed()
            })
            .await
    }

    /// The deploy-time migration, once check 20 of `pre-release-checks.sql` reads 0: see
    /// [`crate::services::Services::migrate_pending_changes`].
    pub async fn migrate_pending_changes(&self) -> StoreResult<Option<PendingMigration>> {
        self.services.migrate_pending_changes().await
    }
}

impl crate::services::Services {
    /// Writes a pending scheduled change as facts and marks it migrated, then derives the state.
    pub async fn write_pending_event_as_facts(
        &self,
        tenant_id: TenantId,
        event_id: ScheduledEventId,
    ) -> StoreResult<Result<Migration, Unresolvable>> {
        self.write_event_as_facts(
            tenant_id,
            event_id,
            Writing::Ahead {
                leave_fresh_claims: false,
            },
        )
        .await
    }

    async fn write_event_as_facts(
        &self,
        tenant_id: TenantId,
        event_id: ScheduledEventId,
        writing: Writing,
    ) -> StoreResult<Result<Migration, Unresolvable>> {
        self.store
            .transaction(|conn| {
                async move {
                    let event = load_event(conn, tenant_id, event_id).await?;
                    let migrated = write_pending_as_facts_tx(conn, &event, writing).await?;
                    if let Ok(Migration::Written(_)) = migrated {
                        self.derive_state_tx(
                            conn,
                            &common_domain::actor::Actor::System,
                            tenant_id,
                            event.subscription_id,
                            common_clock::now(),
                        )
                        .await?;
                    }
                    Ok(migrated)
                }
                .scope_boxed()
            })
            .await
    }

    /// Every pending plan change and amendment written as facts, each in its own transaction; one
    /// not written stays pending and is listed. The billing worker runs it as it starts, on one
    /// replica at a time: None when another holds it.
    pub async fn migrate_pending_changes(&self) -> StoreResult<Option<PendingMigration>> {
        use crate::leader_lock::{LeaderLock, PgLeaderElection};
        use distributed_lock::LeaderElection;

        let election =
            PgLeaderElection::new(self.store.pool.clone(), LeaderLock::PendingChangesMigration);
        let guard = election.try_acquire().await.map_err(|error| {
            Report::new(StoreError::InitializationError(format!(
                "the pending changes migration's lock: {error:?}"
            )))
        })?;
        let Some(guard) = guard else {
            return Ok(None);
        };
        let migration = self.migrate_listed_changes().await;
        guard.release().await;
        migration.map(Some)
    }

    async fn migrate_listed_changes(&self) -> StoreResult<PendingMigration> {
        let pending = {
            let mut conn = self.store.get_conn().await?;
            db(ScheduledEventRow::list_pending_changes(&mut conn).await)?
        };
        let mut migration = PendingMigration::default();
        for (tenant_id, event_id) in pending {
            let why = match self
                .write_event_as_facts(
                    tenant_id,
                    event_id,
                    Writing::Ahead {
                        leave_fresh_claims: true,
                    },
                )
                .await
            {
                Ok(Ok(Migration::Written(_))) => {
                    migration.written += 1;
                    continue;
                }
                Ok(Ok(Migration::AlreadyMigrated)) => {
                    migration.already += 1;
                    continue;
                }
                Ok(Ok(Migration::Claimed)) => {
                    migration.claimed += 1;
                    continue;
                }
                Ok(Err(why)) => why,
                Err(error) => {
                    log::error!(
                        "pending change {event_id} failed to be written as facts: {error:?}"
                    );
                    Unresolvable::Failed(format!("{error:?}"))
                }
            };
            log::warn!("pending change {event_id} not migrated: {why:?}");
            migration.unresolved.push((event_id, why));
        }
        Ok(migration)
    }
}
