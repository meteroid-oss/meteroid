//! Phases: dated stretches of a subscription with their own terms. Each boundary is written as a
//! change of kind Phase whose facts are the rows billing reads; billing never reads a phase
//! (ADR 0015, R-146).
use chrono::{NaiveDate, NaiveTime};
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, SubscriptionId, SubscriptionPhaseId, SubscriptionPriceComponentId, TenantId,
};
use diesel_models::PgConn;
use diesel_models::enums::SubscriptionChangeKindEnum;
use diesel_models::subscription_changes::SubscriptionChangeRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use diesel_models::subscription_phases::SubscriptionPhaseRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

use crate::StoreResult;
use crate::domain::scheduled_events::{
    ComponentMapping, ResolvedComponentInsert, ScheduledEventData, ScheduledEventNew,
};
use crate::domain::subscription_components::SubscriptionFee;
use crate::domain::subscription_phases::{
    SubscriptionPhase, SubscriptionPhaseInput, chain, repriced,
};
use crate::errors::StoreError;
use crate::services::Services;
use crate::services::subscriptions::timeline_rewrite;

fn db<T>(result: diesel_models::DbResult<T>) -> StoreResult<T> {
    result.map_err(Into::into)
}

fn invalid(message: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message.into()))
}

impl Services {
    pub(in crate::services) async fn list_subscription_phases(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Vec<SubscriptionPhase>> {
        let mut conn = self.store.get_conn().await?;
        list_phases_tx(&mut conn, tenant_id, subscription_id).await
    }

    /// Sets the phases starting after today: those unchanged are kept, the first changed one and
    /// every later one are withdrawn newest first and written again from their terms, in order.
    /// A phase already started is never rewritten.
    pub(in crate::services) async fn set_subscription_phases(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        phases: Vec<SubscriptionPhaseInput>,
    ) -> StoreResult<Vec<SubscriptionPhase>> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                let phases = &phases;
                async move {
                    let lock = SubscriptionRow::lock_subscription_for_update(conn, subscription_id)
                        .await?;
                    diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
                    set_phases_tx(self, conn, actor, tenant_id, subscription_id, phases).await?;
                    self.derive_state_tx(
                        conn,
                        actor,
                        tenant_id,
                        subscription_id,
                        common_clock::now(),
                    )
                    .await?;
                    list_phases_tx(conn, tenant_id, subscription_id).await
                }
                .scope_boxed()
            })
            .await
    }
}

/// Sets the phases starting after today inside the caller's transaction, under its lock: those
/// unchanged are kept, the first changed one and every later one withdrawn newest first and
/// written again (R-146). The subscription's state is derived by the caller.
pub(in crate::services) async fn set_phases_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    phases: &[SubscriptionPhaseInput],
) -> StoreResult<()> {
    let today = common_clock::now().date_naive();

    phases.iter().try_for_each(validate_terms)?;
    let term = crate::services::subscriptions::terms::detach_renewal_tx(
        conn,
        tenant_id,
        subscription_id,
        phases,
    )
    .await?;
    let phases = &normalized(conn, tenant_id, subscription_id, phases).await?;
    let live = list_phases_tx(conn, tenant_id, subscription_id).await?;
    let (started, future): (Vec<_>, Vec<_>) = live
        .into_iter()
        .partition(|phase| phase.start_date <= today);
    validate_dates(phases, today, started.last())?;

    let kept = future
        .iter()
        .zip(phases.iter())
        .take_while(|(stored, input)| same_phase(stored, input))
        .count();
    let from = future
        .get(kept)
        .map(|phase| phase.start_date)
        .into_iter()
        .chain(phases.get(kept).map(|phase| phase.start_date))
        .min();
    let Some(from) = from else {
        if let Some(term) = term {
            crate::services::subscriptions::terms::attach_renewal_tx(services, conn, actor, term)
                .await?;
        }
        return Ok(());
    };
    // The dated items after the first changed phase are written again with the new phases, in
    // date order (R-162).
    let mut lifted = timeline_rewrite::lift_tx(conn, tenant_id, subscription_id, from).await?;
    lifted.take_phases();
    lifted.set_term(term);
    for phase in future[kept..].iter().rev() {
        withdraw_phase_tx(conn, tenant_id, phase, "Replaced by new phases").await?;
    }
    let first_seq = started.len() + kept;
    let authored: Vec<(u32, SubscriptionPhaseInput)> = phases
        .iter()
        .enumerate()
        .skip(kept)
        .map(|(i, input)| {
            (
                u32::try_from(first_seq + i - kept).unwrap_or(u32::MAX),
                input.clone(),
            )
        })
        .collect();
    timeline_rewrite::replay_with_tx(services, conn, actor, lifted, authored).await?;
    Ok(())
}

pub(crate) async fn list_phases_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<Vec<SubscriptionPhase>> {
    let rows = db(SubscriptionPhaseRow::list_live(conn, &tenant_id, subscription_id).await)?;
    let phases = rows
        .into_iter()
        .map(SubscriptionPhase::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(chain(phases))
}

/// A timeline change prices a fee from the start of one of that fee's own periods, so it needs
/// no adjustment: an annual fee of a monthly subscription is repriced on its anniversary (R-084).
pub(crate) fn on_fee_period(
    facts: &billing_core::facts::Facts<crate::services::evaluate::pricer::StoreFee>,
    day: NaiveDate,
    name: &str,
    period: crate::domain::enums::SubscriptionFeeBillingPeriod,
) -> StoreResult<()> {
    let Some(months) = period
        .as_billing_period_opt()
        .map(|period| period.as_months())
    else {
        return Ok(());
    };
    if billing_core::evaluate::starts_fee_period(facts, day, months) {
        return Ok(());
    }
    Err(invalid(format!(
        "{name} is billed every {months} months: {day} does not start one of its periods"
    )))
}

/// The subscription's phases still to start withdrawn newest first, as its termination does.
pub(crate) async fn withdraw_future_phases_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    after: NaiveDate,
    reason: &str,
) -> StoreResult<()> {
    let phases = list_phases_tx(conn, tenant_id, subscription_id).await?;
    for phase in phases.iter().rev().filter(|phase| phase.start_date > after) {
        // One already applied (an ending backdated before it) is history: its owner goes alone.
        let change = db(SubscriptionChangeRow::get_by_id(conn, phase.change_id, &tenant_id).await)?;
        if change.withdrawn_at.is_none() {
            crate::services::facts::pending::withdraw_pending_tx(conn, &change, reason).await?;
        }
        db(SubscriptionPhaseRow::withdraw(conn, phase.id).await)?;
    }
    Ok(())
}

/// The writers of timeline changes: phases (their switches included), dated items and discounts.
pub(crate) const TIMELINE_SOURCES: [&str; 2] = ["phase", "timeline_item"];

/// Whether `change` belongs to the timeline, whatever it does: it goes with its owner (I33).
pub(crate) fn is_timeline_change(change: &SubscriptionChangeRow) -> bool {
    change.kind == SubscriptionChangeKindEnum::Phase
        || TIMELINE_SOURCES.contains(&change.source.as_str())
}

/// The pending changes of `kinds` no timeline owns: a plan change or amendment made alone.
pub(crate) async fn pending_one_offs_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    kinds: &[SubscriptionChangeKindEnum],
) -> StoreResult<Vec<SubscriptionChangeRow>> {
    Ok(
        db(SubscriptionChangeRow::list_pending(conn, &tenant_id, subscription_id, kinds).await)?
            .into_iter()
            .filter(|change| !is_timeline_change(change))
            .collect(),
    )
}

/// [`pending_one_offs_tx`] withdrawn, newest first. Returns how many.
pub(crate) async fn withdraw_one_offs_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    kinds: &[SubscriptionChangeKindEnum],
    reason: &str,
) -> StoreResult<usize> {
    let mut changes = pending_one_offs_tx(conn, tenant_id, subscription_id, kinds).await?;
    changes.sort_by_key(|change| std::cmp::Reverse((change.effective_at, change.created_at)));
    let mut withdrawn = 0;
    for change in &changes {
        if crate::services::facts::pending::withdraw_pending_tx(conn, change, reason).await? {
            withdrawn += 1;
        }
    }
    Ok(withdrawn)
}

pub(crate) async fn withdraw_phase_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    phase: &SubscriptionPhase,
    reason: &str,
) -> StoreResult<()> {
    let change = db(SubscriptionChangeRow::get_by_id(conn, phase.change_id, &tenant_id).await)?;
    if change.withdrawn_at.is_none()
        && !crate::services::facts::pending::withdraw_pending_tx(conn, &change, reason).await?
    {
        return Err(invalid(format!(
            "the phase starting {} has started and cannot be withdrawn",
            phase.start_date
        )));
    }
    db(SubscriptionPhaseRow::withdraw(conn, phase.id).await)?;
    Ok(())
}

/// A price is at least zero, to at most 8 decimals, as the catalog's.
pub(crate) fn validate_price(price: rust_decimal::Decimal) -> StoreResult<()> {
    if price.is_sign_negative() || price.normalize().scale() > 8 {
        return Err(invalid(format!(
            "{price} is not a price: at least zero, to at most 8 decimals"
        )));
    }
    Ok(())
}

fn validate_terms(phase: &SubscriptionPhaseInput) -> StoreResult<()> {
    if phase.terms.prorated
        && (phase.terms.plan.is_some()
            || phase.terms.minimum != crate::domain::subscription_phases::PhaseMinimum::Kept)
    {
        return Err(invalid(
            "a phase inside a period prices fees and seats: plans and minimums change on renewals",
        ));
    }
    for component in &phase.terms.components {
        if component.list_rate.is_some() {
            return Err(invalid(
                "a subscription's phase prices by amounts in its currency: a share of the list price is a template's",
            ));
        }
        validate_price(component.price)?;
    }
    if let crate::domain::subscription_phases::PhaseMinimum::Amount { amount_subunit, .. } =
        &phase.terms.minimum
        && *amount_subunit <= 0
    {
        return Err(invalid("a phase's minimum is more than zero"));
    }
    Ok(())
}

fn validate_dates(
    phases: &[SubscriptionPhaseInput],
    today: NaiveDate,
    last_started: Option<&SubscriptionPhase>,
) -> StoreResult<()> {
    let floor = last_started.map_or(today, |phase| phase.start_date.max(today));
    let mut previous = floor;
    for phase in phases {
        if phase.start_date <= previous {
            return Err(invalid(format!(
                "phases start after today and after one another: {} is not after {previous}",
                phase.start_date
            )));
        }
        previous = phase.start_date;
    }
    Ok(())
}

/// A phase starts on one of the subscription's renewals, so its boundary bills with that
/// renewal and needs no adjustment of its own (R-146). Read with the phases written before it,
/// whose switches set the cadence (R-150). The plan version in force on its start.
pub(crate) async fn check_on_renewal(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    start: NaiveDate,
    prorated: bool,
) -> StoreResult<(common_domain::ids::PlanVersionId, bool)> {
    check_on_renewal_as(conn, tenant_id, subscription_id, start, prorated, false).await
}

/// [`check_on_renewal`]; `through_ending`: on the renewals the subscription has without its
/// ending still to come, which cuts a timeline written again only when it happens (R-162).
async fn check_on_renewal_as(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    start: NaiveDate,
    prorated: bool,
    through_ending: bool,
) -> StoreResult<(common_domain::ids::PlanVersionId, bool)> {
    let loaded = facts_through_ending(conn, tenant_id, subscription_id, through_ending).await?;
    let at = start.and_time(NaiveTime::MIN).and_utc();
    let renewal = billing_core::evaluate::renewal_on_or_after(&loaded.facts, start, at);
    let on_renewal = renewal == Some(start);
    if !on_renewal && (!prorated || renewal.is_none()) {
        return Err(invalid(format!(
            "a phase starts on a renewal of the subscription: {start} is not one{}",
            renewal.map_or(String::new(), |next| format!(" (next: {next})"))
        )));
    }
    let plan = loaded
        .facts
        .plan_on(start)
        .map(|window| window.plan_version)
        .ok_or_else(|| invalid(format!("the subscription has no plan on {start}")))?;
    Ok((plan, on_renewal))
}

/// The subscription's facts, without its ending still to come when `through_ending`.
pub(in crate::services) async fn facts_through_ending(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    through_ending: bool,
) -> StoreResult<crate::services::evaluate::load::LoadedFacts> {
    let today = common_clock::now().date_naive();
    let mut loaded =
        crate::services::evaluate::load::load_facts(conn, tenant_id, subscription_id, today, None)
            .await?;
    if through_ending
        && loaded
            .facts
            .ending
            .is_some_and(|ending| ending.on() > today)
    {
        loaded.facts.ending = None;
    }
    Ok(loaded)
}

/// An end date moved (R-140) is refused while the subscription has a term, whose end is its own
/// (R-152), or a timeline change on or after it, which would never apply (R-146).
pub(crate) async fn refuse_end_date_over_timeline_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    end_date: Option<NaiveDate>,
) -> StoreResult<()> {
    if db(
        diesel_models::subscription_phases::SubscriptionTermRow::find(
            conn,
            &tenant_id,
            subscription_id,
        )
        .await,
    )?
    .is_some()
    {
        return Err(invalid(
            "This subscription has a contract term: its end is the term's, edit the term instead",
        ));
    }
    let Some(end) = end_date else {
        return Ok(());
    };
    if let Some(change) = db(SubscriptionChangeRow::list_pending(
        conn,
        &tenant_id,
        subscription_id,
        &[SubscriptionChangeKindEnum::Phase],
    )
    .await)?
    .iter()
    .find(|change| change.effective_at.date() >= end)
    {
        return Err(invalid(format!(
            "A phase, dated item or discount changes the subscription on {}: an end date on {end} \
             would leave it unapplied, edit the timeline first",
            change.effective_at.date()
        )));
    }
    Ok(())
}

/// The option the subscription holds on `on`: the one the last switch written ahead before it
/// names, else its own. Every switch a phase writes names its option.
async fn option_in_force_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    on: NaiveDate,
) -> StoreResult<Option<common_domain::ids::PlanOptionId>> {
    let switch = db(SubscriptionChangeRow::list_in_force_unapplied(
        conn,
        &tenant_id,
        subscription_id,
        &[
            SubscriptionChangeKindEnum::Phase,
            SubscriptionChangeKindEnum::PlanChange,
        ],
        on.and_time(NaiveTime::MIN),
    )
    .await)?
    .into_iter()
    .filter(|change| change.change.pointer("/ApplyPlanChange").is_some())
    .max_by_key(|change| change.effective_at);
    if let Some(switch) = switch {
        return Ok(switch
            .change
            .pointer("/ApplyPlanChange/new_plan_option_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|id| id.parse().ok()));
    }
    Ok(
        db(SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id).await)?
            .subscription
            .plan_option_id,
    )
}

/// Each priced fee named by its lineage's root and its name, whichever of its rows the caller
/// named, so the same terms compare equal however they were written.
async fn normalized(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    phases: &[SubscriptionPhaseInput],
) -> StoreResult<Vec<SubscriptionPhaseInput>> {
    let rows = db(SubscriptionComponentRow::list_fact_rows(conn, subscription_id).await)?;
    let mut out = phases.to_vec();
    for phase in &mut out {
        name_seats_tx(conn, tenant_id, &mut phase.terms.min_seats).await?;
        for component in &mut phase.terms.components {
            match (component.lineage, component.plan_component) {
                (Some(named), None) => {
                    let row = rows.iter().find(|row| row.id == named).ok_or_else(|| {
                        invalid(format!("no component {named} on the subscription"))
                    })?;
                    component.lineage = Some(lineage_of(row));
                    component.name = Some(row.name.clone());
                }
                (None, Some(_)) => {}
                _ => {
                    return Err(invalid(
                        "a phase names each fee it prices by its lineage or by its plan component",
                    ));
                }
            }
        }
    }
    Ok(out)
}

fn same_phase(stored: &SubscriptionPhase, input: &SubscriptionPhaseInput) -> bool {
    stored.start_date == input.start_date
        && stored.label == input.label
        && serde_json::to_value(&stored.terms).ok() == serde_json::to_value(&input.terms).ok()
}

/// The row a phase opens for a lineage: the same id whenever the same phase is written again.
fn phase_row_id(
    subscription_id: SubscriptionId,
    start: NaiveDate,
    lineage: SubscriptionPriceComponentId,
) -> SubscriptionPriceComponentId {
    SubscriptionPriceComponentId::from(uuid::Uuid::new_v5(
        &subscription_id.as_uuid(),
        format!("phase:{start}:{}", lineage.as_uuid()).as_bytes(),
    ))
}

pub(crate) fn lineage_of(row: &SubscriptionComponentRow) -> SubscriptionPriceComponentId {
    row.lineage_id.unwrap_or(row.id)
}

pub(crate) fn fee_of(row: &SubscriptionComponentRow) -> StoreResult<SubscriptionFee> {
    row.legacy_fee
        .clone()
        .map(SubscriptionFee::try_from)
        .transpose()?
        .ok_or_else(|| invalid(format!("{} has no fee to price", row.name)))
}

/// `base` at the phase's price and seat floor for it: a floor never lowers the fee's own.
fn phased_fee(
    name: &str,
    base: &SubscriptionFee,
    price: Option<rust_decimal::Decimal>,
    min_seats: Option<u32>,
) -> StoreResult<SubscriptionFee> {
    let mut phased = match price {
        Some(price) => repriced(base, price).ok_or_else(|| {
            invalid(format!(
                "a phase prices rates, seats and capacities by one amount, not {name}"
            ))
        })?,
        None => base.clone(),
    };
    if let (
        Some(least),
        SubscriptionFee::Slot {
            min_slots,
            max_slots,
            ..
        },
    ) = (min_seats, &mut phased)
    {
        if max_slots.is_some_and(|max| max < least) {
            return Err(invalid(format!(
                "{name}: {least} seats is above the fee's maximum"
            )));
        }
        *min_slots = Some(min_slots.map_or(least, |min| min.max(least)));
    }
    Ok(phased)
}

fn same_fee(a: &SubscriptionFee, b: &SubscriptionFee) -> bool {
    serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
}

fn seats_of(input: &SubscriptionPhaseInput, fee: &SubscriptionFee) -> Option<u32> {
    input
        .terms
        .min_seats
        .iter()
        .find(|seats| seats.counts(fee))
        .map(|seats| seats.slots)
}

/// The phases' seat floors with the codes of the quantities they name, each of the tenant's.
pub(crate) async fn name_seats_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    seats: &mut [crate::domain::scheduled_events::PhaseSeats],
) -> StoreResult<()> {
    for floor in seats {
        floor.code = db(diesel_models::managed_quantities::ManagedQuantityRow::find(
            conn,
            tenant_id,
            floor.quantity_id,
        )
        .await)?
        .ok_or_else(|| invalid(format!("no quantity {}", floor.quantity_id)))?
        .code;
    }
    Ok(())
}

/// The row a phase opens to floor a seat add-on of `lineage`.
fn phase_add_on_row_id(
    subscription_id: SubscriptionId,
    start: NaiveDate,
    lineage: common_domain::ids::SubscriptionAddOnId,
) -> common_domain::ids::SubscriptionAddOnId {
    common_domain::ids::SubscriptionAddOnId::from(uuid::Uuid::new_v5(
        &subscription_id.as_uuid(),
        format!("phase:{start}:add_on:{}", lineage.as_uuid()).as_bytes(),
    ))
}

/// The seat add-ons a phase floors, as its seat floor floors the plan's seat charges (R-146).
#[derive(Default)]
struct FlooredAddOns {
    close: Vec<common_domain::ids::SubscriptionAddOnId>,
    insert: Vec<crate::domain::scheduled_events::ResolvedAddOnInsert>,
    /// The quantities a seat add-on in force counts.
    counted: Vec<common_domain::ids::ManagedQuantityId>,
}

/// Each seat add-on in force on the phase's start (of `compatible` add-ons, for a switch) at the
/// floor the phase sets on its quantity, or back at its baseline when an earlier phase floored
/// it: its row closed and opened again, never below its own minimum; the seats held untouched.
async fn floor_add_ons_tx(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    input: &SubscriptionPhaseInput,
    compatible: Option<&std::collections::HashSet<common_domain::ids::AddOnId>>,
) -> StoreResult<FlooredAddOns> {
    use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
    let start = input.start_date;
    let rows = db(SubscriptionAddOnRow::list_fact_rows(conn, subscription_id).await)?;
    let lineage_of = |row: &SubscriptionAddOnRow| row.lineage_id.unwrap_or(row.id);
    let fee_of = |row: &SubscriptionAddOnRow| -> StoreResult<Option<SubscriptionFee>> {
        row.legacy_fee
            .clone()
            .map(SubscriptionFee::try_from)
            .transpose()
    };
    let mut floored = FlooredAddOns::default();
    for row in rows.iter().filter(|row| {
        row.effective_to.is_none() && compatible.is_none_or(|ids| ids.contains(&row.add_on_id))
    }) {
        let Some(fee) = fee_of(row)? else { continue };
        let SubscriptionFee::Slot { quantity_id, .. } = &fee else {
            continue;
        };
        if let Some(quantity) = quantity_id {
            floored.counted.push(*quantity);
        }
        let lineage = lineage_of(row);
        let base = rows
            .iter()
            .filter(|other| {
                lineage_of(other) == lineage
                    && other.id
                        != phase_add_on_row_id(subscription_id, other.effective_from, lineage)
            })
            .max_by_key(|other| other.effective_from)
            .unwrap_or(row);
        let Some(base_fee) = fee_of(base)? else {
            continue;
        };
        let target = phased_fee(&row.name, &base_fee, None, seats_of(input, &base_fee))?;
        if same_fee(&target, &fee) {
            continue;
        }
        if row.effective_from > start {
            return Err(invalid(format!(
                "{} changes on {}, after the phase's start",
                row.name, row.effective_from
            )));
        }
        let restored = same_fee(&target, &base_fee);
        floored.close.push(row.id);
        floored
            .insert
            .push(crate::domain::scheduled_events::ResolvedAddOnInsert {
                add_on_id: row.add_on_id,
                name: row.name.clone(),
                period: row.period.clone().into(),
                fee: target,
                product_id: row.product_id,
                price_id: restored.then_some(base.price_id).flatten(),
                quantity: row.quantity,
                lineage_id: Some(lineage),
                subscription_add_on_id: Some(phase_add_on_row_id(subscription_id, start, lineage)),
            });
    }
    Ok(floored)
}

fn is_phase_row(subscription_id: SubscriptionId, row: &SubscriptionComponentRow) -> bool {
    row.id == phase_row_id(subscription_id, row.effective_from, lineage_of(row))
}

/// A lineage's fee before any phase priced it: its latest row no phase wrote (R-146). Terms are
/// absolute over it, so a fee a phase leaves out bills it again.
fn baseline_of(
    subscription_id: SubscriptionId,
    rows: &[SubscriptionComponentRow],
    lineage: SubscriptionPriceComponentId,
) -> Option<&SubscriptionComponentRow> {
    rows.iter()
        .filter(|row| lineage_of(row) == lineage && !is_phase_row(subscription_id, row))
        .max_by_key(|row| row.effective_from)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::services) async fn write_phase_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    seq: u32,
    input: &SubscriptionPhaseInput,
) -> StoreResult<SubscriptionPhaseId> {
    refuse_before_one_offs_tx(conn, tenant_id, subscription_id, input.start_date).await?;
    write_phase_as_tx(
        services,
        conn,
        actor,
        tenant_id,
        subscription_id,
        seq,
        input,
        PhaseWrite::Authoring,
    )
    .await
    .map(|written| written.id)
}

/// How a phase is written: as asked (every term must name something), or replayed by a rewrite
/// of the timeline over a change before it, a term naming nothing left inactive (R-162).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PhaseWrite {
    Authoring,
    Replay(SubscriptionPhaseId),
}

/// A phase written: its terms naming nothing on its start (replays only), and whether a replay
/// wrote it inside a period, its start no longer a renewal.
#[derive(Debug, Default)]
pub(crate) struct WrittenPhase {
    pub id: SubscriptionPhaseId,
    pub inactive: Vec<String>,
    pub prorated: bool,
}

/// A timeline edit dated before a pending plan change or scheduled amendment is refused: that
/// change's rows are snapshots of the rows in force when it was written (R-162).
pub(crate) async fn refuse_before_one_offs_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    on: NaiveDate,
) -> StoreResult<()> {
    if let Some(change) = pending_one_offs_tx(
        conn,
        tenant_id,
        subscription_id,
        crate::services::facts::pending::CHANGES,
    )
    .await?
    .iter()
    .find(|change| change.effective_at.date() > on)
    {
        return Err(invalid(format!(
            "A plan change or amendment is scheduled on {}, after {on}: cancel it before editing \
             the timeline before it",
            change.effective_at.date()
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(in crate::services) async fn write_phase_as_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    seq: u32,
    input: &SubscriptionPhaseInput,
    mode: PhaseWrite,
) -> StoreResult<WrittenPhase> {
    let replay = matches!(mode, PhaseWrite::Replay(_));
    let mut inactive: Vec<String> = Vec::new();
    let mut named = input.clone();
    name_seats_tx(conn, tenant_id, &mut named.terms.min_seats).await?;
    let mut prorated_by_replay = false;
    let checked = check_on_renewal_as(
        conn,
        tenant_id,
        subscription_id,
        named.start_date,
        named.terms.prorated,
        replay,
    )
    .await;
    let (in_force, on_renewal) = match checked {
        Ok(checked) => checked,
        Err(_)
            if replay
                && named.terms.plan.is_none()
                && named.terms.minimum
                    == crate::domain::subscription_phases::PhaseMinimum::Kept =>
        {
            named.terms.prorated = true;
            prorated_by_replay = true;
            check_on_renewal_as(
                conn,
                tenant_id,
                subscription_id,
                named.start_date,
                true,
                true,
            )
            .await?
        }
        Err(error) => return Err(error),
    };
    let input = &named;
    let start = input.start_date;
    let facts = crate::services::evaluate::load::load_facts(
        conn,
        tenant_id,
        subscription_id,
        common_clock::now().date_naive(),
        None,
    )
    .await?
    .facts;
    let rows = db(SubscriptionComponentRow::list_fact_rows(conn, subscription_id).await)?;
    let open: Vec<&SubscriptionComponentRow> = rows
        .iter()
        .filter(|row| row.effective_to.is_none())
        .collect();
    let refuse_later = |row: &SubscriptionComponentRow| {
        if row.effective_from > start {
            return Err(invalid(format!(
                "{} changes on {}, after the phase's start",
                row.name, row.effective_from
            )));
        }
        Ok(())
    };
    let discounted = crate::services::subscriptions::timeline_items::discounts_to_end_tx(
        conn,
        tenant_id,
        subscription_id,
    )
    .await?;
    let refuse_discounted = |lineage: SubscriptionPriceComponentId, name: &str| match discounted
        .iter()
        .find(|d| d.lineage == lineage)
    {
        Some(discount) => Err(invalid(format!(
            "{name} is discounted until {}: a phase cannot price it before the discount ends",
            discount.until
        ))),
        None => Ok(()),
    };
    let phase_id = match mode {
        PhaseWrite::Replay(existing) => existing,
        PhaseWrite::Authoring => SubscriptionPhaseId::new(),
    };

    let event_data = match &input.terms.plan {
        Some(plan) => {
            if input.terms.minimum != crate::domain::subscription_phases::PhaseMinimum::Kept {
                return Err(invalid(
                    "a phase switching plan takes the new plan's minimum: set another in a later phase",
                ));
            }
            if input.terms.components.iter().any(|c| c.lineage.is_some()) {
                return Err(invalid(
                    "a phase switching plan prices the new plan's components, by plan component",
                ));
            }
            for row in &open {
                refuse_discounted(lineage_of(row), &row.name)?;
                refuse_later(row)?;
            }
            let components =
                db(SubscriptionComponentRow::list_fact_rows(conn, subscription_id).await)?
                    .into_iter()
                    .filter(|row| row.effective_to.is_none())
                    .map(TryInto::try_into)
                    .collect::<StoreResult<Vec<_>>>()?;
            let plan_option_id =
                option_in_force_tx(conn, tenant_id, subscription_id, start).await?;
            let offered: std::collections::HashSet<common_domain::ids::PriceComponentId> = db(
                diesel_models::price_components::PriceComponentRow::list_by_plan_version_id(
                    conn,
                    tenant_id,
                    plan.plan_version_id,
                )
                .await,
            )?
            .into_iter()
            .map(|component| component.id)
            .collect();
            let mut prices = Vec::new();
            for component in &input.terms.components {
                match component.plan_component {
                    Some(id) if offered.contains(&id) => {
                        prices.push(crate::domain::subscription_changes::TargetPrice {
                            component_id: id,
                            price: component.price,
                        });
                    }
                    Some(id) if !replay => {
                        return Err(invalid(format!(
                            "the plan switched to has no component {id}"
                        )));
                    }
                    _ => inactive.push(
                        component
                            .name
                            .clone()
                            .unwrap_or_else(|| "A fee".to_string()),
                    ),
                }
            }
            // Priced as a plan change prices its target (R-163), then floored.
            let (mut mappings, new_plan_option_id) =
                crate::services::subscriptions::plan_change::switch_mappings_tx(
                    conn,
                    &services.store,
                    tenant_id,
                    subscription_id,
                    crate::domain::subscription_changes::PlanTarget {
                        version: plan.plan_version_id,
                        option: plan
                            .option
                            .clone()
                            .map(crate::domain::plan_options::PlanOptionRef::Alias),
                        prices,
                    },
                    crate::services::subscriptions::plan_change::SwitchFrom {
                        plan_version_id: in_force,
                        plan_option_id,
                        components,
                    },
                    &plan.component_params,
                )
                .await?;
            for mapping in &mut mappings {
                let (name, fee) = match mapping {
                    ComponentMapping::Matched { name, fee, .. }
                    | ComponentMapping::Added { name, fee, .. } => (name.clone(), fee),
                    ComponentMapping::Removed { .. } => continue,
                };
                let before = fee.clone();
                *fee = phased_fee(&name, &before, None, seats_of(input, &before))?;
                if !same_fee(fee, &before) {
                    match mapping {
                        ComponentMapping::Matched { priced, .. } => *priced = true,
                        ComponentMapping::Added { price_id, .. } => *price_id = None,
                        ComponentMapping::Removed { .. } => {}
                    }
                }
            }
            let compatible: std::collections::HashSet<common_domain::ids::AddOnId> = db(
                diesel_models::plan_version_add_ons::PlanVersionAddOnRow::list_by_plan_version_id(
                    conn,
                    plan.plan_version_id,
                    tenant_id,
                )
                .await,
            )?
            .into_iter()
            .map(|offered| offered.add_on_id)
            .collect();
            let floored = floor_add_ons_tx(conn, subscription_id, input, Some(&compatible)).await?;
            ScheduledEventData::ApplyPlanChange {
                source_plan_version_id: None,
                new_plan_version_id: plan.plan_version_id,
                new_plan_option_id,
                component_mappings: mappings,
                addon_close: floored.close,
                addon_insert: floored.insert,
            }
        }
        None => {
            // The fees the phase prices or floors, and those an earlier phase left priced.
            let mut targets: Vec<(&SubscriptionComponentRow, Option<rust_decimal::Decimal>)> =
                Vec::new();
            for component in &input.terms.components {
                let row =
                    open.iter()
                        .find(|row| match (component.lineage, component.plan_component) {
                            (Some(lineage), _) => lineage_of(row) == lineage,
                            (None, Some(plan_component)) => {
                                row.price_component_id == Some(plan_component)
                            }
                            (None, None) => false,
                        });
                match row {
                    Some(row) => targets.push((row, Some(component.price))),
                    None if replay => {
                        inactive.push(
                            component
                                .name
                                .clone()
                                .unwrap_or_else(|| "A fee".to_string()),
                        );
                    }
                    None => {
                        return Err(invalid("a fee the phase prices is not on the subscription"));
                    }
                }
            }
            let floored = floor_add_ons_tx(conn, subscription_id, input, None).await?;
            for seats in &input.terms.min_seats {
                let mut found = floored.counted.contains(&seats.quantity_id);
                for row in &open {
                    if seats.counts(&fee_of(row)?) {
                        found = true;
                        if !targets.iter().any(|(target, _)| target.id == row.id) {
                            targets.push((row, None));
                        }
                    }
                }
                if !found && replay {
                    inactive.push(format!("the seat floor of {}", seats.code));
                } else if !found {
                    return Err(invalid(format!(
                        "no seat fee of the subscription counts {}",
                        seats.code
                    )));
                }
            }
            for row in &open {
                if is_phase_row(subscription_id, row)
                    && !targets.iter().any(|(target, _)| target.id == row.id)
                {
                    targets.push((row, None));
                }
            }

            let mut component_close = Vec::new();
            let mut component_insert = Vec::new();
            for (row, price) in targets {
                let lineage = lineage_of(row);
                let base = baseline_of(subscription_id, &rows, lineage).unwrap_or(row);
                let base_fee = fee_of(base)?;
                let fee = phased_fee(&row.name, &base_fee, price, seats_of(input, &base_fee))?;
                if same_fee(&fee, &fee_of(row)?) {
                    continue;
                }
                refuse_discounted(lineage, &row.name)?;
                refuse_later(row)?;
                if on_renewal {
                    on_fee_period(&facts, start, &row.name, row.period.clone().into())?;
                }
                let restored = same_fee(&fee, &base_fee);
                component_close.push(row.id);
                component_insert.push(ResolvedComponentInsert {
                    price_component_id: row.price_component_id,
                    product_id: row.product_id,
                    name: row.name.clone(),
                    period: row.period.clone().into(),
                    fee,
                    is_override: true,
                    price_id: restored.then_some(base.price_id).flatten(),
                    lineage_id: Some(lineage),
                    subscription_component_id: Some(phase_row_id(subscription_id, start, lineage)),
                });
            }
            ScheduledEventData::ApplyPhase {
                owner_id: phase_id.as_uuid(),
                component_close,
                component_insert,
                addon_close: floored.close,
                addon_insert: floored.insert,
                minimum: input.terms.minimum.clone(),
            }
        }
    };

    let change = ScheduledEventNew {
        subscription_id,
        tenant_id,
        scheduled_time: start.and_time(NaiveTime::MIN),
        event_data,
        source: "phase".to_string(),
        created_by: actor.clone(),
    };
    let event = if replay {
        crate::services::facts::pending::schedule_replayed_as_facts_tx(
            &services.store,
            conn,
            change,
        )
        .await?
    } else {
        crate::services::facts::pending::schedule_as_facts_tx(&services.store, conn, change).await?
    };
    if !on_renewal {
        db(SubscriptionChangeRow::mark_settlement_due_on(
            conn,
            tenant_id,
            subscription_id,
            start,
            SubscriptionChangeKindEnum::Phase,
        )
        .await)?;
    }
    let mut stored_terms = input.terms.clone();
    stored_terms.prorated = stored_terms.prorated && !prorated_by_replay;
    let terms = serde_json::to_value(&stored_terms)
        .map_err(|error| Report::new(StoreError::SerdeError("phase terms".into(), error)))?;
    let phase_id = match mode {
        PhaseWrite::Replay(existing) => {
            db(SubscriptionPhaseRow::rewritten(conn, existing, event.id, terms).await)?;
            existing
        }
        PhaseWrite::Authoring => {
            db(SubscriptionPhaseRow {
                id: phase_id,
                tenant_id,
                subscription_id,
                seq: i32::try_from(seq).unwrap_or(i32::MAX),
                start_date: start,
                label: input.label.clone(),
                terms,
                change_id: event.id,
                created_by: actor.as_uuid(),
                created_at: common_clock::now().naive_utc(),
                withdrawn_at: None,
            }
            .insert(conn)
            .await)?;
            phase_id
        }
    };
    Ok(WrittenPhase {
        id: phase_id,
        inactive,
        prorated: prorated_by_replay,
    })
}

/// An operator's minimum would overwrite the dated minimums a phase still to start writes.
pub(crate) async fn refuse_minimum_edit_while_phased_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<()> {
    for change in db(SubscriptionChangeRow::list_pending(
        conn,
        &tenant_id,
        subscription_id,
        &[SubscriptionChangeKindEnum::Phase],
    )
    .await)?
    {
        let event = crate::domain::scheduled_events::ScheduledEvent::try_from(change)?;
        if let ScheduledEventData::ApplyPhase { minimum, .. } = event.event_data
            && minimum != crate::domain::subscription_phases::PhaseMinimum::Kept
        {
            return Err(invalid(
                "A phase still to start sets the minimum commitment: edit the subscription's phases",
            ));
        }
    }
    Ok(())
}
