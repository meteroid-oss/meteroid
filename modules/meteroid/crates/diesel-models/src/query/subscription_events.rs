use crate::errors::IntoDbResult;

use crate::enums::SubscriptionEventType;
use crate::lock::CustomerLock;
use crate::subscription_events::{SettlingEnding, SettlingSwitchDue, SubscriptionEventRow};
use crate::{DbResult, PgConn};
use chrono::NaiveDate;

use common_domain::ids::{BaseId, SubscriptionId};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use error_stack::ResultExt;

#[derive(diesel::QueryableByName)]
struct CadenceOrigin {
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Date>)]
    origin: Option<NaiveDate>,
}

impl SubscriptionEventRow {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<SubscriptionEventRow> {
        use crate::schema::subscription_event::dsl::subscription_event;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(subscription_event)
            .values(self)
            .returning(SubscriptionEventRow::as_returning());

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting slot transaction")
            .into_db_result()
    }

    pub async fn insert_batch(
        conn: &mut PgConn,
        events: Vec<&SubscriptionEventRow>,
    ) -> DbResult<Vec<SubscriptionEventRow>> {
        use crate::schema::subscription_event::dsl::subscription_event;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(subscription_event)
            .values(events)
            .returning(SubscriptionEventRow::as_returning());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while inserting slot transaction")
            .into_db_result()
    }

    pub async fn fetch_by_subscription_id_and_date(
        conn: &mut PgConn,
        subscription_uid: SubscriptionId,
        date: NaiveDate,
    ) -> DbResult<Vec<SubscriptionEventRow>> {
        use crate::schema::subscription_event::dsl::{
            applies_to, subscription_event, subscription_id,
        };
        use diesel_async::RunQueryDsl;

        let query = subscription_event
            .filter(subscription_id.eq(subscription_uid))
            .filter(applies_to.eq(date))
            .select(SubscriptionEventRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while fetching subscription events")
            .into_db_result()
    }

    /// The days the subscription switched plan, on or before `up_to`, latest first. Amendments,
    /// recorded as switches to the same plan, are left out.
    pub async fn list_plan_switch_dates(
        conn: &mut PgConn,
        subscription_uid: SubscriptionId,
        up_to: NaiveDate,
    ) -> DbResult<Vec<NaiveDate>> {
        use crate::schema::subscription_event::dsl::{
            applies_to, event_type, subscription_event, subscription_id,
        };
        use diesel_async::RunQueryDsl;

        subscription_event
            .filter(subscription_id.eq(subscription_uid))
            .filter(event_type.eq(SubscriptionEventType::Switch))
            .filter(applies_to.le(up_to))
            .filter(diesel::dsl::sql::<diesel::sql_types::Bool>(
                "details ->> 'old_plan_version_id' IS DISTINCT FROM details ->> 'new_plan_version_id'",
            ))
            .order_by(applies_to.desc())
            .select(applies_to)
            .distinct()
            .load::<NaiveDate>(conn)
            .await
            .attach("Error while listing plan switches")
            .into_db_result()
    }

    /// Where the grid of the cadence the plan switch on `on` opened starts, once recorded.
    pub async fn cadence_origin(
        conn: &mut PgConn,
        subscription_uid: SubscriptionId,
        on: NaiveDate,
    ) -> DbResult<Option<NaiveDate>> {
        use diesel::sql_types::{Date, Uuid};
        use diesel_async::RunQueryDsl;

        diesel::sql_query(
            "SELECT max((details ->> 'cadence_origin')::date) AS origin \
             FROM subscription_event \
             WHERE subscription_id = $1 AND applies_to = $2 AND event_type = 'SWITCH' \
               AND details ->> 'old_plan_version_id' IS DISTINCT FROM details ->> 'new_plan_version_id'",
        )
        .bind::<Uuid, _>(subscription_uid)
        .bind::<Date, _>(on)
        .get_result::<CadenceOrigin>(conn)
        .await
        .map(|row| row.origin)
        .attach("Error while reading the origin of a cadence")
        .into_db_result()
    }

    /// Records, unless already recorded, where the grid of the cadence the plan switch on `on`
    /// opened starts. The subscription must be the locked customer's.
    pub async fn record_cadence_origin(
        conn: &mut PgConn,
        lock: &CustomerLock,
        subscription_uid: SubscriptionId,
        on: NaiveDate,
        origin: NaiveDate,
    ) -> DbResult<usize> {
        use diesel::sql_types::{Date, Uuid};
        use diesel_async::RunQueryDsl;

        lock.on(conn);
        diesel::sql_query(
            "UPDATE subscription_event e \
             SET details = COALESCE(e.details, '{}'::jsonb) || jsonb_build_object('cadence_origin', $3::date) \
             WHERE e.subscription_id = $1 AND e.applies_to = $2 AND e.event_type = 'SWITCH' \
               AND e.details ->> 'old_plan_version_id' IS DISTINCT FROM e.details ->> 'new_plan_version_id' \
               AND e.details ->> 'cadence_origin' IS NULL \
               AND EXISTS (SELECT 1 FROM subscription s \
                           WHERE s.id = $1 AND s.tenant_id = $4 AND s.customer_id = $5)",
        )
        .bind::<Uuid, _>(subscription_uid)
        .bind::<Date, _>(on)
        .bind::<Date, _>(origin)
        .bind::<Uuid, _>(lock.tenant_id())
        .bind::<Uuid, _>(lock.customer_id())
        .execute(conn)
        .await
        .attach("Error while recording the origin of a cadence")
        .into_db_result()
    }

    pub async fn fetch_by_subscription_id_and_event_type(
        conn: &mut PgConn,
        subscription_uid: SubscriptionId,
        param_event_type: SubscriptionEventType,
        date: NaiveDate,
    ) -> DbResult<Option<SubscriptionEventRow>> {
        use crate::schema::subscription_event::dsl::{
            applies_to, event_type, subscription_event, subscription_id,
        };
        use diesel_async::RunQueryDsl;

        let query = subscription_event
            .filter(subscription_id.eq(subscription_uid))
            .filter(event_type.eq(param_event_type))
            .filter(applies_to.eq(date))
            .select(SubscriptionEventRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while fetching subscription event by type")
            .into_db_result()
    }

    /// Whether an immediate plan change applied on `on`. Seat and amendment switches change no
    /// plan and are left out.
    pub async fn plan_switched_on(
        conn: &mut PgConn,
        subscription_uid: SubscriptionId,
        on: NaiveDate,
    ) -> DbResult<bool> {
        use crate::schema::subscription_event::dsl::{
            applies_to, event_type, subscription_event, subscription_id,
        };
        use diesel::dsl::exists;
        use diesel_async::RunQueryDsl;

        diesel::select(exists(
            subscription_event
                .filter(subscription_id.eq(subscription_uid))
                .filter(event_type.eq(SubscriptionEventType::Switch))
                .filter(applies_to.eq(on))
                .filter(diesel::dsl::sql::<diesel::sql_types::Bool>(
                    "details ? 'new_plan_version_id' AND details ->> 'mode' = 'immediate'",
                )),
        ))
        .get_result(conn)
        .await
        .attach("Error while looking for a plan switch")
        .into_db_result()
    }

    /// Days in `[from, to)` an immediate plan change of the subscription settled the old plan on.
    pub async fn list_settling_switches(
        conn: &mut PgConn,
        subscription_uid: SubscriptionId,
        from: NaiveDate,
        to: NaiveDate,
    ) -> DbResult<Vec<NaiveDate>> {
        use crate::schema::subscription_event::dsl::{
            applies_to, event_type, subscription_event, subscription_id,
        };
        use diesel_async::RunQueryDsl;

        subscription_event
            .filter(subscription_id.eq(subscription_uid))
            .filter(event_type.eq(SubscriptionEventType::Switch))
            .filter(applies_to.ge(from))
            .filter(applies_to.lt(to))
            .filter(diesel::dsl::sql::<diesel::sql_types::Bool>(
                "details ->> 'outgoing' = 'settled'",
            ))
            .select(applies_to)
            .distinct()
            .order_by(applies_to.asc())
            .load(conn)
            .await
            .attach("Error while listing the plan switches that settle the old plan")
            .into_db_result()
    }

    /// The settling plan switches of the subscription at `at`, oldest first: when each ran and
    /// the seats it read (`details -> 'seats'`, absent on switches of earlier releases).
    pub async fn list_settling_switch_records(
        conn: &mut PgConn,
        subscription_uid: SubscriptionId,
        at: NaiveDate,
    ) -> DbResult<Vec<(chrono::NaiveDateTime, Option<serde_json::Value>)>> {
        use crate::schema::subscription_event::dsl::{
            applies_to, created_at, event_type, subscription_event, subscription_id,
        };
        use diesel_async::RunQueryDsl;

        subscription_event
            .filter(subscription_id.eq(subscription_uid))
            .filter(event_type.eq(SubscriptionEventType::Switch))
            .filter(applies_to.eq(at))
            .filter(diesel::dsl::sql::<diesel::sql_types::Bool>(
                "details ->> 'outgoing' = 'settled'",
            ))
            .select((
                created_at,
                diesel::dsl::sql::<diesel::sql_types::Nullable<diesel::sql_types::Jsonb>>(
                    "details -> 'seats'",
                ),
            ))
            .order_by(created_at.asc())
            .load(conn)
            .await
            .attach("Error while reading the plan switches of a day")
            .into_db_result()
    }

    /// Days in `[from, to)` an immediate cancellation of the subscription settled its lines on.
    pub async fn list_settling_cancellations(
        conn: &mut PgConn,
        subscription_uid: SubscriptionId,
        from: NaiveDate,
        to: NaiveDate,
    ) -> DbResult<Vec<NaiveDate>> {
        use crate::schema::subscription_event::dsl::{
            applies_to, event_type, subscription_event, subscription_id,
        };
        use diesel_async::RunQueryDsl;

        subscription_event
            .filter(subscription_id.eq(subscription_uid))
            .filter(event_type.eq(SubscriptionEventType::Cancelled))
            .filter(applies_to.ge(from))
            .filter(applies_to.lt(to))
            .filter(diesel::dsl::sql::<diesel::sql_types::Bool>(
                "details ->> 'outgoing' = 'settled'",
            ))
            .select(applies_to)
            .distinct()
            .order_by(applies_to.asc())
            .load(conn)
            .await
            .attach("Error while listing the cancellations that settle a subscription's lines")
            .into_db_result()
    }

    /// What the settling cancellation of the subscription on `at` recorded (`details`).
    pub async fn settling_cancellation(
        conn: &mut PgConn,
        subscription_uid: SubscriptionId,
        at: NaiveDate,
    ) -> DbResult<Option<serde_json::Value>> {
        use crate::schema::subscription_event::dsl::{
            applies_to, created_at, details, event_type, subscription_event, subscription_id,
        };
        use diesel_async::RunQueryDsl;

        subscription_event
            .filter(subscription_id.eq(subscription_uid))
            .filter(event_type.eq(SubscriptionEventType::Cancelled))
            .filter(applies_to.eq(at))
            .filter(diesel::dsl::sql::<diesel::sql_types::Bool>(
                "details ->> 'outgoing' = 'settled'",
            ))
            .order_by(created_at.desc())
            .select(details)
            .first::<Option<serde_json::Value>>(conn)
            .await
            .optional()
            .map(Option::flatten)
            .attach("Error while reading a settling cancellation")
            .into_db_result()
    }

    /// Marks the latest cancellation of the subscription on `at` as settling its lines, with what a
    /// later settlement needs (`record`). None was scheduled: 0.
    pub async fn record_settling_cancellation(
        conn: &mut PgConn,
        subscription_uid: SubscriptionId,
        at: NaiveDate,
        record: serde_json::Value,
    ) -> DbResult<usize> {
        use diesel::sql_types::{Date, Jsonb, Uuid};
        use diesel_async::RunQueryDsl;

        diesel::sql_query(
            "UPDATE subscription_event \
             SET details = COALESCE(details, '{}'::jsonb) || $3 \
             WHERE id = (SELECT id FROM subscription_event \
                         WHERE subscription_id = $1 AND applies_to = $2 AND event_type = 'CANCELLED' \
                         ORDER BY created_at DESC, id DESC LIMIT 1)",
        )
        .bind::<Uuid, _>(subscription_uid)
        .bind::<Date, _>(at)
        .bind::<Jsonb, _>(record)
        .execute(conn)
        .await
        .attach("Error while recording a settling cancellation")
        .into_db_result()
    }

    /// Records the notes a plan change or cancellation issued on its event: what marks the lines it
    /// settled.
    pub async fn record_ending_notes(
        conn: &mut PgConn,
        kind: SettlingEnding,
        subscription_uid: SubscriptionId,
        at: NaiveDate,
        note_ids: &[uuid::Uuid],
    ) -> DbResult<usize> {
        use diesel::sql_types::{Date, Jsonb, Text, Uuid};
        use diesel_async::RunQueryDsl;

        let ids =
            serde_json::to_value(note_ids.iter().map(ToString::to_string).collect::<Vec<_>>())
                .unwrap_or_default();
        diesel::sql_query(
            "UPDATE subscription_event \
             SET details = jsonb_set(details, '{notes}', COALESCE(details -> 'notes', '[]'::jsonb) || $3) \
             WHERE subscription_id = $1 AND applies_to = $2 AND event_type::text = $4 \
               AND details ->> 'outgoing' = 'settled'",
        )
        .bind::<Uuid, _>(subscription_uid)
        .bind::<Date, _>(at)
        .bind::<Jsonb, _>(ids)
        .bind::<Text, _>(kind.event_type())
        .execute(conn)
        .await
        .attach("Error while recording the notes of a plan change or cancellation")
        .into_db_result()
    }

    /// Every note the subscription's plan changes and cancellations issued.
    pub async fn list_ending_note_ids(
        conn: &mut PgConn,
        subscription_uid: SubscriptionId,
    ) -> DbResult<Vec<uuid::Uuid>> {
        use diesel::sql_types::{Text, Uuid};
        use diesel_async::RunQueryDsl;

        #[derive(diesel::QueryableByName)]
        struct Note {
            #[diesel(sql_type = Text)]
            id: String,
        }
        let notes: Vec<Note> = diesel::sql_query(
            "SELECT jsonb_array_elements_text(details -> 'notes') AS id FROM subscription_event \
             WHERE subscription_id = $1 AND event_type IN ('SWITCH', 'CANCELLED') \
               AND details ? 'notes'",
        )
        .bind::<Uuid, _>(subscription_uid)
        .load(conn)
        .await
        .attach("Error while listing the notes of plan changes and cancellations")
        .into_db_result()?;
        Ok(notes
            .into_iter()
            .filter_map(|note| note.id.parse().ok())
            .collect())
    }

    /// The plan change or cancellation at `at` has nothing left to settle: no invoice it ended
    /// lines of is unissued.
    pub async fn mark_settling_ending_complete(
        conn: &mut PgConn,
        kind: SettlingEnding,
        subscription_uid: SubscriptionId,
        at: NaiveDate,
    ) -> DbResult<usize> {
        use diesel::sql_types::{Date, Text, Uuid};
        use diesel_async::RunQueryDsl;

        diesel::sql_query(
            "UPDATE subscription_event SET details = details || '{\"outgoing_complete\": true}' \
             WHERE subscription_id = $1 AND applies_to = $2 AND event_type::text = $3 \
               AND details ->> 'outgoing' = 'settled'",
        )
        .bind::<Uuid, _>(subscription_uid)
        .bind::<Date, _>(at)
        .bind::<Text, _>(kind.event_type())
        .execute(conn)
        .await
        .attach("Error while completing a plan change or cancellation")
        .into_db_result()
    }

    /// The next `limit` plan switches (or cancellations) that settle the lines they ended and may
    /// still have invoices to settle, keyset ordered on (day, subscription) after `after`, each
    /// flagged `due` for a check at `now`: backoff elapsed, or an issued invoice of the
    /// subscription, or a consolidated invoice holding one of its drafts, changed since the last
    /// check (issued by any release).
    pub async fn list_incomplete_settling_endings(
        conn: &mut PgConn,
        kind: SettlingEnding,
        after: Option<(NaiveDate, SubscriptionId)>,
        limit: i64,
        now: chrono::NaiveDateTime,
    ) -> DbResult<Vec<SettlingSwitchDue>> {
        use diesel::sql_types::{BigInt, Bool, Date, Timestamp, Uuid};
        use diesel_async::RunQueryDsl;

        #[derive(diesel::QueryableByName)]
        struct Pending {
            #[diesel(sql_type = Uuid)]
            tenant_id: uuid::Uuid,
            #[diesel(sql_type = Uuid)]
            customer_id: uuid::Uuid,
            #[diesel(sql_type = Uuid)]
            subscription_id: uuid::Uuid,
            #[diesel(sql_type = Date)]
            applies_to: NaiveDate,
            #[diesel(sql_type = Bool)]
            invoices_changed: bool,
            #[diesel(sql_type = Bool)]
            due: bool,
        }

        let (after_day, after_subscription) = after
            .map(|(day, subscription)| (day, subscription.as_uuid()))
            .unwrap_or((
                NaiveDate::from_ymd_opt(1, 1, 1).unwrap_or_default(),
                uuid::Uuid::nil(),
            ));
        let rows: Vec<Pending> = diesel::sql_query(kind.incomplete_page())
            .bind::<Date, _>(after_day)
            .bind::<Uuid, _>(after_subscription)
            .bind::<BigInt, _>(limit)
            .bind::<Timestamp, _>(now)
            .load(conn)
            .await
            .attach("Error while listing plan switches or cancellations to settle")
            .into_db_result()?;
        Ok(rows
            .into_iter()
            .map(|row| SettlingSwitchDue {
                tenant_id: common_domain::ids::TenantId::from(row.tenant_id),
                customer_id: common_domain::ids::CustomerId::from(row.customer_id),
                subscription_id: SubscriptionId::from(row.subscription_id),
                applies_to: row.applies_to,
                invoices_changed: row.invoices_changed,
                due: row.due,
            })
            .collect())
    }

    /// Records a check that left the plan change or cancellation at `at` incomplete: the next one
    /// is due after twice the last wait (10 minutes to a day), or after 10 minutes again when
    /// `reset`.
    pub async fn record_settling_ending_check(
        conn: &mut PgConn,
        kind: SettlingEnding,
        subscription_uid: SubscriptionId,
        at: NaiveDate,
        now: chrono::NaiveDateTime,
        reset: bool,
    ) -> DbResult<usize> {
        use diesel::sql_types::{Bool, Date, Text, Timestamp, Uuid};
        use diesel_async::RunQueryDsl;

        diesel::sql_query(
            "UPDATE subscription_event \
             SET outgoing_checked_at = $3, \
                 outgoing_next_check_at = $3 + LEAST(interval '1 day', GREATEST(interval '10 minutes', \
                   CASE WHEN $4 THEN interval '0' \
                        ELSE 2 * COALESCE(outgoing_next_check_at - outgoing_checked_at, interval '0') END)) \
             WHERE subscription_id = $1 AND applies_to = $2 AND event_type::text = $5 \
               AND details ->> 'outgoing' = 'settled'",
        )
        .bind::<Uuid, _>(subscription_uid)
        .bind::<Date, _>(at)
        .bind::<Timestamp, _>(now)
        .bind::<Bool, _>(reset)
        .bind::<Text, _>(kind.event_type())
        .execute(conn)
        .await
        .attach("Error while recording a plan change or cancellation check")
        .into_db_result()
    }

    pub async fn update_mrr_delta(
        conn: &mut PgConn,
        event_id: uuid::Uuid,
        delta: i64,
    ) -> DbResult<()> {
        use crate::schema::subscription_event::dsl::{id, mrr_delta, subscription_event};
        use diesel_async::RunQueryDsl;

        let query = diesel::update(subscription_event)
            .filter(id.eq(event_id))
            .set(mrr_delta.eq(Some(delta)));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while updating subscription event mrr delta")
            .into_db_result()
    }

    pub async fn update_mrr_movement_log_id(
        conn: &mut PgConn,
        event_id: uuid::Uuid,
        mrr_log_id: uuid::Uuid,
    ) -> DbResult<()> {
        use crate::schema::subscription_event::dsl::{
            bi_mrr_movement_log_id, id, subscription_event,
        };
        use diesel::ExpressionMethods;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(subscription_event)
            .filter(id.eq(event_id))
            .set(bi_mrr_movement_log_id.eq(Some(mrr_log_id)));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while updating subscription event mrr movement log id")
            .into_db_result()
    }
}
