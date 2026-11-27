//! A subscription recorded as dated facts: each path that acts records what it decided, and the
//! controller derives the state and the documents due from them.
use chrono::{Days, NaiveDate, NaiveDateTime};
use common_domain::actor::Actor;
use common_domain::ids::{SubscriptionId, TenantId};
use diesel_models::enums::BillingPeriodEnum as DbBillingPeriod;
use diesel_models::enums::SubscriptionActivationConditionEnum;
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscription_facts::{
    ENDING_CANCEL, ENDING_COMPLETE, StartSettledBy, SubscriptionEndingRow,
    SubscriptionPlanWindowRow, SubscriptionPlanWindowRowNew, SubscriptionStartSettledRow,
    SubscriptionTrialRow,
};
use diesel_models::subscriptions::{SubscriptionRow, SubscriptionRowNew};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

use crate::StoreResult;
use crate::domain::enums::BillingPeriodEnum;
use crate::errors::StoreError;
use crate::repositories::subscriptions::usage_attribution::{self, FeeRowId};
use crate::services::Services;
use crate::store::PgConn;
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_components::SubscriptionComponentRow;

pub mod pending;

fn months(cadence: &DbBillingPeriod) -> u32 {
    BillingPeriodEnum::from(cadence.clone()).as_months()
}

fn db<T>(result: diesel_models::DbResult<T>) -> StoreResult<T> {
    result.map_err(Into::<Report<StoreError>>::into)
}

/// Ends the fee rows `ids` on `on`, one a change written for a later date ends later included
/// (R-106). A row ended by then would leave two rows of one fee in force: an invariant error.
pub(crate) async fn close_components_tx(
    conn: &mut PgConn,
    ids: &[common_domain::ids::SubscriptionPriceComponentId],
    on: NaiveDate,
) -> StoreResult<()> {
    use diesel_models::subscription_components::SubscriptionComponentRow;
    let closed = db(SubscriptionComponentRow::close_components(conn, ids, on).await)?;
    all_closed("component", ids, closed)
}

/// [`close_components_tx`] for add-on rows.
pub(crate) async fn close_add_ons_tx(
    conn: &mut PgConn,
    ids: &[common_domain::ids::SubscriptionAddOnId],
    on: NaiveDate,
) -> StoreResult<()> {
    use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
    let closed = db(SubscriptionAddOnRow::close_addons(conn, ids, on).await)?;
    all_closed("add-on", ids, closed)
}

fn all_closed<T: Eq + std::hash::Hash + std::fmt::Debug>(
    kind: &str,
    ids: &[T],
    closed: usize,
) -> StoreResult<()> {
    let expected = ids.iter().collect::<std::collections::HashSet<_>>().len();
    if closed == expected {
        return Ok(());
    }
    Err(Report::new(StoreError::BillingInvariantViolated(format!(
        "{closed} of the {expected} {kind} rows to close were open: {ids:?}"
    ))))
}

/// The facts of subscriptions just inserted: their first plan window, trial (as resolved, R-153), a
/// start that needs no checkout, a fixed term.
pub(crate) async fn record_inserted_tx(
    conn: &mut PgConn,
    actor: &Actor,
    inserted: &[(
        &SubscriptionRowNew,
        Option<&crate::domain::trials::ResolvedTrial>,
    )],
) -> StoreResult<()> {
    let now = common_clock::now().naive_utc();
    let mut windows = Vec::new();
    let mut trials = Vec::new();
    let mut starts = Vec::new();
    let mut endings = Vec::new();
    for (row, trial) in inserted {
        let billing_start = row.billing_start_date.unwrap_or(row.start_date);
        let until = match trial {
            Some(trial) => Some(
                billing_start
                    .checked_add_days(Days::new(u64::from(trial.days)))
                    .ok_or_else(|| {
                        StoreError::InvalidArgument(format!(
                            "a trial of {} days is out of range",
                            trial.days
                        ))
                    })?,
            ),
            None => None,
        };
        let billing_from = match (trial, until) {
            (Some(trial), Some(until)) if trial.free => until,
            _ => billing_start,
        };
        windows.push(SubscriptionPlanWindowRowNew {
            id: uuid::Uuid::now_v7(),
            tenant_id: row.tenant_id,
            subscription_id: row.id,
            plan_version_id: row.plan_version_id,
            from_date: row.start_date,
            cadence: row.period.clone(),
            cadence_origin: billing_from,
        });
        if let (Some(trial), Some(until)) = (trial, until) {
            trials.push(SubscriptionTrialRow {
                subscription_id: row.id,
                tenant_id: row.tenant_id,
                from_date: billing_start,
                until_date: until,
                free: trial.free,
                created_at: now,
                trial_plan_id: trial.trial_plan_id,
                trial_offer_id: trial.offer,
                end_action: trial.end_action.as_str().to_owned(),
            });
        }
        // An On Start subscription's billing starts on its start date, known at insert even when
        // that date is ahead and its activation comes with the cycle.
        let settled = row.activated_at.or_else(|| {
            (row.activation_condition == SubscriptionActivationConditionEnum::OnStart)
                .then(|| row.start_date.and_time(chrono::NaiveTime::MIN))
        });
        if let Some(at) = settled {
            let by = if row.quote_id.is_some() {
                StartSettledBy::Quote
            } else {
                StartSettledBy::Start
            };
            starts.push(SubscriptionStartSettledRow {
                subscription_id: row.id,
                tenant_id: row.tenant_id,
                at,
                settled_by: by.as_str().to_owned(),
                created_at: now,
            });
        }
        if let Some(end_date) = row.end_date {
            endings.push(SubscriptionEndingRow {
                subscription_id: row.id,
                tenant_id: row.tenant_id,
                kind: ENDING_COMPLETE.to_owned(),
                at: end_date.and_time(chrono::NaiveTime::MIN),
                reason: None,
                actor_type: actor.actor_type().into(),
                actor_uuid: actor.as_uuid(),
                actor_alias: actor.actor_alias(),
                created_at: now,
                requested_at: now,
            });
        }
    }
    db(SubscriptionPlanWindowRow::insert_batch(conn, &windows).await)?;
    db(SubscriptionTrialRow::insert_batch(conn, &trials).await)?;
    db(SubscriptionStartSettledRow::insert_batch(conn, &starts).await)?;
    db(SubscriptionEndingRow::insert_batch(conn, &endings).await)
}

/// The subscription is cancelled at `at`, replacing a cancellation recorded before: the instant of
/// an immediate cancellation, else its date's midnight. The controller applies it then.
/// `requested_at` is when it was decided: after `at`'s day, it is backdated (R-016).
pub(crate) async fn record_cancellation_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    at: NaiveDateTime,
    requested_at: NaiveDateTime,
    reason: Option<String>,
    actor: &Actor,
) -> StoreResult<()> {
    let recorded = cancellation_at(conn, subscription_id).await?;
    let moved_from = recorded.map_or(at, |(_, before)| before.min(at));
    if recorded.is_none_or(|(_, before)| before != at) {
        usage_attribution::ensure_unfinalized_move(
            conn,
            tenant_id,
            subscription_id,
            moved_from,
            "This cancellation",
        )
        .await?;
    }
    if recorded.is_some_and(|(_, before)| at > before) {
        usage_attribution::ensure_still_unsold(conn, tenant_id, subscription_id, Some(at)).await?;
    }
    if recorded.is_none_or(|(_, before)| before != at) {
        usage_attribution::claims_moved(conn, tenant_id, subscription_id).await?;
    }
    db(SubscriptionEndingRow::record_cancellation(
        conn,
        &SubscriptionEndingRow {
            subscription_id,
            tenant_id,
            kind: ENDING_CANCEL.to_owned(),
            at,
            reason,
            actor_type: actor.actor_type().into(),
            actor_uuid: actor.as_uuid(),
            actor_alias: actor.actor_alias(),
            created_at: common_clock::now().naive_utc(),
            requested_at,
        },
    )
    .await)
}

/// The subscription's fixed term moved to `end_date` (its midnight), or withdrawn with `None`: it
/// completes then, or renews until cancelled (R-140). Its `end_date` column is set beside it. The
/// usage its claim gains or gives back moves as a cancellation's does (R-143).
pub(crate) async fn record_end_date_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    end_date: Option<NaiveDate>,
    actor: &Actor,
) -> StoreResult<()> {
    let before =
        db(SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id).await)?
            .subscription
            .end_date;
    let cancelled = cancellation_at(conn, subscription_id)
        .await?
        .map(|(_, at)| at);
    let claim_end = |date: Option<NaiveDate>| {
        cancelled
            .into_iter()
            .chain(date.map(usage_attribution::midnight))
            .min()
    };
    let (was, will) = (claim_end(before), claim_end(end_date));
    if let Some(moved_from) = was.into_iter().chain(will).min().filter(|_| was != will) {
        usage_attribution::ensure_unfinalized_move(
            conn,
            tenant_id,
            subscription_id,
            moved_from,
            "This end date",
        )
        .await?;
    }
    let now = common_clock::now().naive_utc();
    let row = end_date.map(|date| SubscriptionEndingRow {
        subscription_id,
        tenant_id,
        kind: ENDING_COMPLETE.to_owned(),
        at: date.and_time(chrono::NaiveTime::MIN),
        reason: None,
        actor_type: actor.actor_type().into(),
        actor_uuid: actor.as_uuid(),
        actor_alias: actor.actor_alias(),
        created_at: now,
        requested_at: now,
    });
    db(SubscriptionEndingRow::record_completion(conn, subscription_id, row.as_ref()).await)?;
    db(SubscriptionRow::set_end_date(conn, tenant_id, subscription_id, end_date).await)?;
    if was != will {
        // Read with the end date just set: a longer term must not oversell its resource.
        if will.is_none_or(|will| was.is_some_and(|was| will > was)) {
            usage_attribution::ensure_still_unsold(conn, tenant_id, subscription_id, cancelled)
                .await?;
        }
        usage_attribution::claims_moved(conn, tenant_id, subscription_id).await?;
    }
    Ok(())
}

/// The subscription's cancellation withdrawn before it applied, its `Cancelled` event with it: a
/// revoked cancellation books no churn (R-141). Its usage from the cancellation on comes back to
/// it: refused once another subscription finalized it (R-143).
pub(crate) async fn withdraw_cancellation_tx(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
) -> StoreResult<()> {
    if let Some((tenant_id, at)) = cancellation_at(conn, subscription_id).await? {
        usage_attribution::ensure_unfinalized_move(
            conn,
            tenant_id,
            subscription_id,
            at,
            "Withdrawing this cancellation",
        )
        .await?;
        usage_attribution::ensure_still_unsold(conn, tenant_id, subscription_id, None).await?;
        usage_attribution::claims_moved(conn, tenant_id, subscription_id).await?;
    }
    db(SubscriptionEventRow::withdraw_pending_cancellation(conn, subscription_id).await)?;
    db(SubscriptionEndingRow::withdraw_cancellation(conn, subscription_id).await)
}

/// The cancellation recorded for the subscription, with its tenant.
async fn cancellation_at(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
) -> StoreResult<Option<(TenantId, NaiveDateTime)>> {
    use diesel::{ExpressionMethods, OptionalExtension, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::errors::IntoDbResult;
    use diesel_models::schema::subscription_ending::dsl as e;
    db(e::subscription_ending
        .filter(e::subscription_id.eq(subscription_id))
        .filter(e::kind.eq(ENDING_CANCEL))
        .select((e::tenant_id, e::at))
        .first(conn)
        .await
        .optional()
        .into_db_result())
}

/// From `usage_from` on, the rows `rows` bill their metric: a late change wrote them over usage
/// another subscription of the customer invoiced (R-143).
pub(crate) async fn record_usage_floor_tx(
    conn: &mut PgConn,
    rows: &[FeeRowId],
    usage_from: NaiveDateTime,
) -> StoreResult<()> {
    let components = rows
        .iter()
        .filter_map(|row| match row {
            FeeRowId::Component(id) => Some(*id),
            FeeRowId::AddOn(_) => None,
        })
        .collect::<Vec<_>>();
    let add_ons = rows
        .iter()
        .filter_map(|row| match row {
            FeeRowId::AddOn(id) => Some(*id),
            FeeRowId::Component(_) => None,
        })
        .collect::<Vec<_>>();
    let at = usage_from.and_utc();
    db(SubscriptionComponentRow::set_usage_from(conn, &components, at).await)?;
    db(SubscriptionAddOnRow::set_usage_from(conn, &add_ons, at).await)?;
    Ok(())
}

/// The start settled at `at` (a checkout, a payment, an activation); the first one is kept.
pub(crate) async fn settle_start_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    at: NaiveDateTime,
    by: StartSettledBy,
) -> StoreResult<()> {
    db(SubscriptionStartSettledRow::insert_batch(
        conn,
        &[SubscriptionStartSettledRow {
            subscription_id,
            tenant_id,
            at,
            settled_by: by.as_str().to_owned(),
            created_at: common_clock::now().naive_utc(),
        }],
    )
    .await)
}

/// A running trial moved to end on `until`: its end, and a free trial's billing origin with it,
/// on the one plan window it started with (a plan change since ended the trial already).
pub(crate) async fn record_trial_end_tx(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    trial: &SubscriptionTrialRow,
    until: NaiveDate,
) -> StoreResult<()> {
    if trial.free {
        let windows =
            db(SubscriptionPlanWindowRow::list_by_subscription(conn, subscription_id).await)?;
        let [window] = windows.as_slice() else {
            return Err(Report::new(StoreError::InvalidArgument(
                "the subscription changed plan since its trial started".to_string(),
            )));
        };
        if window.cadence_origin != trial.until_date {
            return Err(Report::new(StoreError::InvalidArgument(
                "the subscription's billing no longer starts on its trial's end".to_string(),
            )));
        }
        db(
            SubscriptionPlanWindowRow::set_origin(conn, window.id, until.max(trial.from_date))
                .await,
        )?;
    }
    if until <= trial.from_date {
        db(SubscriptionTrialRow::remove(conn, subscription_id).await)?;
    } else {
        db(SubscriptionTrialRow::move_end(conn, subscription_id, until).await)?;
    }
    Ok(())
}

/// A plan change applied on `on` to `plan_version_id` on `cadence`: its plan window from then, and
/// the trial it ends. `starts_billing`: it ended a free trial, billing starting on `on`. Returns
/// the cadence it renews on (`cadence_of_change`).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn record_plan_change_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    on: NaiveDate,
    plan_version_id: common_domain::ids::PlanVersionId,
    cadence: DbBillingPeriod,
    starts_billing: bool,
) -> StoreResult<DbBillingPeriod> {
    let sub = db(SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id).await)?
        .subscription;
    let as_created = starts_billing || on == billing_from(conn, &sub).await?;
    db(SubscriptionTrialRow::end_by(conn, subscription_id, on).await)?;
    let (windows, from) = windows_before(conn, subscription_id, on).await?;
    let origin = if starts_billing {
        from
    } else {
        decided_origin(conn, &sub, &windows, from, &cadence).await?
    };
    let cadence = cadence_of_change(conn, subscription_id, on, cadence, as_created, &[]).await?;
    db(SubscriptionPlanWindowRow::open(
        conn,
        SubscriptionPlanWindowRowNew {
            id: uuid::Uuid::now_v7(),
            tenant_id,
            subscription_id,
            plan_version_id,
            from_date: from,
            cadence: cadence.clone(),
            cadence_origin: origin,
        },
    )
    .await)?;
    Ok(cadence)
}

/// The cadence a plan change on `on` renews on: its plan's `cadence`, or, when it leaves the
/// subscription `as_created` on the new plan, the shortest of it and of the add-ons it keeps (all
/// but `closed`), as at creation (R-084). The grid's origin stays the one the plan's cadence reads.
pub(crate) async fn cadence_of_change(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    on: NaiveDate,
    cadence: DbBillingPeriod,
    as_created: bool,
    closed: &[common_domain::ids::SubscriptionAddOnId],
) -> StoreResult<DbBillingPeriod> {
    use crate::domain::enums::SubscriptionFeeBillingPeriod;
    use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
    if !as_created {
        return Ok(cadence);
    }
    let kept = db(SubscriptionAddOnRow::list_fact_rows(conn, subscription_id).await)?
        .into_iter()
        .filter(|row| row.effective_from <= on && row.effective_to.is_none_or(|to| on < to))
        .filter(|row| !closed.contains(&row.id))
        .filter_map(|row| SubscriptionFeeBillingPeriod::from(row.period).as_billing_period_opt())
        .map(Into::<DbBillingPeriod>::into);
    Ok(kept.fold(cadence, |shortest, period| {
        if months(&period) < months(&shortest) {
            period
        } else {
            shortest
        }
    }))
}

/// The day the subscription bills from: its billing start, or a free trial's end after it.
pub(crate) async fn billing_from(
    conn: &mut PgConn,
    sub: &SubscriptionRow,
) -> StoreResult<NaiveDate> {
    let billing_start = sub.billing_start_date.unwrap_or(sub.current_period_start);
    Ok(match db(SubscriptionTrialRow::find(conn, sub.id).await)? {
        Some(trial) if trial.free => trial.until_date.max(billing_start),
        _ => billing_start,
    })
}

/// The windows a plan change on `on` follows, and the day the window it opens starts: a window
/// opened on or after `on` is replaced.
async fn windows_before(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    on: NaiveDate,
) -> StoreResult<(Vec<SubscriptionPlanWindowRow>, NaiveDate)> {
    let mut windows =
        db(SubscriptionPlanWindowRow::list_by_subscription(conn, subscription_id).await)?;
    let from = match windows.last() {
        Some(open) if open.from_date >= on => {
            let from = open.from_date;
            windows.pop();
            from
        }
        _ => on,
    };
    Ok((windows, from))
}

/// Where the grid of a window opening on `on` with `cadence` after `before` starts: the grid in
/// force when the cadence stays, else its recorded origin or the rule's (R-084).
async fn decided_origin(
    conn: &mut PgConn,
    sub: &SubscriptionRow,
    before: &[SubscriptionPlanWindowRow],
    on: NaiveDate,
    cadence: &DbBillingPeriod,
) -> StoreResult<NaiveDate> {
    let billing_from = billing_from(conn, sub).await?;
    // A window opening on the billing start, as a change replacing the one that ended a free
    // trial that day: its grid starts there, whatever the trial's window ran on.
    if on == billing_from {
        return Ok(on);
    }
    if let Some(previous) = before.last()
        && months(&previous.cadence) == months(cadence)
    {
        return Ok(previous.cadence_origin);
    }
    if let Some(recorded) = db(SubscriptionEventRow::cadence_origin(conn, sub.id, on).await)? {
        return Ok(recorded);
    }
    Ok(origin_by_rule(
        billing_from,
        u32::from(sub.billing_day_anchor.unsigned_abs()),
        before,
        on,
        months(cadence),
    ))
}

/// The rule's origin for a change of cadence on `on`, the windows before it keeping theirs.
fn origin_by_rule(
    billing_from: NaiveDate,
    anchor: u32,
    before: &[SubscriptionPlanWindowRow],
    on: NaiveDate,
    after: u32,
) -> NaiveDate {
    let mut switches: Vec<billing_core::timeline::CadenceSwitch> = before
        .windows(2)
        .filter(|pair| months(&pair[0].cadence) != months(&pair[1].cadence))
        .map(|pair| billing_core::timeline::CadenceSwitch {
            on: pair[1].from_date,
            before: months(&pair[0].cadence),
            after: months(&pair[1].cadence),
            recorded: Some(pair[1].cadence_origin),
        })
        .collect();
    switches.push(billing_core::timeline::CadenceSwitch {
        on,
        before: before.last().map_or(after, |w| months(&w.cadence)),
        after,
        recorded: None,
    });
    billing_core::timeline::cadence_origins(billing_from, anchor, &switches)
        .into_iter()
        .rev()
        .find(|(switched_on, _)| *switched_on == on)
        .map_or(on, |(_, origin)| origin)
}

/// Subscriptions decided per run of the sweep.
const PENDING_ORIGINS_PER_RUN: i64 = 500;

impl Services {
    /// Decides the origins the migration could not (a change of cadence that recorded none), by
    /// the rule, a switch's recorded origin first. Returns how many windows are left.
    pub(crate) async fn decide_pending_window_origins(&self) -> StoreResult<i64> {
        let subscriptions = {
            let mut conn = self.store.get_conn().await?;
            db(
                SubscriptionPlanWindowRow::with_pending_origins(&mut conn, PENDING_ORIGINS_PER_RUN)
                    .await,
            )?
        };
        for subscription_id in subscriptions {
            let decided = self
                .store
                .transaction(|conn| {
                    async move {
                        let lock = SubscriptionRow::lock_customer_of(conn, subscription_id).await?;
                        diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
                        let sub = db(SubscriptionRow::get_subscription_by_id(
                            conn,
                            &lock.tenant_id(),
                            subscription_id,
                        )
                        .await)?
                        .subscription;
                        let pending = db(SubscriptionPlanWindowRow::pending_origins_of(
                            conn,
                            subscription_id,
                        )
                        .await)?;
                        let mut windows = db(SubscriptionPlanWindowRow::list_by_subscription(
                            conn,
                            subscription_id,
                        )
                        .await)?;
                        for index in 0..windows.len() {
                            if !pending.contains(&windows[index].id) {
                                continue;
                            }
                            let window = windows[index].clone();
                            let origin = decided_origin(
                                conn,
                                &sub,
                                &windows[..index],
                                window.from_date,
                                &window.cadence,
                            )
                            .await?;
                            db(
                                SubscriptionPlanWindowRow::set_origin(conn, window.id, origin)
                                    .await,
                            )?;
                            windows[index].cadence_origin = origin;
                        }
                        db(SubscriptionPlanWindowRow::origins_decided(conn, &pending).await)
                    }
                    .scope_boxed()
                })
                .await;
            // One undecidable subscription is left pending, counted below; the others go on.
            if let Err(error) = decided {
                log::error!(
                    "cadence origin of subscription {subscription_id} not decided: {error:?}"
                );
            }
        }
        let mut conn = self.store.get_conn().await?;
        db(SubscriptionPlanWindowRow::count_pending_origins(&mut conn).await)
    }
}
