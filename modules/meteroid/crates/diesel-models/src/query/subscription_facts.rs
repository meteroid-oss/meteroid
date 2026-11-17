use crate::enums::ScheduledEventTypeEnum;
use crate::errors::IntoDbResult;
use crate::scheduled_events::ScheduledEventRow;
use crate::subscription_facts::{
    SubscriptionEndingRow, SubscriptionPlanWindowRow, SubscriptionPlanWindowRowNew,
    SubscriptionStartSettledRow, SubscriptionTrialRow,
};
use crate::{DbResult, PgConn};
use chrono::NaiveDate;
use common_domain::ids::{BaseId, SubscriptionId};
use diesel::sql_types::{Array, Date, Timestamp, Uuid as SqlUuid};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

/// The statuses of a subscription still running, inlined as SQL.
const LIVE_SQL: &str = "('PENDING_ACTIVATION', 'PENDING_CHARGE', 'TRIAL_ACTIVE', 'ACTIVE', \
     'TRIAL_EXPIRED', 'PAUSED', 'SUSPENDED')";

impl SubscriptionPlanWindowRow {
    /// Earliest first.
    pub async fn list_by_subscription(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
    ) -> DbResult<Vec<SubscriptionPlanWindowRow>> {
        use crate::schema::subscription_plan_window::dsl as w;

        let query = w::subscription_plan_window
            .filter(w::subscription_id.eq(subscription_id))
            .order_by(w::from_date.asc())
            .select(SubscriptionPlanWindowRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing plan windows")
            .into_db_result()
    }

    pub async fn open_of(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
    ) -> DbResult<Option<SubscriptionPlanWindowRow>> {
        use crate::schema::subscription_plan_window::dsl as w;

        let query = w::subscription_plan_window
            .filter(w::subscription_id.eq(subscription_id))
            .filter(w::to_date.is_null())
            .select(SubscriptionPlanWindowRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while reading the open plan window")
            .into_db_result()
    }

    /// Opens `window` from its date: the open window ends there, or gives way when it opened on
    /// or after it, the new one then opening where it did.
    pub async fn open(
        conn: &mut PgConn,
        mut window: SubscriptionPlanWindowRowNew,
    ) -> DbResult<SubscriptionPlanWindowRow> {
        use crate::schema::subscription_plan_window::dsl as w;

        if let Some(open) = Self::open_of(conn, window.subscription_id).await? {
            if open.from_date < window.from_date {
                diesel::update(w::subscription_plan_window)
                    .filter(w::id.eq(open.id))
                    .set(w::to_date.eq(Some(window.from_date)))
                    .execute(conn)
                    .await
                    .attach("Error while closing a plan window")
                    .into_db_result()?;
            } else {
                diesel::delete(w::subscription_plan_window)
                    .filter(w::id.eq(open.id))
                    .execute(conn)
                    .await
                    .attach("Error while replacing a plan window")
                    .into_db_result()?;
                window.from_date = open.from_date;
            }
        }

        let query = diesel::insert_into(w::subscription_plan_window)
            .values((
                &window,
                w::created_at.eq(common_domain::clock::now().naive_utc()),
            ))
            .returning(SubscriptionPlanWindowRow::as_returning());

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while opening a plan window")
            .into_db_result()
    }

    /// The first windows of subscriptions just inserted.
    pub async fn insert_batch(
        conn: &mut PgConn,
        windows: &[SubscriptionPlanWindowRowNew],
    ) -> DbResult<()> {
        use crate::schema::subscription_plan_window::dsl as w;

        if windows.is_empty() {
            return Ok(());
        }
        let now = common_domain::clock::now().naive_utc();
        diesel::insert_into(w::subscription_plan_window)
            .values(
                windows
                    .iter()
                    .map(|window| (window, w::created_at.eq(now)))
                    .collect::<Vec<_>>(),
            )
            .execute(conn)
            .await
            .attach("Error while recording plan windows")
            .map(|_| ())
            .into_db_result()
    }

    pub async fn set_origin(conn: &mut PgConn, id: Uuid, origin: NaiveDate) -> DbResult<()> {
        use crate::schema::subscription_plan_window::dsl as w;

        diesel::update(w::subscription_plan_window)
            .filter(w::id.eq(id))
            .set(w::cadence_origin.eq(origin))
            .execute(conn)
            .await
            .attach("Error while setting a plan window's origin")
            .map(|_| ())
            .into_db_result()
    }

    /// Subscriptions with a window whose origin the rule is still to decide, up to `limit`.
    pub async fn with_pending_origins(
        conn: &mut PgConn,
        limit: i64,
    ) -> DbResult<Vec<SubscriptionId>> {
        use crate::schema::subscription_plan_window::dsl as w;
        use crate::schema::subscription_plan_window_origin_pending::dsl as p;

        p::subscription_plan_window_origin_pending
            .inner_join(w::subscription_plan_window)
            .select(w::subscription_id)
            .distinct()
            .limit(limit)
            .load(conn)
            .await
            .attach("Error while listing plan windows of pending origin")
            .into_db_result()
    }

    pub async fn pending_origins_of(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
    ) -> DbResult<Vec<Uuid>> {
        use crate::schema::subscription_plan_window::dsl as w;
        use crate::schema::subscription_plan_window_origin_pending::dsl as p;

        p::subscription_plan_window_origin_pending
            .inner_join(w::subscription_plan_window)
            .filter(w::subscription_id.eq(subscription_id))
            .select(p::window_id)
            .load(conn)
            .await
            .attach("Error while reading plan windows of pending origin")
            .into_db_result()
    }

    pub async fn origins_decided(conn: &mut PgConn, ids: &[Uuid]) -> DbResult<()> {
        use crate::schema::subscription_plan_window_origin_pending::dsl as p;

        diesel::delete(p::subscription_plan_window_origin_pending)
            .filter(p::window_id.eq_any(ids))
            .execute(conn)
            .await
            .attach("Error while clearing decided plan window origins")
            .map(|_| ())
            .into_db_result()
    }

    pub async fn count_pending_origins(conn: &mut PgConn) -> DbResult<i64> {
        use crate::schema::subscription_plan_window_origin_pending::dsl as p;

        p::subscription_plan_window_origin_pending
            .count()
            .get_result(conn)
            .await
            .attach("Error while counting plan windows of pending origin")
            .into_db_result()
    }
}

/// The origin a renewal records late on the switch opening a window is that window's too.
pub(crate) async fn record_window_origin(
    conn: &mut PgConn,
    subscription_id: SubscriptionId,
    from_date: NaiveDate,
    origin: NaiveDate,
) -> DbResult<()> {
    diesel::sql_query(
        "WITH decided AS ( \
             UPDATE subscription_plan_window SET cadence_origin = $3 \
             WHERE subscription_id = $1 AND from_date = $2 RETURNING id) \
         DELETE FROM subscription_plan_window_origin_pending \
         WHERE window_id IN (SELECT id FROM decided)",
    )
    .bind::<SqlUuid, _>(subscription_id)
    .bind::<Date, _>(from_date)
    .bind::<Date, _>(origin)
    .execute(conn)
    .await
    .attach("Error while recording a plan window's origin")
    .map(|_| ())
    .into_db_result()
}

impl SubscriptionTrialRow {
    pub async fn insert_batch(conn: &mut PgConn, rows: &[SubscriptionTrialRow]) -> DbResult<()> {
        use crate::schema::subscription_trial::dsl as t;

        if rows.is_empty() {
            return Ok(());
        }
        diesel::insert_into(t::subscription_trial)
            .values(rows)
            .on_conflict_do_nothing()
            .execute(conn)
            .await
            .attach("Error while recording trials")
            .map(|_| ())
            .into_db_result()
    }

    pub async fn find(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
    ) -> DbResult<Option<SubscriptionTrialRow>> {
        use crate::schema::subscription_trial::dsl as t;

        t::subscription_trial
            .filter(t::subscription_id.eq(subscription_id))
            .select(SubscriptionTrialRow::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while reading a trial")
            .into_db_result()
    }

    /// A plan change on `on` ends the trial there, if it had not ended yet.
    pub async fn end_by(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        on: NaiveDate,
    ) -> DbResult<()> {
        diesel::sql_query(
            "UPDATE subscription_trial \
             SET until_date = GREATEST(from_date, LEAST(until_date, $2)) \
             WHERE subscription_id = $1 AND until_date > GREATEST(from_date, $2)",
        )
        .bind::<SqlUuid, _>(subscription_id)
        .bind::<Date, _>(on)
        .execute(conn)
        .await
        .attach("Error while ending a trial")
        .map(|_| ())
        .into_db_result()
    }
}

impl SubscriptionStartSettledRow {
    /// The first settlement of a start is the one recorded.
    pub async fn insert_batch(
        conn: &mut PgConn,
        rows: &[SubscriptionStartSettledRow],
    ) -> DbResult<()> {
        use crate::schema::subscription_start_settled::dsl as s;

        if rows.is_empty() {
            return Ok(());
        }
        diesel::insert_into(s::subscription_start_settled)
            .values(rows)
            .on_conflict_do_nothing()
            .execute(conn)
            .await
            .attach("Error while recording settled starts")
            .map(|_| ())
            .into_db_result()
    }
}

impl SubscriptionEndingRow {
    pub async fn insert_batch(conn: &mut PgConn, rows: &[SubscriptionEndingRow]) -> DbResult<()> {
        use crate::schema::subscription_ending::dsl as e;

        if rows.is_empty() {
            return Ok(());
        }
        diesel::insert_into(e::subscription_ending)
            .values(rows)
            .on_conflict_do_nothing()
            .execute(conn)
            .await
            .attach("Error while recording endings")
            .map(|_| ())
            .into_db_result()
    }

    /// Drops the cancellation of a live subscription once its cancellation events are cancelled.
    pub async fn withdraw_cancellation(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
    ) -> DbResult<()> {
        diesel::sql_query(format!(
            "DELETE FROM subscription_ending e \
             WHERE e.subscription_id = $1 AND e.kind = 'CANCEL' \
               AND EXISTS (SELECT 1 FROM subscription s WHERE s.id = $1 AND s.status IN {LIVE_SQL}) \
               AND NOT EXISTS (SELECT 1 FROM scheduled_event v \
                               WHERE v.subscription_id = $1 \
                                 AND v.event_type = 'CANCEL_SUBSCRIPTION' \
                                 AND v.status <> 'CANCELED')"
        ))
        .bind::<SqlUuid, _>(subscription_id)
        .execute(conn)
        .await
        .attach("Error while withdrawing a cancellation")
        .map(|_| ())
        .into_db_result()
    }
}

/// What newly scheduled events decide, recorded as facts: a cancellation's ending, an invoice's
/// finalization after its grace, its next payment attempt.
pub(crate) async fn record_scheduled_facts(
    conn: &mut PgConn,
    events: &[ScheduledEventRow],
) -> DbResult<()> {
    let of = |kind: ScheduledEventTypeEnum| -> Vec<Uuid> {
        events
            .iter()
            .filter(|e| e.event_type == kind)
            .map(|e| e.id.as_uuid())
            .collect()
    };
    let now = common_domain::clock::now().naive_utc();

    let cancellations = of(ScheduledEventTypeEnum::CancelSubscription);
    if !cancellations.is_empty() {
        diesel::sql_query(
            "INSERT INTO subscription_ending (subscription_id, tenant_id, kind, at, reason, \
                                              actor_type, actor_uuid, actor_alias, created_at) \
             SELECT DISTINCT ON (e.subscription_id) e.subscription_id, e.tenant_id, 'CANCEL', \
                    COALESCE((e.event_data -> 'CancelSubscription' ->> 'usage_cutoff')::timestamp, \
                             e.scheduled_time), \
                    e.event_data -> 'CancelSubscription' ->> 'reason', \
                    e.actor_type, e.actor_uuid, e.actor_alias, $2 \
             FROM scheduled_event e \
             WHERE e.id = ANY($1) AND e.event_type = 'CANCEL_SUBSCRIPTION' \
             ORDER BY e.subscription_id, e.scheduled_time DESC \
             ON CONFLICT (subscription_id, kind) DO UPDATE \
             SET at = EXCLUDED.at, reason = EXCLUDED.reason, actor_type = EXCLUDED.actor_type, \
                 actor_uuid = EXCLUDED.actor_uuid, actor_alias = EXCLUDED.actor_alias, \
                 created_at = EXCLUDED.created_at",
        )
        .bind::<Array<SqlUuid>, _>(&cancellations)
        .bind::<Timestamp, _>(now)
        .execute(conn)
        .await
        .attach("Error while recording a scheduled cancellation")
        .into_db_result()?;
    }

    for (kind, variant, column) in [
        (
            ScheduledEventTypeEnum::FinalizeInvoice,
            "FinalizeInvoice",
            "finalize_after",
        ),
        (
            ScheduledEventTypeEnum::RetryPayment,
            "RetryPayment",
            "next_payment_attempt_at",
        ),
    ] {
        let ids = of(kind);
        if ids.is_empty() {
            continue;
        }
        diesel::sql_query(format!(
            "UPDATE invoice i SET {column} = e.scheduled_time \
             FROM scheduled_event e \
             WHERE e.id = ANY($1) AND i.tenant_id = e.tenant_id \
               AND i.id = (e.event_data -> '{variant}' ->> 'invoice_id')::uuid"
        ))
        .bind::<Array<SqlUuid>, _>(&ids)
        .execute(conn)
        .await
        .attach("Error while recording an invoice deadline")
        .into_db_result()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Listings of current fee rows still reading "not ended" without a date; each takes `as_of`
    /// once future-dated rows exist. A new listing takes it from the start.
    const UNDATED: &[&str] = &[
        "close_incompatible",
        "close_addons",
        "close_components",
        "list_active_add_on_ids",
        "list_active_product_ids",
        "list_subscription_components_by_subscriptions",
        "list_by_product_id",
        "count_active_subscriptions_by_product_id",
        "list_product_ids",
    ];

    fn undated_listings(source: &str) -> Vec<String> {
        source
            .split("pub async fn ")
            .skip(1)
            .filter(|body| body.contains("effective_to.is_null()") && !body.contains("as_of"))
            .filter_map(|body| body.split('(').next().map(str::to_owned))
            .collect()
    }

    #[test]
    fn every_listing_of_current_fee_rows_takes_a_date() {
        let found: Vec<String> = [
            include_str!("subscription_components.rs"),
            include_str!("subscription_add_ons.rs"),
        ]
        .into_iter()
        .flat_map(undated_listings)
        .filter(|name| !UNDATED.contains(&name.as_str()))
        .collect();
        assert!(found.is_empty(), "listings without `as_of`: {found:?}");
    }
}
