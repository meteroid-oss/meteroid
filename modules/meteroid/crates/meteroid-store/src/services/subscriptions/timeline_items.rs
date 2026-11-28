//! Timeline items: dated add-ons (an add-on for three months, a trial of one converting to paid)
//! and dated discounts of a fee. Their start and end are timeline changes on renewals, written
//! ahead like a phase's boundary, so they share the phases' guards (ADR 0015, R-147, R-149).
use chrono::{NaiveDate, NaiveTime};
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, ScheduledEventId, SubscriptionAddOnId, SubscriptionId, SubscriptionPriceComponentId,
    TenantId,
};
use diesel_models::PgConn;
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_changes::SubscriptionChangeRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use diesel_models::subscription_phases::SubscriptionTimelineItemRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

use crate::StoreResult;
use crate::domain::scheduled_events::{
    ResolvedAddOnInsert, ResolvedComponentInsert, ScheduledEventData, ScheduledEventNew,
};
use crate::domain::subscription_components::SubscriptionFee;
use crate::domain::subscription_phases::{
    DatedDiscount, DatedDiscountInput, DiscountTerms, ITEM_ADD_ON, ITEM_DISCOUNT, ItemEnd,
    TimelineItem, TimelineItemInput, price_of, repriced,
};
use crate::errors::StoreError;
use crate::repositories::subscriptions::SubscriptionInterface;
use crate::services::Services;

fn db<T>(result: diesel_models::DbResult<T>) -> StoreResult<T> {
    result.map_err(Into::into)
}

fn invalid(message: impl Into<String>) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(message.into()))
}

/// The id of the row an item writes: the same whenever the same item is written again.
fn item_row_id(
    subscription_id: SubscriptionId,
    item: uuid::Uuid,
    which: &str,
) -> SubscriptionAddOnId {
    SubscriptionAddOnId::from(uuid::Uuid::new_v5(
        &subscription_id.as_uuid(),
        format!("item:{item}:{which}").as_bytes(),
    ))
}

impl Services {
    pub(in crate::services) async fn list_timeline_items(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Vec<TimelineItem>> {
        let mut conn = self.store.get_conn().await?;
        list_items_tx(&mut conn, tenant_id, subscription_id).await
    }

    pub(in crate::services) async fn list_dated_discounts(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Vec<DatedDiscount>> {
        let mut conn = self.store.get_conn().await?;
        list_discounts_tx(&mut conn, tenant_id, subscription_id).await
    }

    pub(in crate::services) async fn add_dated_discount(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        input: DatedDiscountInput,
    ) -> StoreResult<DatedDiscount> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                let input = &input;
                async move {
                    let lock = SubscriptionRow::lock_subscription_for_update(conn, subscription_id)
                        .await?;
                    diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
                    let discount =
                        write_discount_tx(self, conn, actor, tenant_id, subscription_id, input)
                            .await?;
                    self.derive_state_tx(
                        conn,
                        actor,
                        tenant_id,
                        subscription_id,
                        common_clock::now(),
                    )
                    .await?;
                    Ok(discount)
                }
                .scope_boxed()
            })
            .await
    }

    pub(in crate::services) async fn add_timeline_item(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        input: TimelineItemInput,
    ) -> StoreResult<TimelineItem> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                let input = &input;
                async move {
                    let lock = SubscriptionRow::lock_subscription_for_update(conn, subscription_id)
                        .await?;
                    diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
                    let item =
                        write_item_tx(self, conn, actor, tenant_id, subscription_id, input).await?;
                    self.derive_state_tx(
                        conn,
                        actor,
                        tenant_id,
                        subscription_id,
                        common_clock::now(),
                    )
                    .await?;
                    Ok(item)
                }
                .scope_boxed()
            })
            .await
    }

    /// Withdraws what of the item is still to come, newest first: once started, its add-on stays
    /// on the subscription with no end, and a discount is kept to its end.
    pub(in crate::services) async fn remove_timeline_item(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        item_id: uuid::Uuid,
    ) -> StoreResult<()> {
        self.store
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    let lock = SubscriptionRow::lock_subscription_for_update(conn, subscription_id)
                        .await?;
                    diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
                    remove_item_tx(conn, tenant_id, subscription_id, item_id).await?;
                    self.derive_state_tx(
                        conn,
                        actor,
                        tenant_id,
                        subscription_id,
                        common_clock::now(),
                    )
                    .await?;
                    Ok(())
                }
                .scope_boxed()
            })
            .await
    }
}

/// Removes a timeline item inside the caller's transaction: what of it is still to come,
/// newest first; refused once a discount or free add-on has started.
pub(crate) async fn remove_item_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    item_id: uuid::Uuid,
) -> StoreResult<()> {
    let item = db(SubscriptionTimelineItemRow::list_live(conn, &tenant_id, subscription_id).await)?
        .into_iter()
        .find(|item| item.id == item_id)
        .ok_or_else(|| {
            Report::new(StoreError::ValueNotFound(format!(
                "no timeline item {item_id}"
            )))
        })?;
    let started = item.start_date <= common_clock::now().date_naive();
    let free = item.kind == ITEM_ADD_ON
        && serde_json::from_value::<crate::domain::subscription_phases::TimelineItemTerms>(
            item.terms.clone(),
        )
        .is_ok_and(|terms| terms.free);
    if started && (item.kind == ITEM_DISCOUNT || free) {
        return Err(invalid(format!(
            "it has started: it ends on {}, its price before coming back",
            item.until_date
        )));
    }
    withdraw_item_tx(conn, tenant_id, &item, "Timeline item removed").await?;
    Ok(())
}

pub(crate) async fn list_items_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<Vec<TimelineItem>> {
    db(SubscriptionTimelineItemRow::list_live(conn, &tenant_id, subscription_id).await)?
        .into_iter()
        .filter(|row| row.kind == ITEM_ADD_ON)
        .map(TimelineItem::try_from)
        .collect()
}

pub(crate) async fn list_discounts_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<Vec<DatedDiscount>> {
    db(SubscriptionTimelineItemRow::list_live(conn, &tenant_id, subscription_id).await)?
        .into_iter()
        .filter(|row| row.kind == ITEM_DISCOUNT)
        .map(DatedDiscount::try_from)
        .collect()
}

/// The discounts still to end: no phase may price their fees meanwhile (R-149).
pub(crate) async fn discounts_to_end_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<Vec<DatedDiscount>> {
    let today = common_clock::now().date_naive();
    Ok(list_discounts_tx(conn, tenant_id, subscription_id)
        .await?
        .into_iter()
        .filter(|discount| discount.until > today)
        .collect())
}

/// The item's changes still to come withdrawn, newest first, and the item with them.
async fn withdraw_item_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    item: &SubscriptionTimelineItemRow,
    reason: &str,
) -> StoreResult<()> {
    for change_id in [Some(item.end_change_id), item.start_change_id]
        .into_iter()
        .flatten()
    {
        let change = db(SubscriptionChangeRow::get_by_id(conn, change_id, &tenant_id).await)?;
        if change.withdrawn_at.is_none() {
            crate::services::facts::pending::withdraw_pending_tx(conn, &change, reason).await?;
        }
    }
    db(SubscriptionTimelineItemRow::withdraw(conn, item.id).await)?;
    Ok(())
}

/// The live item `item_id` withdrawn with its changes still to come, started or not: what a
/// subscription that never started was given (an offer's add-on, R-174).
pub(crate) async fn withdraw_item_by_id_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    item_id: uuid::Uuid,
    reason: &str,
) -> StoreResult<()> {
    let items =
        db(SubscriptionTimelineItemRow::list_live(conn, &tenant_id, subscription_id).await)?;
    if let Some(item) = items.iter().find(|item| item.id == item_id) {
        withdraw_item_tx(conn, tenant_id, item, reason).await?;
    }
    Ok(())
}

/// The items whose end is still to come after `after` withdrawn, as the subscription's
/// termination does.
pub(crate) async fn withdraw_future_items_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    after: NaiveDate,
    reason: &str,
) -> StoreResult<()> {
    let items =
        db(SubscriptionTimelineItemRow::list_live(conn, &tenant_id, subscription_id).await)?;
    for item in items.iter().rev().filter(|item| item.until_date > after) {
        withdraw_item_tx(conn, tenant_id, item, reason).await?;
    }
    Ok(())
}

/// Whether `day` is a renewal: one that is not is refused, unless `prorated` lets it fall inside
/// a period, settled on its day (R-151).
async fn on_renewal(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    day: NaiveDate,
    what: &str,
    prorated: bool,
) -> StoreResult<bool> {
    let loaded = crate::services::evaluate::load::load_facts(
        conn,
        tenant_id,
        subscription_id,
        common_clock::now().date_naive(),
        None,
    )
    .await?;
    let renewal = billing_core::evaluate::renewal_on_or_after(
        &loaded.facts,
        day,
        day.and_time(NaiveTime::MIN).and_utc(),
    );
    if renewal == Some(day) {
        return Ok(true);
    }
    if prorated && renewal.is_some() {
        return Ok(false);
    }
    Err(invalid(format!(
        "{what} is on a renewal of the subscription: {day} is not one{}",
        renewal.map_or(String::new(), |next| format!(" (next: {next})"))
    )))
}

pub(in crate::services) async fn write_item_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    input: &TimelineItemInput,
) -> StoreResult<TimelineItem> {
    crate::services::subscriptions::phases::refuse_before_one_offs_tx(
        conn,
        tenant_id,
        subscription_id,
        input.start_date,
    )
    .await?;
    let today = common_clock::now().date_naive();
    if input.until <= input.start_date || input.until <= today {
        return Err(invalid(
            "a timeline item ends after it starts, and after today",
        ));
    }
    if input.quantity < 1 {
        return Err(invalid("a timeline item adds at least one add-on"));
    }
    let end_on_renewal = on_renewal(
        conn,
        tenant_id,
        subscription_id,
        input.until,
        "An item's end",
        input.terms.prorated,
    )
    .await?;
    let details = services
        .store
        .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
        .await?;
    let item_id = uuid::Uuid::now_v7();
    let facts =
        crate::services::evaluate::load::load_facts(conn, tenant_id, subscription_id, today, None)
            .await?
            .facts;

    let (lineage, start_change_id) = match input.existing {
        Some(existing) => {
            let row = db(SubscriptionAddOnRow::list_fact_rows(conn, subscription_id).await)?
                .into_iter()
                .find(|row| row.id == existing && row.effective_to.is_none())
                .ok_or_else(|| {
                    invalid(format!("no add-on {existing} on the subscription to end"))
                })?;
            if row.add_on_id != input.add_on_id {
                return Err(invalid("the add-on to end is not of the item's add-on"));
            }
            if row.effective_from > today {
                return Err(invalid(format!(
                    "{} starts on {}: end it once it has started",
                    row.name, row.effective_from
                )));
            }
            if end_on_renewal {
                crate::services::subscriptions::phases::on_fee_period(
                    &facts,
                    input.until,
                    &row.name,
                    row.period.clone().into(),
                )?;
            }
            (row.id, None)
        }
        None => {
            if input.start_date < today {
                return Err(invalid("a new timeline item starts today or later"));
            }
            let start_on_renewal = if input.start_date > today || !input.terms.free {
                on_renewal(
                    conn,
                    tenant_id,
                    subscription_id,
                    input.start_date,
                    "A paid item's start",
                    input.terms.prorated,
                )
                .await?
            } else {
                false
            };
            let mut insert = crate::services::subscriptions::amendment::resolve_added_add_on(
                conn,
                &details,
                input.add_on_id,
                input.quantity,
                input.start_date,
            )
            .await?;
            for day in [
                (input.start_date, start_on_renewal),
                (input.until, end_on_renewal),
            ]
            .into_iter()
            .filter(|(_, on_renewal)| *on_renewal)
            .map(|(day, _)| day)
            {
                crate::services::subscriptions::phases::on_fee_period(
                    &facts,
                    day,
                    &insert.name,
                    insert.period,
                )?;
            }
            if input.terms.free {
                insert.fee = repriced(&insert.fee, rust_decimal::Decimal::ZERO)
                    .ok_or_else(|| invalid("a free item is an add-on priced by one amount"))?;
                insert.price_id = None;
            }
            let row_id = item_row_id(subscription_id, item_id, "start");
            insert.subscription_add_on_id = Some(row_id);
            insert.lineage_id = None;
            let start = schedule_tx(
                services,
                conn,
                actor,
                tenant_id,
                subscription_id,
                item_id,
                input.start_date,
                ChangeRows {
                    addon_insert: vec![insert],
                    ..Default::default()
                },
            )
            .await?;
            (row_id, Some(start))
        }
    };

    let mut convert: Vec<ResolvedAddOnInsert> = Vec::new();
    if let ItemEnd::Convert { price } = &input.terms.end {
        let mut paid = crate::services::subscriptions::amendment::resolve_added_add_on(
            conn,
            &details,
            input.add_on_id,
            input.quantity,
            input.until,
        )
        .await?;
        if let Some(price) = price {
            crate::services::subscriptions::phases::validate_price(*price)?;
            paid.fee = repriced(&paid.fee, *price)
                .ok_or_else(|| invalid("a converted item is an add-on priced by one amount"))?;
            paid.price_id = None;
        }
        paid.subscription_add_on_id = Some(item_row_id(subscription_id, item_id, "convert"));
        paid.lineage_id = Some(lineage);
        convert.push(paid);
    }
    let end = schedule_tx(
        services,
        conn,
        actor,
        tenant_id,
        subscription_id,
        item_id,
        input.until,
        ChangeRows {
            addon_close: vec![lineage],
            addon_insert: convert,
            ..Default::default()
        },
    )
    .await?;

    let terms = serde_json::to_value(&input.terms)
        .map_err(|error| Report::new(StoreError::SerdeError("item terms".into(), error)))?;
    let row = db(SubscriptionTimelineItemRow {
        id: item_id,
        tenant_id,
        subscription_id,
        kind: ITEM_ADD_ON.to_string(),
        add_on_id: Some(input.add_on_id),
        lineage_id: lineage.as_uuid(),
        start_date: input.start_date,
        until_date: input.until,
        terms,
        label: input.label.clone(),
        reason: input.reason.clone(),
        start_change_id,
        end_change_id: end,
        created_by: actor.as_uuid(),
        created_at: common_clock::now().naive_utc(),
        withdrawn_at: None,
    }
    .insert(conn)
    .await)?;
    TimelineItem::try_from(row)
}

/// What a timeline change closes and opens on its day.
#[derive(Default)]
pub(crate) struct ChangeRows {
    pub component_close: Vec<SubscriptionPriceComponentId>,
    pub component_insert: Vec<ResolvedComponentInsert>,
    pub addon_close: Vec<SubscriptionAddOnId>,
    pub addon_insert: Vec<ResolvedAddOnInsert>,
}

#[allow(clippy::too_many_arguments)]
pub(in crate::services) async fn schedule_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    item_id: uuid::Uuid,
    on: NaiveDate,
    rows: ChangeRows,
) -> StoreResult<ScheduledEventId> {
    schedule_as_tx(
        services,
        conn,
        actor,
        (tenant_id, subscription_id),
        item_id,
        on,
        rows,
        false,
    )
    .await
}

/// [`schedule_tx`] for an item's boundary written again over a change (R-162), through an ending
/// still to come.
#[allow(clippy::too_many_arguments)]
pub(in crate::services) async fn schedule_replayed_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    item_id: uuid::Uuid,
    on: NaiveDate,
    rows: ChangeRows,
) -> StoreResult<ScheduledEventId> {
    schedule_as_tx(
        services,
        conn,
        actor,
        (tenant_id, subscription_id),
        item_id,
        on,
        rows,
        true,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn schedule_as_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    (tenant_id, subscription_id): (TenantId, SubscriptionId),
    item_id: uuid::Uuid,
    on: NaiveDate,
    rows: ChangeRows,
    replay: bool,
) -> StoreResult<ScheduledEventId> {
    let today = common_clock::now().date_naive();
    let loaded = crate::services::subscriptions::phases::facts_through_ending(
        conn,
        tenant_id,
        subscription_id,
        replay,
    )
    .await?;
    let inside = on > today
        && billing_core::evaluate::renewal_on_or_after(
            &loaded.facts,
            on,
            on.and_time(NaiveTime::MIN).and_utc(),
        ) != Some(on);
    let change = ScheduledEventNew {
        subscription_id,
        tenant_id,
        scheduled_time: on.and_time(NaiveTime::MIN),
        event_data: ScheduledEventData::ApplyPhase {
            owner_id: item_id,
            component_close: rows.component_close,
            component_insert: rows.component_insert,
            addon_close: rows.addon_close,
            addon_insert: rows.addon_insert,
            minimum: Default::default(),
        },
        source: "timeline_item".to_string(),
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
    if inside {
        db(SubscriptionChangeRow::mark_settlement_due_on(
            conn,
            tenant_id,
            subscription_id,
            on,
            diesel_models::enums::SubscriptionChangeKindEnum::Phase,
        )
        .await)?;
    }
    Ok(event.id)
}

fn discount_row_id(
    subscription_id: SubscriptionId,
    item: uuid::Uuid,
    which: &str,
) -> SubscriptionPriceComponentId {
    SubscriptionPriceComponentId::from(uuid::Uuid::new_v5(
        &subscription_id.as_uuid(),
        format!("discount:{item}:{which}").as_bytes(),
    ))
}

/// The fee's row from `start` at `percent` off, then its row in force before from `until`.
/// Nothing else may change the fee meanwhile: its only row reaching past today is the one in
/// force, open-ended, and the guards keep it so until the discount ends (R-149).
pub(in crate::services) async fn write_discount_tx(
    services: &Services,
    conn: &mut PgConn,
    actor: &Actor,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    input: &DatedDiscountInput,
) -> StoreResult<DatedDiscount> {
    crate::services::subscriptions::phases::refuse_before_one_offs_tx(
        conn,
        tenant_id,
        subscription_id,
        input.start_date,
    )
    .await?;
    let today = common_clock::now().date_naive();
    if input.until <= input.start_date || input.start_date < today {
        return Err(invalid(
            "a discount starts today or later and ends after it starts",
        ));
    }
    let hundred = rust_decimal::Decimal::ONE_HUNDRED;
    if input.percent <= rust_decimal::Decimal::ZERO || input.percent > hundred {
        return Err(invalid("a discount is more than 0% and at most 100% off"));
    }
    let start_on_renewal = on_renewal(
        conn,
        tenant_id,
        subscription_id,
        input.start_date,
        "A discount's start",
        input.prorated,
    )
    .await?;
    let end_on_renewal = on_renewal(
        conn,
        tenant_id,
        subscription_id,
        input.until,
        "A discount's end",
        input.prorated,
    )
    .await?;

    let rows = db(SubscriptionComponentRow::list_fact_rows(conn, subscription_id).await)?;
    let named = rows
        .iter()
        .find(|row| row.id == input.component)
        .ok_or_else(|| {
            invalid(format!(
                "no component {} on the subscription",
                input.component
            ))
        })?;
    let lineage = named.lineage_id.unwrap_or(named.id);
    let ahead: Vec<&SubscriptionComponentRow> = rows
        .iter()
        .filter(|row| row.lineage_id.unwrap_or(row.id) == lineage)
        .filter(|row| row.effective_to.is_none_or(|to| to > today))
        .collect();
    let current = match ahead.as_slice() {
        [row] if row.effective_to.is_none() && row.effective_from <= today => *row,
        [] => return Err(invalid(format!("{} has ended", named.name))),
        _ => {
            return Err(invalid(format!(
                "{} has a change still to come: a discount starts from its price in force",
                named.name
            )));
        }
    };
    let fee: SubscriptionFee = crate::services::subscriptions::phases::RowFees::on(
        conn,
        tenant_id,
        &rows,
        input.start_date,
    )
    .await?
    .fee_of(current)?;
    let price = price_of(&fee).ok_or_else(|| {
        invalid(format!(
            "a discount prices rates, seats and capacities, not {}",
            current.name
        ))
    })?;
    let facts =
        crate::services::evaluate::load::load_facts(conn, tenant_id, subscription_id, today, None)
            .await?
            .facts;
    for day in [
        (input.start_date, start_on_renewal),
        (input.until, end_on_renewal),
    ]
    .into_iter()
    .filter(|(_, on_renewal)| *on_renewal)
    .map(|(day, _)| day)
    {
        crate::services::subscriptions::phases::on_fee_period(
            &facts,
            day,
            &current.name,
            current.period.clone().into(),
        )?;
    }
    let discounted = repriced(
        &fee,
        (price * (hundred - input.percent) / hundred).round_dp(8),
    )
    .ok_or_else(|| invalid(format!("{} has no price to discount", current.name)))?;

    let item_id = uuid::Uuid::now_v7();
    let row_of = |fee: SubscriptionFee, price_id, which| ResolvedComponentInsert {
        price_component_id: current.price_component_id,
        product_id: current.product_id,
        name: current.name.clone(),
        period: current.period.clone().into(),
        fee,
        is_override: true,
        price_id,
        lineage_id: Some(lineage),
        subscription_component_id: Some(discount_row_id(subscription_id, item_id, which)),
    };
    let start = schedule_tx(
        services,
        conn,
        actor,
        tenant_id,
        subscription_id,
        item_id,
        input.start_date,
        ChangeRows {
            component_close: vec![current.id],
            component_insert: vec![row_of(discounted, None, "start")],
            ..Default::default()
        },
    )
    .await?;
    let end = schedule_tx(
        services,
        conn,
        actor,
        tenant_id,
        subscription_id,
        item_id,
        input.until,
        ChangeRows {
            component_close: vec![discount_row_id(subscription_id, item_id, "start")],
            component_insert: vec![row_of(fee, current.price_id, "end")],
            ..Default::default()
        },
    )
    .await?;

    let terms = serde_json::to_value(DiscountTerms {
        percent: input.percent,
        name: Some(current.name.clone()),
        prorated: input.prorated,
    })
    .map_err(|error| Report::new(StoreError::SerdeError("discount terms".into(), error)))?;
    let row = db(SubscriptionTimelineItemRow {
        id: item_id,
        tenant_id,
        subscription_id,
        kind: ITEM_DISCOUNT.to_string(),
        add_on_id: None,
        lineage_id: lineage.as_uuid(),
        start_date: input.start_date,
        until_date: input.until,
        terms,
        label: input.label.clone(),
        reason: input.reason.clone(),
        start_change_id: Some(start),
        end_change_id: end,
        created_by: actor.as_uuid(),
        created_at: common_clock::now().naive_utc(),
        withdrawn_at: None,
    }
    .insert(conn)
    .await)?;
    DatedDiscount::try_from(row)
}
