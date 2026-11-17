//! A subscription recorded as dated facts (Phase 7): each path that acts records what it decided,
//! next to the events and state that still drive billing. Nothing bills from them yet. Scheduled
//! cancellations and invoice deadlines are recorded with their events (`record_scheduled_facts`).
use chrono::{Days, NaiveDate, NaiveDateTime};
use common_domain::actor::Actor;
use common_domain::ids::{SubscriptionId, TenantId};
use diesel_models::enums::BillingPeriodEnum as DbBillingPeriod;
use diesel_models::enums::SubscriptionActivationConditionEnum;
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscription_facts::{
    ENDING_COMPLETE, StartSettledBy, SubscriptionEndingRow, SubscriptionPlanWindowRow,
    SubscriptionPlanWindowRowNew, SubscriptionStartSettledRow, SubscriptionTrialRow,
};
use diesel_models::subscriptions::{SubscriptionRow, SubscriptionRowNew};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

use crate::StoreResult;
use crate::domain::enums::BillingPeriodEnum;
use crate::errors::StoreError;
use crate::services::Services;
use crate::store::PgConn;

fn months(cadence: &DbBillingPeriod) -> u32 {
    BillingPeriodEnum::from(cadence.clone()).as_months()
}

fn db<T>(result: diesel_models::DbResult<T>) -> StoreResult<T> {
    result.map_err(Into::<Report<StoreError>>::into)
}

/// The facts of subscriptions just inserted: their first plan window, trial, a start that needs no
/// checkout, a fixed term. `trial_is_free` is the plan version's, in force at the start.
pub(crate) async fn record_inserted_tx(
    conn: &mut PgConn,
    actor: &Actor,
    inserted: &[(&SubscriptionRowNew, bool)],
) -> StoreResult<()> {
    let now = common_domain::clock::now().naive_utc();
    let mut windows = Vec::new();
    let mut trials = Vec::new();
    let mut starts = Vec::new();
    let mut endings = Vec::new();
    for (row, trial_is_free) in inserted {
        let billing_start = row.billing_start_date.unwrap_or(row.start_date);
        let trial = match row
            .trial_duration
            .and_then(|days| u64::try_from(days).ok())
            .filter(|days| *days > 0)
        {
            Some(days) => Some(billing_start.checked_add_days(Days::new(days)).ok_or_else(
                || StoreError::InvalidArgument(format!("a trial of {days} days is out of range")),
            )?),
            None => None,
        };
        let billing_from = match trial {
            Some(until) if *trial_is_free => until,
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
        if let Some(until) = trial {
            trials.push(SubscriptionTrialRow {
                subscription_id: row.id,
                tenant_id: row.tenant_id,
                from_date: billing_start,
                until_date: until,
                free: *trial_is_free,
                created_at: now,
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
            });
        }
    }
    db(SubscriptionPlanWindowRow::insert_batch(conn, &windows).await)?;
    db(SubscriptionTrialRow::insert_batch(conn, &trials).await)?;
    db(SubscriptionStartSettledRow::insert_batch(conn, &starts).await)?;
    db(SubscriptionEndingRow::insert_batch(conn, &endings).await)
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
            created_at: common_domain::clock::now().naive_utc(),
        }],
    )
    .await)
}

/// A plan change applied on `on`, read from the subscription it left: its plan window from then,
/// and the trial it ends. `starts_billing`: it ended a free trial, billing starting on `on`.
pub(crate) async fn record_plan_change_tx(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    on: NaiveDate,
    starts_billing: bool,
) -> StoreResult<()> {
    let sub = db(SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id).await)?
        .subscription;
    db(SubscriptionTrialRow::end_by(conn, subscription_id, on).await)?;
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
    let origin = if starts_billing {
        from
    } else {
        decided_origin(conn, &sub, &windows, from, &sub.period).await?
    };
    db(SubscriptionPlanWindowRow::open(
        conn,
        SubscriptionPlanWindowRowNew {
            id: uuid::Uuid::now_v7(),
            tenant_id,
            subscription_id,
            plan_version_id: sub.plan_version_id,
            from_date: from,
            cadence: sub.period.clone(),
            cadence_origin: origin,
        },
    )
    .await)?;
    Ok(())
}

/// The subscription moved onto `period` from `on` as if created with the fees it bills then: its
/// grid keeps its origin.
pub(crate) async fn record_cadence_change_tx(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    on: NaiveDate,
    period: DbBillingPeriod,
) -> StoreResult<()> {
    let Some(open) = db(SubscriptionPlanWindowRow::open_of(conn, subscription_id).await)? else {
        return Ok(());
    };
    db(SubscriptionPlanWindowRow::open(
        conn,
        SubscriptionPlanWindowRowNew {
            id: uuid::Uuid::now_v7(),
            tenant_id: open.tenant_id,
            subscription_id,
            plan_version_id: open.plan_version_id,
            from_date: on,
            cadence: period,
            cadence_origin: open.cadence_origin,
        },
    )
    .await)?;
    Ok(())
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
    if let Some(previous) = before.last()
        && months(&previous.cadence) == months(cadence)
    {
        return Ok(previous.cadence_origin);
    }
    if let Some(recorded) = db(SubscriptionEventRow::cadence_origin(conn, sub.id, on).await)? {
        return Ok(recorded);
    }
    let billing_start = sub.billing_start_date.unwrap_or(sub.current_period_start);
    let billing_from = match db(SubscriptionTrialRow::find(conn, sub.id).await)? {
        Some(trial) if trial.free => trial.until_date.max(billing_start),
        _ => billing_start,
    };
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
            self.store
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
                .await?;
        }
        let mut conn = self.store.get_conn().await?;
        db(SubscriptionPlanWindowRow::count_pending_origins(&mut conn).await)
    }
}
