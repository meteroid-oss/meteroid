//! Scheduled changes as the facts they become: a plan change resolved against its target
//! version, an amendment written as it is. A change is written when it is scheduled, recorded
//! with the rows it wrote, and withdrawn by removing them before its date.
use std::collections::{HashMap, HashSet};

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use common_domain::ids::{
    AddOnId, BaseId, ManagedQuantityId, PlanVersionId, PriceComponentId, PriceId, ProductId,
    ScheduledEventId, SlotTransactionId, SubscriptionAddOnId, SubscriptionId,
    SubscriptionPriceComponentId, TenantId,
};
use diesel_models::enums::BillingPeriodEnum as DbBillingPeriod;
use diesel_models::enums::SubscriptionChangeKindEnum;
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
use diesel_models::slot_transactions::SlotTransactionRow;
use diesel_models::subscription_add_ons::{SubscriptionAddOnRow, SubscriptionAddOnRowNew};
use diesel_models::subscription_adjustments::SubscriptionAdjustmentRow;
use diesel_models::subscription_changes::SubscriptionChangeRow;
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
use crate::domain::enums::BillingPeriodEnum;
use crate::domain::scheduled_events::{
    ComponentMapping, ResolvedAddOnInsert, ResolvedComponentInsert, ScheduledEvent,
    ScheduledEventData, ScheduledEventNew,
};
use crate::domain::slot_transactions::SlotTransactionNewInternal;
use crate::domain::subscription_add_ons::{SubscriptionAddOnNew, SubscriptionAddOnNewInternal};
use crate::domain::subscription_components::{
    SubscriptionComponentNew, SubscriptionComponentNewInternal, SubscriptionFee,
};
use crate::domain::subscription_phases::PhaseMinimum;
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
    /// The quantity the seats count; None for a fee recorded before quantities were named,
    /// counting the quantity coded as `unit`.
    pub quantity_id: Option<ManagedQuantityId>,
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

/// Why a change cannot be written as facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unresolvable {
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
}

pub type Resolution = Result<PendingFacts, Unresolvable>;

struct Context<'a> {
    event: &'a ScheduledEvent,
    /// Due already, as an earlier release stored it: each price taken as named, offered or not.
    as_stored: bool,
    /// A timeline written again: an ending still to come cuts it only when it happens (R-162).
    through_ending: bool,
    sub: &'a SubscriptionRow,
    on: NaiveDate,
    open_components: HashSet<SubscriptionPriceComponentId>,
    open_add_ons: Vec<(SubscriptionAddOnId, AddOnId)>,
    /// The quantities the open seat fees count: a new seat fee of one carries its seats.
    held_quantities: HashSet<ManagedQuantityId>,
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
            scheduled_at: self.as_stored.then_some(self.event.created_at),
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

/// The facts a change becomes, read and never written.
pub(crate) async fn resolve_change_tx(
    conn: &mut PgConn,
    event: &ScheduledEvent,
) -> StoreResult<Resolution> {
    resolve_change_with(conn, event, false, false).await
}

async fn resolve_change_with(
    conn: &mut PgConn,
    event: &ScheduledEvent,
    as_stored: bool,
    through_ending: bool,
) -> StoreResult<Resolution> {
    let sub =
        db(
            SubscriptionRow::get_subscription_by_id(conn, &event.tenant_id, event.subscription_id)
                .await,
        )?
        .subscription;
    let component_rows: Vec<SubscriptionComponentRow> =
        db(SubscriptionComponentRow::list_fact_rows(conn, sub.id).await)?
            .into_iter()
            .filter(|row| row.effective_to.is_none())
            .collect();
    let add_on_rows: Vec<SubscriptionAddOnRow> =
        db(SubscriptionAddOnRow::list_fact_rows(conn, sub.id).await)?
            .into_iter()
            .filter(|row| row.effective_to.is_none())
            .collect();
    let held_quantities =
        held_seat_quantities(conn, event.tenant_id, &component_rows, &add_on_rows).await?;
    let open_components = component_rows.iter().map(|row| row.id).collect();
    let open_add_ons = add_on_rows
        .iter()
        .map(|row| (row.id, row.add_on_id))
        .collect();
    let ctx = Context {
        event,
        as_stored,
        through_ending,
        sub: &sub,
        on: event.scheduled_time.date(),
        open_components,
        open_add_ons,
        held_quantities,
    };
    match &event.event_data {
        ScheduledEventData::ApplyPlanChange {
            source_plan_version_id,
            new_plan_version_id,
            component_mappings,
            addon_close,
            addon_insert,
            ..
        } => {
            let resolution = resolve_plan_change(
                conn,
                &ctx,
                *source_plan_version_id,
                *new_plan_version_id,
                component_mappings,
            )
            .await?;
            let Ok(mut facts) = resolution else {
                return Ok(resolution);
            };
            if let Some(row) = addon_close
                .iter()
                .find(|row| !ctx.open_add_ons.iter().any(|(open, _)| open == *row))
            {
                return Ok(Err(Unresolvable::AddOnRowNotOpen(*row)));
            }
            facts.closes_add_ons.extend(addon_close.iter().copied());
            for a in addon_insert {
                facts
                    .opens_add_ons
                    .push(floored_add_on_row(ctx.sub, ctx.on, a)?);
            }
            Ok(Ok(facts))
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
        ScheduledEventData::ApplyPhase {
            component_close,
            component_insert,
            addon_close,
            addon_insert,
            minimum,
            ..
        } => {
            let resolution = resolve_amendment(
                conn,
                &ctx,
                component_close,
                component_insert,
                addon_close,
                addon_insert,
            )
            .await?;
            Ok(resolution.map(|mut facts| {
                // Ids derived from their owner: writing the same terms again writes the same rows.
                for (row, insert) in facts.opens_components.iter_mut().zip(component_insert) {
                    if let Some(id) = insert.subscription_component_id {
                        row.id = id;
                    }
                }
                for (row, insert) in facts.opens_add_ons.iter_mut().zip(addon_insert) {
                    if let Some(id) = insert.subscription_add_on_id {
                        row.id = id;
                    }
                }
                facts.minimum = match minimum {
                    PhaseMinimum::Kept => MinimumChange::Kept,
                    PhaseMinimum::Ended => MinimumChange::Ended,
                    PhaseMinimum::Amount {
                        amount_subunit,
                        products,
                    } => MinimumChange::Replaced {
                        amount_subunit: *amount_subunit,
                        includes_all_components: products.is_empty(),
                        products: products.clone(),
                    },
                };
                facts
            }))
        }
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
    if let Some(why) = not_live_on(conn, sub, on, ctx.through_ending).await? {
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
    let offered = (!ctx.as_stored).then_some(&offered);
    if let Some(why) = catalog_gone(conn, ctx, &named, &components, offered).await? {
        return Ok(Err(why));
    }

    let mut facts = ctx.facts();
    // The rows the plan change writes when applied, ids aside.
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
                priced,
            } => {
                facts.closes_components.push(*current_component_id);
                lay_target_seats(&mut facts, &ctx.held_quantities, fee, *seats, true, on);
                SubscriptionComponentNewInternal {
                    price_component_id: Some(*target_component_id),
                    product_id: Some(*product_id),
                    name: name.clone(),
                    period: *period,
                    fee: fee.clone(),
                    is_override: *priced,
                    price_id: (!priced).then_some(*price_id),
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
                seats,
            } => {
                lay_target_seats(&mut facts, &ctx.held_quantities, fee, *seats, false, on);
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
    facts.plan_window = Some(
        plan_window(
            conn,
            sub,
            on,
            new_plan_version_id,
            mappings,
            &facts.closes_add_ons,
        )
        .await?,
    );
    Ok(Ok(facts))
}

/// The window `record_plan_change_tx` opens for a change on `on`, read before it applies.
async fn plan_window(
    conn: &mut PgConn,
    sub: &SubscriptionRow,
    on: NaiveDate,
    plan_version_id: PlanVersionId,
    mappings: &[ComponentMapping],
    closes_add_ons: &[SubscriptionAddOnId],
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
    let as_created = matches!((trial.as_ref(), trial_until), (Some(t), Some(_)) if t.free)
        || on == super::billing_from(conn, sub).await?;
    let (windows, from) = super::windows_before(conn, sub.id, on).await?;
    let cadence_origin = match (trial.as_ref(), trial_until) {
        (Some(t), Some(until)) if t.free => {
            origin_after_free_trial(conn, sub, &windows, from, &db_cadence, until).await?
        }
        _ => super::decided_origin(conn, sub, &windows, from, &db_cadence).await?,
    };
    let cadence =
        super::cadence_of_change(conn, sub.id, on, db_cadence, as_created, closes_add_ons)
            .await?
            .into();
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
    if let Some(why) = not_live_on(conn, sub, on, ctx.through_ending).await? {
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
    // An override names a component of the plan version it runs on: the one in force on its
    // day, a switch written ahead of it included (R-150).
    let plan_version_id = db(SubscriptionPlanWindowRow::list_by_subscription(conn, sub.id).await)?
        .into_iter()
        .rfind(|window| window.from_date <= on)
        .map_or(sub.plan_version_id, |window| window.plan_version_id);
    let components: HashSet<PriceComponentId> =
        db(
            PriceComponentRow::list_by_plan_version_id(conn, ctx.event.tenant_id, plan_version_id)
                .await,
        )?
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
    // The rows the amendment writes when applied, ids aside.
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
        // A seat fee of a quantity held binds to its count (R-137): no seed resets it.
        if !c.is_override && !holds(&ctx.held_quantities, &c.fee) {
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
        if a.lineage_id.is_none() && !holds(&ctx.held_quantities, &a.fee) {
            facts.seat_seeds.extend(seat_seed(&a.fee, on));
        }
    }
    Ok(Ok(facts))
}

/// The add-on row a phase's switch opens to floor a seat add-on (R-146): its id derived from the
/// phase, its lineage kept.
fn floored_add_on_row(
    sub: &SubscriptionRow,
    on: NaiveDate,
    a: &ResolvedAddOnInsert,
) -> StoreResult<SubscriptionAddOnRowNew> {
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
    if let Some(id) = a.subscription_add_on_id {
        row.id = id;
    }
    Ok(row)
}

/// Why the subscription will not take a change on `on`: not running, or ended by then (an ending
/// still to come left out with `through_ending`).
async fn not_live_on(
    conn: &mut PgConn,
    sub: &SubscriptionRow,
    on: NaiveDate,
    through_ending: bool,
) -> StoreResult<Option<Unresolvable>> {
    let now = common_clock::now().naive_utc();
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
        .filter(|at| at.date() <= on && (!through_ending || *at <= now))
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
        quantity_id: seed.quantity_id,
        slots: seed.prev_active_slots,
    })
}

/// Whether `fee` counts seats of a quantity the subscription holds.
fn holds(held: &HashSet<ManagedQuantityId>, fee: &SubscriptionFee) -> bool {
    fee.seat_quantity().is_some_and(|q| held.contains(&q))
}

/// The seats a plan change's target fee starts from: a quantity the subscription holds carries
/// its count, whatever charge or product counted it, unless the change sets one (R-108, R-137);
/// another starts at the fee's initial count. A fee scheduled before quantities were named
/// (none on it) is read as then: a matched fee carries, an added one starts anew.
fn lay_target_seats(
    facts: &mut PendingFacts,
    held: &HashSet<ManagedQuantityId>,
    fee: &SubscriptionFee,
    sets: Option<u32>,
    matched: bool,
    on: NaiveDate,
) {
    let Some(mut seed) = seat_seed(fee, on) else {
        return;
    };
    let carried = match fee.seat_quantity() {
        Some(quantity) => held.contains(&quantity),
        None => matched,
    };
    match (carried, sets) {
        (true, Some(slots)) => {
            seed.slots = i32::try_from(slots).unwrap_or(i32::MAX);
            facts.set_seats.push(seed);
        }
        (true, None) => {}
        // A quantity not held opens at what the change sets, else the fee's initial count, as
        // `plan_change_seat_seed` writes it.
        (false, Some(slots)) => {
            seed.slots = i32::try_from(slots).unwrap_or(i32::MAX);
            facts.seat_seeds.push(seed);
        }
        (false, None) => facts.seat_seeds.push(seed),
    }
}

/// The quantities the open seat fees of `components` and `add_ons` count, as stored.
pub(crate) async fn held_seat_quantities(
    conn: &mut PgConn,
    tenant_id: TenantId,
    components: &[SubscriptionComponentRow],
    add_ons: &[SubscriptionAddOnRow],
) -> StoreResult<HashSet<ManagedQuantityId>> {
    let mut fees: Vec<SubscriptionFee> = components
        .iter()
        .filter_map(|row| row.legacy_fee.clone())
        .chain(add_ons.iter().filter_map(|row| row.legacy_fee.clone()))
        .filter_map(|json| serde_json::from_value::<SubscriptionFee>(json).ok())
        .filter(|fee| matches!(fee, SubscriptionFee::Slot { .. }))
        .collect();
    crate::repositories::subscriptions::slots::name_seat_quantities(
        conn,
        tenant_id,
        fees.iter_mut().collect(),
    )
    .await?;
    Ok(fees
        .iter()
        .filter_map(SubscriptionFee::seat_quantity)
        .collect())
}

/// What a count set for `at` holds beyond itself so the ledger reads it plus the seats of its
/// quantity changed since `scheduled_at` (now when `None`): those changed since and effective
/// before `at`, less those changed before and effective at `at`, which the ledger's reads add at
/// `at` (R-108).
fn set_seats_carry(
    ledger: &[SlotTransactionRow],
    seed: &SeatSeed,
    at: NaiveDateTime,
    scheduled_at: Option<NaiveDateTime>,
) -> i32 {
    ledger
        .iter()
        .filter(|row| {
            row.delta != 0
                && match seed.quantity_id {
                    Some(quantity) => row.managed_quantity_id == quantity,
                    None => row.unit == seed.unit,
                }
        })
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
    for change in db(SubscriptionChangeRow::list_pending(
        conn,
        &tenant_id,
        row.subscription_id,
        &[SubscriptionChangeKindEnum::PlanChange],
    )
    .await)?
    {
        let Some(record) = change.written_facts else {
            continue;
        };
        let written: WrittenFacts = serde_json::from_value(record)
            .map_err(|error| Report::new(StoreError::SerdeError("written facts".into(), error)))?;
        seeds.extend(written.set_seats.into_iter().map(SlotTransactionId::from));
    }
    db(SlotTransactionRow::carry_into_seeds(
        conn,
        &seeds,
        row.managed_quantity_id,
        row.effective_at,
        row.delta,
    )
    .await)?;
    Ok(())
}

/// What a change wrote as facts, kept on its record: withdrawn when the change is cancelled
/// before its date (ids as UUIDs, for SQL).
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

/// A minimum of a change keeping the plan (a phase's): the one in force ends on `on`, and any
/// written from then on goes, before the change's own (R-146).
async fn write_dated_minimum_tx(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    on: NaiveDate,
    minimum: &MinimumChange,
) -> StoreResult<()> {
    use diesel_models::subscription_adjustments::{
        SubscriptionAdjustmentComponentRow, SubscriptionAdjustmentComponentRowNew,
        SubscriptionAdjustmentRow, SubscriptionAdjustmentRowNew,
    };
    db(SubscriptionAdjustmentRow::end_current(conn, &subscription_id, on).await)?;
    let MinimumChange::Replaced {
        amount_subunit,
        includes_all_components,
        products,
    } = minimum
    else {
        return Ok(());
    };
    db(SubscriptionAdjustmentRowNew {
        subscription_id,
        kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
        amount_subunit: *amount_subunit,
        includes_all_components: *includes_all_components,
        is_override: true,
        effective_from: on,
    }
    .insert(conn)
    .await)?;
    if !includes_all_components {
        let scope: Vec<SubscriptionAdjustmentComponentRowNew> = products
            .iter()
            .map(|product_id| SubscriptionAdjustmentComponentRowNew {
                subscription_id,
                kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
                product_id: *product_id,
                effective_from: on,
            })
            .collect();
        db(SubscriptionAdjustmentComponentRow::insert_batch(conn, &scope).await)?;
    }
    Ok(())
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
            quantity_id: seed.quantity_id,
            delta: 0,
            prev_active_slots: seed.slots,
            effective_at: at,
            transaction_at: at,
        };
        written.seat_seeds.push(seed.id.as_uuid());
        let rows = crate::repositories::subscriptions::slots::ledger_rows(
            conn,
            facts.tenant_id,
            [&(facts.subscription_id, seed)],
        )
        .await?;
        db(rows[0].insert(conn).await)?;
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
                quantity_id: set.quantity_id,
                delta: 0,
                prev_active_slots: set.slots
                    + set_seats_carry(&ledger, set, at, facts.scheduled_at),
                effective_at: at,
                transaction_at: at,
            };
            written.seat_seeds.push(seed.id.as_uuid());
            written.set_seats.push(seed.id.as_uuid());
            let rows = crate::repositories::subscriptions::slots::ledger_rows(
                conn,
                facts.tenant_id,
                [&(facts.subscription_id, seed)],
            )
            .await?;
            db(rows[0].insert(conn).await)?;
        }
    }
    let Some(window) = &facts.plan_window else {
        if facts.minimum != MinimumChange::Kept {
            write_dated_minimum_tx(conn, facts.subscription_id, on, &facts.minimum).await?;
            written.minimum = true;
        }
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
    use crate::services::invoice_lines::cadence::CadenceSubject;

    use diesel_models::enums::SubscriptionStatusEnum;
    let sub = db(SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id).await)?
        .subscription;
    // A change dated on or after the trial's end finds the subscription active; one inside a
    // free trial moves the grid its billing starts on.
    let trialing = sub.status == SubscriptionStatusEnum::TrialActive;
    let trial = db(SubscriptionTrialRow::find(conn, sub.id).await)?;
    let trial_over_by = trialing && trial.as_ref().is_some_and(|trial| trial.until_date <= on);
    let in_free_trial = trialing && trial.is_some_and(|trial| trial.free && on < trial.until_date);
    if (sub.status != SubscriptionStatusEnum::Active && !trial_over_by && !in_free_trial)
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
    if crate::services::subscriptions::cadence::switched_onto_own_grid(
        conn,
        CadenceSubject::of_row(&sub),
        on,
    )
    .await?
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

/// Withdraws a change before its date: the rows it opened go, those it closed reopen. False when
/// it took effect or was withdrawn already.
pub(crate) async fn withdraw_pending_tx(
    conn: &mut PgConn,
    change: &SubscriptionChangeRow,
    reason: &str,
) -> StoreResult<bool> {
    let Some(record) = change.written_facts.clone() else {
        return Ok(false);
    };
    let written: WrittenFacts = serde_json::from_value(record)
        .map_err(|error| Report::new(StoreError::SerdeError("written facts".into(), error)))?;
    if !db(SubscriptionChangeRow::withdraw(conn, &change.id, reason).await)? {
        return Ok(false);
    }
    if change.kind == SubscriptionChangeKindEnum::Phase {
        db(SubscriptionChangeRow::drop_phase_settlement_due(
            conn,
            change.subscription_id,
            change.effective_at.date(),
        )
        .await)?;
    }
    db(ScheduledRows::withdraw(
        &written.rows(change.tenant_id, change.subscription_id),
        conn,
        change.subscription_id,
        written.on,
    )
    .await)?;
    Ok(true)
}

/// An amendment scheduled for its period's end that a later change left inside a period (a
/// cadence lengthened by an immediate amendment): moved to the subscription's first renewal on or
/// after its day, or withdrawn when the subscription ends first (R-106).
pub(crate) async fn defer_to_renewal_tx(
    conn: &mut PgConn,
    change: &SubscriptionChangeRow,
    facts: &billing_core::facts::Facts<crate::services::evaluate::pricer::StoreFee>,
    now: chrono::DateTime<chrono::Utc>,
) -> StoreResult<()> {
    let Some(record) = change.written_facts.clone() else {
        return Ok(());
    };
    let mut written: WrittenFacts = serde_json::from_value(record)
        .map_err(|error| Report::new(StoreError::SerdeError("written facts".into(), error)))?;
    let on = written.on;
    // Its own plan window would make its day a renewal: read without it.
    let mut without = facts.clone();
    if written.plan_window.is_some()
        && let Some(i) = without.plan_windows.iter().position(|w| w.from == on)
    {
        let window = without.plan_windows.remove(i);
        if let Some(replaced) = &written.replaced_plan_window
            && let Some(row) = replaced.row(change.tenant_id, change.subscription_id)
        {
            without.plan_windows.push(billing_core::facts::PlanWindow {
                plan_version: row.plan_version_id,
                from: row.from_date,
                to: window.to,
                cadence: BillingPeriodEnum::from(row.cadence).as_months(),
                cadence_origin: row.cadence_origin,
            });
        }
    }
    match billing_core::evaluate::renewal_on_or_after(&without, on, now) {
        Some(renewal) if renewal == on => Ok(()),
        Some(renewal) => {
            db(ScheduledRows::defer(
                &written.rows(change.tenant_id, change.subscription_id),
                conn,
                change.subscription_id,
                on,
                renewal,
            )
            .await)?;
            written.on = renewal;
            written.plan_window = None;
            written.replaced_plan_window = None;
            let record = serde_json::to_value(&written).map_err(|error| {
                Report::new(StoreError::SerdeError("written facts".into(), error))
            })?;
            db(SubscriptionChangeRow::defer(
                conn,
                &change.id,
                renewal.and_time(NaiveTime::MIN),
                record,
            )
            .await)?;
            db(SubscriptionChangeRow::drop_amendment_settlement_due(
                conn,
                change.subscription_id,
                on,
            )
            .await)
        }
        None => withdraw_pending_tx(conn, change, "The subscription ends before its renewal")
            .await
            .map(|_| ()),
    }
}

/// Every kind of change written as facts.
pub(crate) const CHANGES: &[SubscriptionChangeKindEnum] = &[
    SubscriptionChangeKindEnum::PlanChange,
    SubscriptionChangeKindEnum::Amendment,
];

/// Withdraws the subscription's changes of `kinds` still to take effect; returns how many.
pub(crate) async fn withdraw_pending_of_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    kinds: &[SubscriptionChangeKindEnum],
    reason: &str,
) -> StoreResult<usize> {
    withdraw_pending_of_except_tx(conn, tenant_id, subscription_id, kinds, &[], reason).await
}

/// [`withdraw_pending_of_tx`], `keep` aside.
pub(crate) async fn withdraw_pending_of_except_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    kinds: &[SubscriptionChangeKindEnum],
    keep: &[ScheduledEventId],
    reason: &str,
) -> StoreResult<usize> {
    let mut withdrawn = 0;
    for change in
        db(SubscriptionChangeRow::list_pending(conn, &tenant_id, subscription_id, kinds).await)?
            .into_iter()
            .filter(|change| !keep.contains(&change.id))
    {
        if withdraw_pending_tx(conn, &change, reason).await? {
            withdrawn += 1;
        }
    }
    Ok(withdrawn)
}

/// A change requested now for a later date, written as facts at once and recorded with the rows
/// it wrote, so it stays cancellable until its date.
pub(crate) async fn schedule_as_facts_tx(
    store: &crate::Store,
    conn: &mut PgConn,
    change: ScheduledEventNew,
) -> StoreResult<ScheduledEvent> {
    schedule_with(store, conn, change, false).await
}

/// [`schedule_as_facts_tx`] for a timeline change written again (R-162), on or after an ending
/// still to come: the ending withdraws it when it happens, as it does the timeline it cuts.
pub(crate) async fn schedule_replayed_as_facts_tx(
    store: &crate::Store,
    conn: &mut PgConn,
    change: ScheduledEventNew,
) -> StoreResult<ScheduledEvent> {
    schedule_with(store, conn, change, true).await
}

async fn schedule_with(
    _store: &crate::Store,
    conn: &mut PgConn,
    change: ScheduledEventNew,
    through_ending: bool,
) -> StoreResult<ScheduledEvent> {
    let lock =
        db(SubscriptionRow::lock_subscription_for_update(conn, change.subscription_id).await)?;
    let event = change.into_event(common_clock::now().naive_utc());
    let facts = match resolve_change_with(conn, &event, false, through_ending).await? {
        Ok(facts) => facts,
        Err(why) => {
            return Err(Report::new(StoreError::InvalidArgument(format!(
                "the change cannot be scheduled: {why:?}"
            ))));
        }
    };
    let written = write_facts(conn, &facts, &event).await?;
    let record = serde_json::to_value(&written)
        .map_err(|error| Report::new(StoreError::SerdeError("written facts".into(), error)))?;
    let row = db(event.to_row(record)?.insert(conn).await)?;
    db(diesel_models::query::credit_inputs::bump_customer(conn, &lock).await)?;
    ScheduledEvent::try_from(row)
}

impl ServicesEdge {
    /// What a recorded change resolves to against the rows before it wrote them, read in a
    /// transaction rolled back: nothing changes.
    pub async fn resolve_scheduled_change(
        &self,
        tenant_id: TenantId,
        event_id: ScheduledEventId,
    ) -> StoreResult<Resolution> {
        let mut resolution = None;
        let rolled_back = self
            .store
            .transaction(|conn| {
                let resolution = &mut resolution;
                async move {
                    let row =
                        db(SubscriptionChangeRow::get_by_id(conn, event_id, &tenant_id).await)?;
                    withdraw_pending_tx(conn, &row, "resolved again").await?;
                    let event = ScheduledEvent::try_from(row)?;
                    *resolution = Some(resolve_change_tx(conn, &event).await?);
                    Err::<(), _>(Report::new(StoreError::FailedPrecondition(
                        "a resolution is rolled back".into(),
                    )))
                }
                .scope_boxed()
            })
            .await;
        match (resolution, rolled_back) {
            (Some(resolution), _) => Ok(resolution),
            (None, Err(error)) => Err(error),
            (None, Ok(())) => Err(Report::new(StoreError::FailedPrecondition(
                "a resolution was not rolled back".into(),
            ))),
        }
    }

    /// Derives the subscription's state from its facts at `at`, under its customer's lock, as a
    /// fact edit does (documents left to the controller); the step it took.
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
                        .step_subscription_tx(
                            conn,
                            &common_domain::actor::Actor::System,
                            tenant_id,
                            subscription_id,
                            at,
                            crate::services::lifecycle::derived::Issue::StateOnly,
                        )
                        .await?
                        .map(|stepped| stepped.step);
                    db(diesel_models::query::credit_inputs::bump_customer(conn, &lock).await)?;
                    Ok(step)
                }
                .scope_boxed()
            })
            .await
    }
}

/// What the conversion of the scheduled changes an earlier release left pending did.
#[derive(Debug, Default, Clone)]
pub struct PendingConversion {
    pub written: usize,
    /// Each change of a tenant not live that could not be written as facts, cancelled, and why.
    pub cancelled: Vec<(ScheduledEventId, String)>,
}

/// The changes of live tenants that could not be written as facts, and why: the release refuses.
#[derive(Debug, Clone)]
pub struct LiveChangesUnwritten(pub Vec<(ScheduledEventId, TenantId, String)>);

#[derive(diesel::QueryableByName)]
struct PendingEventRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    id: ScheduledEventId,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    tenant_id: TenantId,
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    subscription_id: SubscriptionId,
    #[diesel(sql_type = diesel::sql_types::Timestamp)]
    scheduled_time: NaiveDateTime,
    #[diesel(sql_type = diesel::sql_types::Jsonb)]
    event_data: serde_json::Value,
    #[diesel(sql_type = diesel::sql_types::Timestamp)]
    created_at: NaiveDateTime,
    #[diesel(sql_type = diesel::sql_types::Timestamp)]
    updated_at: NaiveDateTime,
    #[diesel(sql_type = diesel::sql_types::Text)]
    source: String,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    created_by_customer: bool,
    #[diesel(sql_type = diesel_models::schema::sql_types::ActorTypeEnum)]
    actor_type: diesel_models::enums::ActorTypeEnum,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Uuid>)]
    actor_uuid: Option<uuid::Uuid>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    actor_alias: Option<String>,
    /// An active tenant in production or development: its changes are never cancelled.
    #[diesel(sql_type = diesel::sql_types::Bool)]
    live: bool,
}

impl PendingEventRow {
    fn event(self) -> StoreResult<ScheduledEvent> {
        let event_data = ScheduledEventData::try_from(self.event_data)?;
        Ok(ScheduledEvent {
            id: self.id,
            subscription_id: self.subscription_id,
            tenant_id: self.tenant_id,
            event_type: event_data.to_event_type_enum(),
            scheduled_time: self.scheduled_time,
            event_data,
            created_at: self.created_at,
            updated_at: self.updated_at,
            withdrawn_at: None,
            source: self.source,
            created_by_customer: self.created_by_customer,
            actor_type: self.actor_type.into(),
            actor_uuid: self.actor_uuid,
            actor_alias: self.actor_alias,
        })
    }
}

/// Every plan change and amendment an earlier release left pending as a scheduled event, of every
/// tenant (archived and sandbox ones included), written as the facts it becomes and moved, id
/// kept, to `subscription_change` with the rows it wrote, each in its own transaction. One of a
/// live tenant (active, in production or
/// development) that cannot be written fails the conversion, every such change listed; one of any
/// other tenant is cancelled with its reason. Run by the migrations just before that drop, on
/// their connection and in their transaction; nothing once the table is gone.
pub async fn write_pending_scheduled_changes(
    conn: &mut PgConn,
) -> StoreResult<Result<PendingConversion, LiveChangesUnwritten>> {
    use diesel_async::RunQueryDsl;

    #[derive(diesel::QueryableByName)]
    struct Exists {
        #[diesel(sql_type = diesel::sql_types::Bool)]
        exists: bool,
    }
    let table: Exists =
        diesel::sql_query("SELECT to_regclass('scheduled_event') IS NOT NULL AS exists")
            .get_result(conn)
            .await
            .map_err(sql_error)?;
    let mut conversion = PendingConversion::default();
    if !table.exists {
        return Ok(Ok(conversion));
    }
    let pending: Vec<PendingEventRow> = diesel::sql_query(
        "SELECT e.id, e.tenant_id, e.subscription_id, e.scheduled_time, e.event_data, e.created_at,
                e.updated_at, e.source, e.created_by_customer, e.actor_type, e.actor_uuid,
                e.actor_alias,
                t.status = 'ACTIVE' AND t.environment IN ('PRODUCTION', 'DEVELOPMENT') AS live
         FROM scheduled_event e
         JOIN tenant t ON t.id = e.tenant_id
         WHERE e.status IN ('PENDING', 'PROCESSING')
           AND e.event_type IN ('APPLY_PLAN_CHANGE', 'APPLY_AMENDMENT')
         ORDER BY e.scheduled_time, e.id",
    )
    .load(conn)
    .await
    .map_err(sql_error)?;
    let mut live_unwritten = Vec::new();
    for row in pending {
        let (id, tenant_id, live) = (row.id, row.tenant_id, row.live);
        let why = match write_pending_event(conn, row).await {
            Ok(()) => {
                conversion.written += 1;
                continue;
            }
            Err(error) => format!("{error:?}"),
        };
        if live {
            live_unwritten.push((id, tenant_id, why));
            continue;
        }
        log::error!("pending scheduled change {id} not written as facts, cancelled: {why}");
        diesel::sql_query(
            "UPDATE scheduled_event
             SET status = 'CANCELED', error = $2, updated_at = $3
             WHERE id = $1 AND status IN ('PENDING', 'PROCESSING')",
        )
        .bind::<diesel::sql_types::Uuid, _>(id)
        .bind::<diesel::sql_types::Text, _>(format!("not written as facts: {why}"))
        .bind::<diesel::sql_types::Timestamp, _>(common_clock::now().naive_utc())
        .execute(conn)
        .await
        .map_err(sql_error)?;
        conversion.cancelled.push((id, why));
    }
    if live_unwritten.is_empty() {
        Ok(Ok(conversion))
    } else {
        Ok(Err(LiveChangesUnwritten(live_unwritten)))
    }
}

fn sql_error(error: diesel::result::Error) -> Report<StoreError> {
    crate::errors::StoreErrorContainer::from(error).into()
}

/// One pending change written as facts and moved to `subscription_change` with them; rolled back
/// whole when it cannot be.
async fn write_pending_event(conn: &mut PgConn, row: PendingEventRow) -> StoreResult<()> {
    use diesel_async::{AsyncConnection, RunQueryDsl};

    let event = row.event()?;
    let event = &event;
    let written = conn
        .transaction(async |conn| {
            let written: StoreResult<()> = async {
                let lock = db(SubscriptionRow::lock_subscription_for_update(
                    conn,
                    event.subscription_id,
                )
                .await)?;
                // Due already, it is written as the earlier release would have applied it.
                let due = event.scheduled_time <= common_clock::now().naive_utc();
                let facts = resolve_change_with(conn, event, due, false)
                    .await?
                    .map_err(|why| Report::new(StoreError::InvalidArgument(format!("{why:?}"))))?;
                let written = write_facts(conn, &facts, event).await?;
                let record = serde_json::to_value(&written).map_err(|error| {
                    Report::new(StoreError::SerdeError("written facts".into(), error))
                })?;
                diesel::sql_query(
                    "WITH moved AS (
                         DELETE FROM scheduled_event
                         WHERE id = $1 AND status IN ('PENDING', 'PROCESSING')
                         RETURNING *)
                     INSERT INTO subscription_change (id, tenant_id, subscription_id, kind,
                         effective_at, change, written_facts, source, created_by_customer,
                         actor_type, actor_uuid, actor_alias, created_at, updated_at)
                     SELECT id, tenant_id, subscription_id,
                            CASE event_type
                                WHEN 'APPLY_PLAN_CHANGE' THEN 'PLAN_CHANGE'::\"SubscriptionChangeKindEnum\"
                                ELSE 'AMENDMENT'::\"SubscriptionChangeKindEnum\"
                            END,
                            scheduled_time, event_data, $2, source, created_by_customer,
                            actor_type, actor_uuid, actor_alias, created_at, $3
                     FROM moved",
                )
                .bind::<diesel::sql_types::Uuid, _>(event.id)
                .bind::<diesel::sql_types::Jsonb, _>(record)
                .bind::<diesel::sql_types::Timestamp, _>(common_clock::now().naive_utc())
                .execute(conn)
                .await
                .map_err(sql_error)?;
                // Inside a period its adjustment and notes are due on its day: the controller's.
                let kind = event.event_data.kind();
                db(SubscriptionChangeRow::mark_settlement_due_inside_period(
                    conn,
                    event.subscription_id,
                    event.scheduled_time.date(),
                    kind,
                )
                .await)?;
                db(diesel_models::query::credit_inputs::bump_customer(conn, &lock).await)
            }
            .await;
            written.map_err(crate::errors::StoreErrorContainer::from)
        })
        .await;
    written.map_err(Into::into)
}
