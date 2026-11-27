use crate::enums::ActorTypeEnum;
use crate::errors::IntoDbResult;
use crate::subscription_facts::{
    SubscriptionEndingRow, SubscriptionPlanWindowRow, SubscriptionPlanWindowRowNew,
    SubscriptionStartSettledRow, SubscriptionTrialRow,
};
use crate::{DbResult, PgConn};
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{BaseId, SubscriptionId, TenantId};
use diesel::sql_types::{Array, Date, Timestamp, Uuid as SqlUuid};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

/// The statuses of a subscription still running, inlined as SQL.
const LIVE_SQL: &str = "('PENDING_ACTIVATION', 'TRIAL_ACTIVE', 'ACTIVE', 'TRIAL_EXPIRED')";

/// The day a subscription's dated rows are read on, as SQL over `subscription`: today, held
/// inside its stored period until the renewal advances it (the rows a change wrote for the
/// period's end are the next period's).
pub fn read_day_sql(subscription: &str) -> String {
    let today = common_clock::now().date_naive();
    format!(
        "LEAST('{today}'::date, COALESCE({subscription}.current_period_end - 1, '{today}'::date))"
    )
}

/// [`read_day_sql`] in Rust: `today`, held before a stored period's end not renewed past yet.
pub fn read_day(today: NaiveDate, current_period_end: Option<NaiveDate>) -> NaiveDate {
    current_period_end
        .and_then(|end| end.pred_opt())
        .map_or(today, |last| today.min(last))
}

/// `table`'s rows in force on their subscription's read day.
pub fn in_force_sql(table: &str, subscription: &str) -> String {
    let day = read_day_sql(subscription);
    format!(
        "({table}.effective_from <= {day} AND ({table}.effective_to IS NULL OR {table}.effective_to > {day}))"
    )
}

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
        window: SubscriptionPlanWindowRowNew,
    ) -> DbResult<SubscriptionPlanWindowRow> {
        Ok(Self::open_replacing(conn, window).await?.0)
    }

    /// [`Self::open`], with the open window it replaced when one started on or after it.
    pub async fn open_replacing(
        conn: &mut PgConn,
        mut window: SubscriptionPlanWindowRowNew,
    ) -> DbResult<(SubscriptionPlanWindowRow, Option<SubscriptionPlanWindowRow>)> {
        use crate::schema::subscription_plan_window::dsl as w;

        let mut replaced = None;
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
                replaced = Some(open);
            }
        }

        let query = diesel::insert_into(w::subscription_plan_window)
            .values((&window, w::created_at.eq(common_clock::now().naive_utc())))
            .returning(SubscriptionPlanWindowRow::as_returning());

        crate::debug_sql(&query);

        let opened = query
            .get_result(conn)
            .await
            .attach("Error while opening a plan window")
            .into_db_result()?;
        Ok((opened, replaced))
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
        let now = common_clock::now().naive_utc();
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

    /// The trial ends on `until` instead, moved by an operator while it runs; false when it had
    /// ended already.
    pub async fn move_end(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        until: NaiveDate,
    ) -> DbResult<bool> {
        diesel::sql_query(
            "UPDATE subscription_trial SET until_date = GREATEST(from_date, $2) \
             WHERE subscription_id = $1",
        )
        .bind::<SqlUuid, _>(subscription_id)
        .bind::<Date, _>(until)
        .execute(conn)
        .await
        .attach("Error while moving a trial's end")
        .map(|moved| moved > 0)
        .into_db_result()
    }

    /// The trial withdrawn: ended on the day it started, it never ran.
    pub async fn remove(conn: &mut PgConn, subscription_id: SubscriptionId) -> DbResult<()> {
        use crate::schema::subscription_trial::dsl as t;

        diesel::delete(t::subscription_trial.filter(t::subscription_id.eq(subscription_id)))
            .execute(conn)
            .await
            .attach("Error while removing a trial")
            .map(|_| ())
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

    /// Drops the cancellation of a live subscription: it was withdrawn before it applied.
    pub async fn withdraw_cancellation(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
    ) -> DbResult<()> {
        diesel::sql_query(format!(
            "DELETE FROM subscription_ending e \
             WHERE e.subscription_id = $1 AND e.kind = 'CANCEL' \
               AND EXISTS (SELECT 1 FROM subscription s WHERE s.id = $1 AND s.status IN {LIVE_SQL})"
        ))
        .bind::<SqlUuid, _>(subscription_id)
        .execute(conn)
        .await
        .attach("Error while withdrawing a cancellation")
        .map(|_| ())
        .into_db_result()
    }
}

/// A cancellation still to apply, as the subscription's details show it.
#[derive(Debug, Clone, diesel::QueryableByName)]
pub struct PendingCancellation {
    #[diesel(sql_type = SqlUuid)]
    pub id: Uuid,
    #[diesel(sql_type = SqlUuid)]
    pub subscription_id: SubscriptionId,
    #[diesel(sql_type = Timestamp)]
    pub at: NaiveDateTime,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub reason: Option<String>,
    #[diesel(sql_type = crate::schema::sql_types::ActorTypeEnum)]
    pub actor_type: ActorTypeEnum,
}

impl SubscriptionEndingRow {
    /// The cancellation of a subscription, replacing one recorded before.
    pub async fn record_cancellation(
        conn: &mut PgConn,
        row: &SubscriptionEndingRow,
    ) -> DbResult<()> {
        use crate::schema::subscription_ending::dsl as e;

        diesel::insert_into(e::subscription_ending)
            .values(row)
            .on_conflict((e::subscription_id, e::kind))
            .do_update()
            .set((
                e::at.eq(row.at),
                e::reason.eq(row.reason.clone()),
                e::actor_type.eq(row.actor_type),
                e::actor_uuid.eq(row.actor_uuid),
                e::actor_alias.eq(row.actor_alias.clone()),
                e::created_at.eq(row.created_at),
                e::requested_at.eq(row.requested_at),
            ))
            .execute(conn)
            .await
            .attach("Error while recording a cancellation")
            .map(|_| ())
            .into_db_result()
    }

    /// A live subscription's fixed term: its end date recorded, replacing the one before, or, with
    /// `None`, withdrawn (it renews until cancelled). The subscription's `end_date` is written beside
    /// it by the same caller.
    pub async fn record_completion(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        row: Option<&SubscriptionEndingRow>,
    ) -> DbResult<()> {
        use crate::schema::subscription_ending::dsl as e;

        let Some(row) = row else {
            return diesel::sql_query(format!(
                "DELETE FROM subscription_ending e \
                 WHERE e.subscription_id = $1 AND e.kind = 'COMPLETE' \
                   AND EXISTS (SELECT 1 FROM subscription s WHERE s.id = $1 AND s.status IN {LIVE_SQL})"
            ))
            .bind::<SqlUuid, _>(subscription_id)
            .execute(conn)
            .await
            .attach("Error while withdrawing an end date")
            .map(|_| ())
            .into_db_result();
        };
        diesel::insert_into(e::subscription_ending)
            .values(row)
            .on_conflict((e::subscription_id, e::kind))
            .do_update()
            .set((
                e::at.eq(row.at),
                e::actor_type.eq(row.actor_type),
                e::actor_uuid.eq(row.actor_uuid),
                e::actor_alias.eq(row.actor_alias.clone()),
                e::created_at.eq(row.created_at),
                e::requested_at.eq(row.requested_at),
            ))
            .execute(conn)
            .await
            .attach("Error while recording an end date")
            .map(|_| ())
            .into_db_result()
    }

    /// The cancellations of the live subscriptions among `subscription_ids`, in the tenant: still
    /// to apply, since applying one ends its subscription.
    pub async fn list_pending_cancellations(
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_ids: &[SubscriptionId],
    ) -> DbResult<Vec<PendingCancellation>> {
        if subscription_ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<Uuid> = subscription_ids.iter().map(|id| id.as_uuid()).collect();
        diesel::sql_query(format!(
            "SELECT e.id, e.subscription_id, e.at, e.reason, e.actor_type \
             FROM subscription_ending e JOIN subscription s ON s.id = e.subscription_id \
             WHERE e.tenant_id = $1 AND e.subscription_id = ANY($2) AND e.kind = 'CANCEL' \
               AND s.status IN {LIVE_SQL} \
             ORDER BY e.at"
        ))
        .bind::<SqlUuid, _>(tenant_id)
        .bind::<Array<SqlUuid>, _>(ids)
        .get_results(conn)
        .await
        .attach("Error while listing pending cancellations")
        .into_db_result()
    }

    /// The cancellation named `id` of a live subscription of the tenant.
    pub async fn find_pending_cancellation(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: Uuid,
    ) -> DbResult<Option<PendingCancellation>> {
        diesel::sql_query(format!(
            "SELECT e.id, e.subscription_id, e.at, e.reason, e.actor_type \
             FROM subscription_ending e JOIN subscription s ON s.id = e.subscription_id \
             WHERE e.tenant_id = $1 AND e.id = $2 AND e.kind = 'CANCEL' \
               AND s.status IN {LIVE_SQL}"
        ))
        .bind::<SqlUuid, _>(tenant_id)
        .bind::<SqlUuid, _>(id)
        .get_result(conn)
        .await
        .optional()
        .attach("Error while reading a pending cancellation")
        .into_db_result()
    }
}

/// Most rows of one kind a subscription's facts are read from: past it the read fails, never
/// truncates.
pub const FACT_ROWS_LIMIT: i64 = 5_000;

fn bounded<T>(rows: Vec<T>, what: &str) -> DbResult<Vec<T>> {
    if rows.len() as i64 > FACT_ROWS_LIMIT {
        return Err(crate::errors::DatabaseError::Others(format!(
            "more than {FACT_ROWS_LIMIT} {what} for one subscription"
        ))
        .into());
    }
    Ok(rows)
}

impl SubscriptionStartSettledRow {
    pub async fn find(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
    ) -> DbResult<Option<SubscriptionStartSettledRow>> {
        use crate::schema::subscription_start_settled::dsl as s;

        s::subscription_start_settled
            .filter(s::subscription_id.eq(subscription_id))
            .select(SubscriptionStartSettledRow::as_select())
            .first(conn)
            .await
            .optional()
            .attach("Error while reading a settled start")
            .into_db_result()
    }
}

impl SubscriptionEndingRow {
    /// Its cancellation and its fixed term's end, each at most once.
    pub async fn list_of(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
    ) -> DbResult<Vec<SubscriptionEndingRow>> {
        use crate::schema::subscription_ending::dsl as e;

        e::subscription_ending
            .filter(e::subscription_id.eq(subscription_id))
            .select(SubscriptionEndingRow::as_select())
            .load(conn)
            .await
            .attach("Error while reading endings")
            .into_db_result()
    }
}

impl crate::subscription_components::SubscriptionComponentRow {
    /// Every row of the subscription, closed and future-dated ones included, earliest first.
    pub async fn list_fact_rows(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::subscription_component::dsl as c;

        let rows = c::subscription_component
            .filter(c::subscription_id.eq(subscription_id))
            .order_by((c::effective_from.asc(), c::id.asc()))
            .limit(FACT_ROWS_LIMIT + 1)
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while reading component rows")
            .into_db_result()?;
        bounded(rows, "component rows")
    }
}

impl crate::subscription_add_ons::SubscriptionAddOnRow {
    /// Every row of the subscription, closed and future-dated ones included, earliest first.
    pub async fn list_fact_rows(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::subscription_add_on::dsl as a;

        let rows = a::subscription_add_on
            .filter(a::subscription_id.eq(subscription_id))
            .order_by((a::effective_from.asc(), a::id.asc()))
            .limit(FACT_ROWS_LIMIT + 1)
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while reading add-on rows")
            .into_db_result()?;
        bounded(rows, "add-on rows")
    }
}

impl crate::subscription_adjustments::SubscriptionAdjustmentRow {
    /// Every dated minimum of the subscription, earliest first.
    pub async fn list_minimum_facts(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::subscription_adjustment::dsl as m;

        let rows = m::subscription_adjustment
            .filter(m::subscription_id.eq(subscription_id))
            .filter(m::kind.eq(crate::enums::AdjustmentKindEnum::Minimum))
            .order_by(m::effective_from.asc())
            .limit(FACT_ROWS_LIMIT + 1)
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while reading minimums")
            .into_db_result()?;
        bounded(rows, "minimums")
    }
}

impl crate::subscription_adjustments::SubscriptionAdjustmentComponentRow {
    /// The products of every dated minimum of the subscription.
    pub async fn list_minimum_fact_scopes(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
    ) -> DbResult<Vec<Self>> {
        use crate::schema::subscription_adjustment_component::dsl as c;

        let rows = c::subscription_adjustment_component
            .filter(c::subscription_id.eq(subscription_id))
            .filter(c::kind.eq(crate::enums::AdjustmentKindEnum::Minimum))
            .limit(FACT_ROWS_LIMIT + 1)
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while reading minimum scopes")
            .into_db_result()?;
        bounded(rows, "minimum scopes")
    }
}

impl crate::subscriptions::SubscriptionRow {
    /// The reconcile watermark, periods ending by it settled before billing from facts, and the
    /// watermark of what a period bills at its end.
    pub async fn reconcile_from(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
    ) -> DbResult<(Option<NaiveDate>, Option<NaiveDate>)> {
        use crate::schema::subscription::dsl as s;

        s::subscription
            .filter(s::id.eq(subscription_id))
            .select((s::reconcile_from, s::reconcile_arrears_from))
            .first(conn)
            .await
            .attach("Error while reading the reconcile watermark")
            .into_db_result()
    }

    /// Sets the reconcile watermarks.
    pub async fn set_reconcile_from(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        from: Option<NaiveDate>,
        arrears_from: Option<NaiveDate>,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl as s;

        diesel::update(s::subscription.filter(s::id.eq(subscription_id)))
            .set((
                s::reconcile_from.eq(from),
                s::reconcile_arrears_from.eq(arrears_from),
            ))
            .execute(conn)
            .await
            .attach("Error while setting the reconcile watermark")
            .into_db_result()
            .map(|_| ())
    }
}

impl crate::subscriptions::SubscriptionRow {
    /// The day its periods start on, moved with a free trial's end.
    pub async fn set_billing_day_anchor(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        billing_day_anchor: i16,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl as s;

        diesel::update(s::subscription.filter(s::id.eq(subscription_id)))
            .set(s::billing_day_anchor.eq(billing_day_anchor))
            .execute(conn)
            .await
            .attach("Error while setting a billing day anchor")
            .into_db_result()
            .map(|_| ())
    }

    /// When the facts next change the subscription's derived state, as the writer read them.
    pub async fn set_next_wake_at(
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> DbResult<()> {
        use crate::schema::subscription::dsl as s;

        diesel::update(s::subscription.filter(s::id.eq(subscription_id)))
            .set(s::next_wake_at.eq(at))
            .execute(conn)
            .await
            .attach("Error while setting the next wake")
            .into_db_result()
            .map(|_| ())
    }
}

/// What a scheduled change wrote dated `on`, to withdraw: the rows it opened go, the rows it
/// closed on `on` reopen, the minimum and trial it changed are restored.
#[derive(Debug, Clone, Default)]
pub struct ScheduledRows {
    pub plan_window: Option<Uuid>,
    /// The open window `plan_window` replaced on its day, restored in its place.
    pub replaced_plan_window: Option<SubscriptionPlanWindowRowNew>,
    pub opened_components: Vec<common_domain::ids::SubscriptionPriceComponentId>,
    pub closed_components: Vec<common_domain::ids::SubscriptionPriceComponentId>,
    pub opened_add_ons: Vec<common_domain::ids::SubscriptionAddOnId>,
    pub closed_add_ons: Vec<common_domain::ids::SubscriptionAddOnId>,
    pub seat_seeds: Vec<common_domain::ids::SlotTransactionId>,
    pub minimum: bool,
    /// The trial's end before the change cut it.
    pub trial_until: Option<NaiveDate>,
}

impl ScheduledRows {
    /// Withdraws the rows of a change still to take effect on `on` (a pending change cancelled or
    /// replaced, or the rollback of the facts migration).
    pub async fn withdraw(
        rows: &Self,
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        on: NaiveDate,
    ) -> DbResult<()> {
        let run = |sql: &'static str| {
            diesel::sql_query(sql)
                .bind::<SqlUuid, _>(subscription_id)
                .bind::<Date, _>(on)
        };
        if let (Some(window), Some(replaced)) = (rows.plan_window, &rows.replaced_plan_window) {
            run("DELETE FROM subscription_plan_window \
                 WHERE id = $3 AND subscription_id = $1 AND from_date >= $2")
            .bind::<SqlUuid, _>(window)
            .execute(conn)
            .await
            .attach("Error while withdrawing a plan window")
            .into_db_result()?;
            diesel::insert_into(crate::schema::subscription_plan_window::table)
                .values((
                    replaced,
                    crate::schema::subscription_plan_window::created_at
                        .eq(common_clock::now().naive_utc()),
                ))
                .execute(conn)
                .await
                .attach("Error while restoring a replaced plan window")
                .into_db_result()?;
        } else if let Some(window) = rows.plan_window {
            // One open window at a time: the change's goes before the one it closed reopens.
            run("WITH gone AS (DELETE FROM subscription_plan_window \
                               WHERE id = $3 AND subscription_id = $1 AND from_date >= $2 \
                               RETURNING subscription_id, from_date) \
                 UPDATE subscription_plan_window p SET to_date = NULL \
                 FROM gone WHERE p.subscription_id = gone.subscription_id \
                   AND p.to_date = gone.from_date")
            .bind::<SqlUuid, _>(window)
            .execute(conn)
            .await
            .attach("Error while withdrawing a plan window")
            .into_db_result()?;
        }
        let ids = |ids: &[Uuid]| ids.to_vec();
        let opened: Vec<Uuid> = rows
            .opened_components
            .iter()
            .map(|id| id.as_uuid())
            .collect();
        let closed: Vec<Uuid> = rows
            .closed_components
            .iter()
            .map(|id| id.as_uuid())
            .collect();
        // R-106: a row an immediate amendment wrote under the change, ended where the change's
        // row starts, runs on.
        run("UPDATE subscription_component c SET effective_to = NULL \
             WHERE c.subscription_id = $1 AND c.effective_to = $2 AND NOT (c.id = ANY($3)) \
               AND COALESCE(c.lineage_id, c.id) IN \
                   (SELECT COALESCE(o.lineage_id, o.id) FROM subscription_component o WHERE o.id = ANY($3))")
        .bind::<Array<SqlUuid>, _>(ids(&opened))
        .execute(conn)
        .await
        .attach("Error while reopening components written under a change")
        .into_db_result()?;
        run("DELETE FROM subscription_component \
             WHERE subscription_id = $1 AND effective_from = $2 AND id = ANY($3)")
        .bind::<Array<SqlUuid>, _>(ids(&opened))
        .execute(conn)
        .await
        .attach("Error while withdrawing components")
        .into_db_result()?;
        run("UPDATE subscription_component SET effective_to = NULL \
             WHERE subscription_id = $1 AND effective_to = $2 AND id = ANY($3)")
        .bind::<Array<SqlUuid>, _>(ids(&closed))
        .execute(conn)
        .await
        .attach("Error while reopening components")
        .into_db_result()?;
        let opened: Vec<Uuid> = rows.opened_add_ons.iter().map(|id| id.as_uuid()).collect();
        let closed: Vec<Uuid> = rows.closed_add_ons.iter().map(|id| id.as_uuid()).collect();
        // R-106: a row an immediate amendment wrote under the change, ended where the change's
        // row starts, runs on.
        run("UPDATE subscription_add_on c SET effective_to = NULL \
             WHERE c.subscription_id = $1 AND c.effective_to = $2 AND NOT (c.id = ANY($3)) \
               AND COALESCE(c.lineage_id, c.id) IN \
                   (SELECT COALESCE(o.lineage_id, o.id) FROM subscription_add_on o WHERE o.id = ANY($3))")
        .bind::<Array<SqlUuid>, _>(ids(&opened))
        .execute(conn)
        .await
        .attach("Error while reopening add-ons written under a change")
        .into_db_result()?;
        run("DELETE FROM subscription_add_on \
             WHERE subscription_id = $1 AND effective_from = $2 AND id = ANY($3)")
        .bind::<Array<SqlUuid>, _>(ids(&opened))
        .execute(conn)
        .await
        .attach("Error while withdrawing add-ons")
        .into_db_result()?;
        run("UPDATE subscription_add_on SET effective_to = NULL \
             WHERE subscription_id = $1 AND effective_to = $2 AND id = ANY($3)")
        .bind::<Array<SqlUuid>, _>(ids(&closed))
        .execute(conn)
        .await
        .attach("Error while reopening add-ons")
        .into_db_result()?;
        let seeds: Vec<Uuid> = rows.seat_seeds.iter().map(|id| id.as_uuid()).collect();
        run("DELETE FROM slot_transaction \
             WHERE subscription_id = $1 AND effective_at = $2::timestamp AND id = ANY($3)")
        .bind::<Array<SqlUuid>, _>(seeds)
        .execute(conn)
        .await
        .attach("Error while withdrawing seat seeds")
        .into_db_result()?;
        if rows.minimum {
            run("DELETE FROM subscription_adjustment_component \
                 WHERE subscription_id = $1 AND kind = 'MINIMUM' AND effective_from = $2")
            .execute(conn)
            .await
            .attach("Error while withdrawing a minimum's scope")
            .into_db_result()?;
            run("DELETE FROM subscription_adjustment \
                 WHERE subscription_id = $1 AND kind = 'MINIMUM' AND effective_from = $2")
            .execute(conn)
            .await
            .attach("Error while withdrawing a minimum")
            .into_db_result()?;
            run("UPDATE subscription_adjustment SET effective_to = NULL \
                 WHERE subscription_id = $1 AND kind = 'MINIMUM' AND effective_to = $2")
            .execute(conn)
            .await
            .attach("Error while reopening a minimum")
            .into_db_result()?;
        }
        if let Some(until) = rows.trial_until {
            run("UPDATE subscription_trial SET until_date = $3 \
                 WHERE subscription_id = $1 AND until_date = GREATEST(from_date, $2)")
            .bind::<Date, _>(until)
            .execute(conn)
            .await
            .attach("Error while restoring a trial")
            .into_db_result()?;
        }
        Ok(())
    }

    /// Moves the fee rows of an amendment still to take effect on `on` to `to`: those it opens
    /// start then, those it ends (and those written under it) end then, its seat seeds go with
    /// them. The plan window it opened goes, its cadence read again on `to`.
    pub async fn defer(
        rows: &Self,
        conn: &mut PgConn,
        subscription_id: SubscriptionId,
        on: NaiveDate,
        to: NaiveDate,
    ) -> DbResult<()> {
        if rows.plan_window.is_some() {
            let window = ScheduledRows {
                plan_window: rows.plan_window,
                replaced_plan_window: rows.replaced_plan_window.clone(),
                ..ScheduledRows::default()
            };
            Self::withdraw(&window, conn, subscription_id, on).await?;
        }
        let run = |sql: &'static str, ids: Vec<Uuid>| {
            diesel::sql_query(sql)
                .bind::<SqlUuid, _>(subscription_id)
                .bind::<Date, _>(on)
                .bind::<Date, _>(to)
                .bind::<Array<SqlUuid>, _>(ids)
        };
        let opened: Vec<Uuid> = rows
            .opened_components
            .iter()
            .map(|id| id.as_uuid())
            .collect();
        let closed: Vec<Uuid> = rows
            .closed_components
            .iter()
            .map(|id| id.as_uuid())
            .collect();
        run(
            "UPDATE subscription_component c SET effective_to = $3 \
             WHERE c.subscription_id = $1 AND c.effective_to = $2 \
               AND (c.id = ANY($4) OR COALESCE(c.lineage_id, c.id) IN \
                   (SELECT COALESCE(o.lineage_id, o.id) FROM subscription_component o \
                    WHERE o.id = ANY($5)))",
            closed,
        )
        .bind::<Array<SqlUuid>, _>(opened.clone())
        .execute(conn)
        .await
        .attach("Error while deferring the components an amendment ends")
        .into_db_result()?;
        run(
            "UPDATE subscription_component SET effective_from = $3 \
             WHERE subscription_id = $1 AND effective_from = $2 AND id = ANY($4)",
            opened,
        )
        .execute(conn)
        .await
        .attach("Error while deferring the components an amendment opens")
        .into_db_result()?;
        let opened: Vec<Uuid> = rows.opened_add_ons.iter().map(|id| id.as_uuid()).collect();
        let closed: Vec<Uuid> = rows.closed_add_ons.iter().map(|id| id.as_uuid()).collect();
        run(
            "UPDATE subscription_add_on c SET effective_to = $3 \
             WHERE c.subscription_id = $1 AND c.effective_to = $2 \
               AND (c.id = ANY($4) OR COALESCE(c.lineage_id, c.id) IN \
                   (SELECT COALESCE(o.lineage_id, o.id) FROM subscription_add_on o \
                    WHERE o.id = ANY($5)))",
            closed,
        )
        .bind::<Array<SqlUuid>, _>(opened.clone())
        .execute(conn)
        .await
        .attach("Error while deferring the add-ons an amendment ends")
        .into_db_result()?;
        run(
            "UPDATE subscription_add_on SET effective_from = $3 \
             WHERE subscription_id = $1 AND effective_from = $2 AND id = ANY($4)",
            opened,
        )
        .execute(conn)
        .await
        .attach("Error while deferring the add-ons an amendment opens")
        .into_db_result()?;
        let seeds: Vec<Uuid> = rows.seat_seeds.iter().map(|id| id.as_uuid()).collect();
        run(
            "UPDATE slot_transaction SET effective_at = $3::timestamp, transaction_at = $3::timestamp \
             WHERE subscription_id = $1 AND effective_at = $2::timestamp AND id = ANY($4)",
            seeds,
        )
        .execute(conn)
        .await
        .attach("Error while deferring seat seeds")
        .into_db_result()?;
        Ok(())
    }
}

impl crate::invoices::InvoiceRow {
    /// The issued (finalized, uncollectible, voided or closed) invoices of a subscription settling
    /// a period from `from` on, its drafts too when `drafts`, all but `except`, and the consolidated
    /// parents its drafts were merged into. A voided or waived one forgives what it held (R-096).
    pub async fn list_issued_of_subscription(
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        from: NaiveDate,
        drafts: bool,
        except: Option<common_domain::ids::InvoiceId>,
    ) -> DbResult<Vec<Self>> {
        use crate::enums::InvoiceStatusEnum;
        use crate::schema::invoice::dsl as i;
        use diesel::NullableExpressionMethods;

        let mut issued = vec![
            InvoiceStatusEnum::Finalized,
            InvoiceStatusEnum::Uncollectible,
            InvoiceStatusEnum::Void,
            InvoiceStatusEnum::Closed,
        ];
        if drafts {
            issued.push(InvoiceStatusEnum::Draft);
        }
        let mut invoices: Vec<Self> = i::invoice
            .filter(i::tenant_id.eq(tenant_id))
            .filter(i::subscription_id.eq(subscription_id))
            .filter(i::billing_period_start.ge(from))
            .filter(i::status.eq_any(issued.clone()))
            .order_by((i::billing_period_start.asc(), i::id.asc()))
            .limit(FACT_ROWS_LIMIT + 1)
            .select(Self::as_select())
            .load(conn)
            .await
            .attach("Error while listing issued invoices")
            .into_db_result()?;
        let parents: Vec<common_domain::ids::InvoiceId> = i::invoice
            .filter(i::tenant_id.eq(tenant_id))
            .filter(i::subscription_id.eq(subscription_id))
            .filter(i::billing_period_start.ge(from))
            .filter(i::consolidated_into_invoice_id.is_not_null())
            .select(i::consolidated_into_invoice_id.assume_not_null())
            .distinct()
            .load(conn)
            .await
            .attach("Error while listing consolidated parents")
            .into_db_result()?;
        if !parents.is_empty() {
            invoices.extend(
                i::invoice
                    .filter(i::tenant_id.eq(tenant_id))
                    .filter(i::id.eq_any(&parents))
                    .filter(i::status.eq_any(issued))
                    .select(Self::as_select())
                    .load(conn)
                    .await
                    .attach("Error while loading consolidated parents")
                    .into_db_result()?,
            );
        }
        invoices.retain(|invoice| Some(invoice.id) != except);
        bounded(invoices, "issued invoices")
    }

    /// The period start of a subscription's latest recurring invoice before `before`, a void one
    /// included: its period was billed, then given back.
    pub async fn latest_recurring_period_before(
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        before: NaiveDate,
    ) -> DbResult<Option<NaiveDate>> {
        use crate::enums::InvoiceType;
        use crate::schema::invoice::dsl as i;
        use diesel::dsl::max;

        i::invoice
            .filter(i::tenant_id.eq(tenant_id))
            .filter(i::subscription_id.eq(subscription_id))
            .filter(i::invoice_type.eq(InvoiceType::Recurring))
            .filter(i::billing_period_start.lt(before))
            .select(max(i::billing_period_start))
            .first(conn)
            .await
            .attach("Error while finding the latest recurring invoice")
            .into_db_result()
    }
}

#[cfg(test)]
mod tests {
    /// Writers closing the rows still open; every listing of current fee rows takes a date, as
    /// future-dated rows exist (a scheduled change's).
    const UNDATED: &[&str] = &["close_incompatible", "close_addons", "close_components"];

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
