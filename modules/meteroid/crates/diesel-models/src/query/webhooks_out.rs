use chrono::NaiveDateTime;
use common_domain::ids::{EventId, TenantId, WebhookDeliveryId, WebhookEndpointId};
use diesel::prelude::{ExpressionMethods, JoinOnDsl, NullableExpressionMethods, QueryDsl};
use diesel::{OptionalExtension, QueryableByName, SelectableHelper, sql_types};
use error_stack::ResultExt;

use crate::enums::{WebhookDeliveryStatusEnum, WebhookEndpointDisabledReasonEnum};
use crate::errors::IntoDbResult;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use crate::webhooks_out::{
    WebhookOutAttemptRow, WebhookOutAttemptRowNew, WebhookOutDeliveryListRow,
    WebhookOutDeliveryRow, WebhookOutDeliveryRowNew, WebhookOutEndpointRow,
    WebhookOutEndpointRowNew, WebhookOutEndpointRowPatch, WebhookOutMessageRow,
    WebhookOutMessageRowNew,
};
use crate::{DbResult, PgConn};

impl WebhookOutEndpointRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<WebhookOutEndpointRow> {
        use crate::schema::webhook_out_endpoint::dsl::webhook_out_endpoint;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(webhook_out_endpoint)
            .values(self)
            .returning(WebhookOutEndpointRow::as_returning());

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting webhook endpoint")
            .into_db_result()
    }
}

impl WebhookOutEndpointRowPatch {
    pub async fn patch(
        &self,
        conn: &mut PgConn,
        tenant_uid: TenantId,
        endpoint_id: WebhookEndpointId,
    ) -> DbResult<WebhookOutEndpointRow> {
        use crate::schema::webhook_out_endpoint::dsl as e_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(e_dsl::webhook_out_endpoint)
            .filter(e_dsl::id.eq(endpoint_id))
            .filter(e_dsl::tenant_id.eq(tenant_uid))
            .filter(e_dsl::archived_at.is_null())
            .set(self)
            .returning(WebhookOutEndpointRow::as_returning());

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while patching webhook endpoint")
            .into_db_result()
    }
}

impl WebhookOutEndpointRow {
    pub async fn list_by_tenant(
        conn: &mut PgConn,
        tenant_uid: TenantId,
    ) -> DbResult<Vec<WebhookOutEndpointRow>> {
        use crate::schema::webhook_out_endpoint::dsl as e_dsl;
        use diesel_async::RunQueryDsl;

        let query = e_dsl::webhook_out_endpoint
            .filter(e_dsl::tenant_id.eq(tenant_uid))
            .filter(e_dsl::archived_at.is_null())
            .select(WebhookOutEndpointRow::as_select())
            .order(e_dsl::created_at.desc());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing webhook endpoints")
            .into_db_result()
    }

    pub async fn count_by_tenant(conn: &mut PgConn, tenant_uid: TenantId) -> DbResult<i64> {
        use crate::schema::webhook_out_endpoint::dsl as e_dsl;
        use diesel_async::RunQueryDsl;

        let query = e_dsl::webhook_out_endpoint
            .filter(e_dsl::tenant_id.eq(tenant_uid))
            .filter(e_dsl::archived_at.is_null())
            .count();

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while counting webhook endpoints")
            .into_db_result()
    }

    /// Non-archived, non-disabled endpoints of the given tenants — the fan-out lookup.
    pub async fn list_active_by_tenants(
        conn: &mut PgConn,
        tenant_uids: &[TenantId],
    ) -> DbResult<Vec<WebhookOutEndpointRow>> {
        use crate::schema::webhook_out_endpoint::dsl as e_dsl;
        use diesel_async::RunQueryDsl;

        let query = e_dsl::webhook_out_endpoint
            .filter(e_dsl::tenant_id.eq_any(tenant_uids))
            .filter(e_dsl::archived_at.is_null())
            .filter(e_dsl::disabled.eq(false))
            .select(WebhookOutEndpointRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing active webhook endpoints")
            .into_db_result()
    }

    pub async fn get_by_id(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        endpoint_id: WebhookEndpointId,
    ) -> DbResult<WebhookOutEndpointRow> {
        use crate::schema::webhook_out_endpoint::dsl as e_dsl;
        use diesel_async::RunQueryDsl;

        let query = e_dsl::webhook_out_endpoint
            .filter(e_dsl::id.eq(endpoint_id))
            .filter(e_dsl::tenant_id.eq(tenant_uid))
            .filter(e_dsl::archived_at.is_null())
            .select(WebhookOutEndpointRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding webhook endpoint")
            .into_db_result()
    }

    /// Unscoped lookup used by the delivery worker, which only knows the endpoint id.
    pub async fn get_by_id_unscoped(
        conn: &mut PgConn,
        endpoint_id: WebhookEndpointId,
    ) -> DbResult<Option<WebhookOutEndpointRow>> {
        use crate::schema::webhook_out_endpoint::dsl as e_dsl;
        use diesel_async::RunQueryDsl;

        let query = e_dsl::webhook_out_endpoint
            .filter(e_dsl::id.eq(endpoint_id))
            .select(WebhookOutEndpointRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding webhook endpoint")
            .into_db_result()
    }

    pub async fn archive(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        endpoint_id: WebhookEndpointId,
        now: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::webhook_out_endpoint::dsl as e_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(e_dsl::webhook_out_endpoint)
            .filter(e_dsl::id.eq(endpoint_id))
            .filter(e_dsl::tenant_id.eq(tenant_uid))
            .filter(e_dsl::archived_at.is_null())
            .set((e_dsl::archived_at.eq(now), e_dsl::updated_at.eq(now)));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while archiving webhook endpoint")
            .into_db_result()
    }

    pub async fn rotate_secret(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        endpoint_id: WebhookEndpointId,
        encrypted_secret: String,
        previous_secret_expires_at: NaiveDateTime,
        now: NaiveDateTime,
    ) -> DbResult<WebhookOutEndpointRow> {
        use crate::schema::webhook_out_endpoint::dsl as e_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(e_dsl::webhook_out_endpoint)
            .filter(e_dsl::id.eq(endpoint_id))
            .filter(e_dsl::tenant_id.eq(tenant_uid))
            .filter(e_dsl::archived_at.is_null())
            .set((
                e_dsl::previous_secret.eq(e_dsl::secret.nullable()),
                e_dsl::previous_secret_expires_at.eq(previous_secret_expires_at),
                e_dsl::secret.eq(encrypted_secret),
                e_dsl::updated_at.eq(now),
            ))
            .returning(WebhookOutEndpointRow::as_returning());

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while rotating webhook endpoint secret")
            .into_db_result()
    }

    pub async fn disable(
        conn: &mut PgConn,
        endpoint_id: WebhookEndpointId,
        reason: WebhookEndpointDisabledReasonEnum,
        now: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::webhook_out_endpoint::dsl as e_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(e_dsl::webhook_out_endpoint)
            .filter(e_dsl::id.eq(endpoint_id))
            .set((
                e_dsl::disabled.eq(true),
                e_dsl::disabled_reason.eq(reason),
                e_dsl::disabled_at.eq(now),
                e_dsl::updated_at.eq(now),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while disabling webhook endpoint")
            .into_db_result()
    }

    /// Applies one attempt to the endpoint's health. A success clears every failure
    /// counter and any pause. A failure starts the failure window if needed and extends
    /// the unreachable streak only for an endpoint-wide failure (any other HTTP answer
    /// proves the endpoint is reachable and resets it). `paused_until` is the pause the
    /// attempt decided on, if any.
    pub async fn record_delivery_outcome(
        conn: &mut PgConn,
        endpoint_id: WebhookEndpointId,
        succeeded: bool,
        endpoint_wide_failure: bool,
        paused_until: Option<NaiveDateTime>,
        now: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::webhook_out_endpoint::dsl as e_dsl;
        use diesel_async::RunQueryDsl;

        diesel::define_sql_function! {
            #[sql_name = "COALESCE"]
            fn coalesce_ts(x: sql_types::Nullable<sql_types::Timestamp>, y: sql_types::Nullable<sql_types::Timestamp>) -> sql_types::Nullable<sql_types::Timestamp>;
        }

        let target = diesel::update(e_dsl::webhook_out_endpoint).filter(e_dsl::id.eq(endpoint_id));

        if succeeded {
            target
                .set((
                    e_dsl::consecutive_failures.eq(0),
                    e_dsl::unreachable_streak.eq(0),
                    e_dsl::failing_since.eq(None::<NaiveDateTime>),
                    e_dsl::paused_until.eq(None::<NaiveDateTime>),
                    e_dsl::last_success_at.eq(now),
                    e_dsl::updated_at.eq(now),
                ))
                .execute(conn)
                .await
        } else if endpoint_wide_failure {
            target
                .set((
                    e_dsl::consecutive_failures.eq(e_dsl::consecutive_failures + 1),
                    e_dsl::unreachable_streak.eq(e_dsl::unreachable_streak + 1),
                    e_dsl::failing_since.eq(coalesce_ts(e_dsl::failing_since, Some(now))),
                    e_dsl::paused_until.eq(paused_until),
                    e_dsl::last_failure_at.eq(now),
                    e_dsl::updated_at.eq(now),
                ))
                .execute(conn)
                .await
        } else {
            target
                .set((
                    e_dsl::consecutive_failures.eq(e_dsl::consecutive_failures + 1),
                    e_dsl::unreachable_streak.eq(0),
                    e_dsl::failing_since.eq(coalesce_ts(e_dsl::failing_since, Some(now))),
                    e_dsl::paused_until.eq(None::<NaiveDateTime>),
                    e_dsl::last_failure_at.eq(now),
                    e_dsl::updated_at.eq(now),
                ))
                .execute(conn)
                .await
        }
        .attach("Error while updating webhook endpoint counters")
        .into_db_result()
    }
}

impl WebhookOutMessageRowNew {
    /// Idempotent on the outbox event id: a redelivered outbox message is a no-op.
    pub async fn insert_ignore_conflict(&self, conn: &mut PgConn) -> DbResult<usize> {
        use crate::schema::webhook_out_message::dsl::webhook_out_message;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(webhook_out_message)
            .values(self)
            .on_conflict_do_nothing();

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while inserting webhook message")
            .into_db_result()
    }
}

impl WebhookOutMessageRow {
    pub async fn get_by_id(
        conn: &mut PgConn,
        message_id: EventId,
    ) -> DbResult<WebhookOutMessageRow> {
        use crate::schema::webhook_out_message::dsl as m_dsl;
        use diesel_async::RunQueryDsl;

        let query = m_dsl::webhook_out_message
            .filter(m_dsl::id.eq(message_id))
            .select(WebhookOutMessageRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding webhook message")
            .into_db_result()
    }

    pub async fn purge_older_than(conn: &mut PgConn, cutoff: NaiveDateTime) -> DbResult<usize> {
        use crate::schema::webhook_out_message::dsl as m_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(m_dsl::webhook_out_message.filter(m_dsl::created_at.lt(cutoff)));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while purging webhook messages")
            .into_db_result()
    }
}

impl WebhookOutDeliveryRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<WebhookOutDeliveryRow> {
        use crate::schema::webhook_out_delivery::dsl::webhook_out_delivery;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(webhook_out_delivery)
            .values(self)
            .returning(WebhookOutDeliveryRow::as_returning());

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting webhook delivery")
            .into_db_result()
    }

    pub async fn insert_batch_ignore_conflict(
        conn: &mut PgConn,
        rows: &[WebhookOutDeliveryRowNew],
    ) -> DbResult<usize> {
        use crate::schema::webhook_out_delivery::dsl as d_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(d_dsl::webhook_out_delivery)
            .values(rows)
            .on_conflict((d_dsl::message_id, d_dsl::endpoint_id))
            .do_nothing();

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while inserting webhook deliveries")
            .into_db_result()
    }
}

impl WebhookOutDeliveryRow {
    pub async fn get_by_id(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        delivery_id: WebhookDeliveryId,
    ) -> DbResult<WebhookOutDeliveryRow> {
        use crate::schema::webhook_out_delivery::dsl as d_dsl;
        use diesel_async::RunQueryDsl;

        let query = d_dsl::webhook_out_delivery
            .filter(d_dsl::id.eq(delivery_id))
            .filter(d_dsl::tenant_id.eq(tenant_uid))
            .select(WebhookOutDeliveryRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding webhook delivery")
            .into_db_result()
    }

    pub async fn list_by_endpoint(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        endpoint_id: WebhookEndpointId,
        status: Option<WebhookDeliveryStatusEnum>,
        pagination: PaginationRequest,
    ) -> DbResult<PaginatedVec<WebhookOutDeliveryListRow>> {
        use crate::schema::webhook_out_delivery::dsl as d_dsl;
        use crate::schema::webhook_out_message::dsl as m_dsl;

        let mut query = d_dsl::webhook_out_delivery
            .inner_join(m_dsl::webhook_out_message.on(m_dsl::id.eq(d_dsl::message_id)))
            .filter(d_dsl::tenant_id.eq(tenant_uid))
            .filter(d_dsl::endpoint_id.eq(endpoint_id))
            .into_boxed();

        if let Some(status) = status {
            query = query.filter(d_dsl::status.eq(status));
        }

        let paginated = query
            .order(d_dsl::created_at.desc())
            .select(WebhookOutDeliveryListRow::as_select())
            .paginate(pagination);

        crate::debug_sql(&paginated);

        paginated
            .load_and_count_pages(conn)
            .await
            .attach("Error while listing webhook deliveries")
            .into_db_result()
    }

    /// Re-queues a delivery for an immediate attempt. `reset_attempts` grants a fresh
    /// retry budget, which a resend of an already-terminal delivery needs.
    pub async fn reschedule_now(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        delivery_id: WebhookDeliveryId,
        reset_attempts: bool,
        now: NaiveDateTime,
    ) -> DbResult<WebhookOutDeliveryRow> {
        use crate::schema::webhook_out_delivery::dsl as d_dsl;
        use diesel_async::RunQueryDsl;

        let target = diesel::update(d_dsl::webhook_out_delivery)
            .filter(d_dsl::id.eq(delivery_id))
            .filter(d_dsl::tenant_id.eq(tenant_uid));

        let common = (
            d_dsl::status.eq(WebhookDeliveryStatusEnum::Pending),
            d_dsl::next_attempt_at.eq(now),
            d_dsl::lease_expires_at.eq(None::<NaiveDateTime>),
            d_dsl::completed_at.eq(None::<NaiveDateTime>),
            d_dsl::manual.eq(true),
            d_dsl::updated_at.eq(now),
        );

        if reset_attempts {
            target
                .set((
                    common,
                    d_dsl::attempt_count.eq(0),
                    d_dsl::last_error.eq(None::<String>),
                    d_dsl::last_response_status.eq(None::<i16>),
                ))
                .returning(WebhookOutDeliveryRow::as_returning())
                .get_result(conn)
                .await
        } else {
            target
                .set(common)
                .returning(WebhookOutDeliveryRow::as_returning())
                .get_result(conn)
                .await
        }
        .attach("Error while rescheduling webhook delivery")
        .into_db_result()
    }

    pub async fn cancel_pending_for_endpoint(
        conn: &mut PgConn,
        endpoint_id: WebhookEndpointId,
        now: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::webhook_out_delivery::dsl as d_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(d_dsl::webhook_out_delivery)
            .filter(d_dsl::endpoint_id.eq(endpoint_id))
            .filter(d_dsl::status.eq(WebhookDeliveryStatusEnum::Pending))
            .set((
                d_dsl::status.eq(WebhookDeliveryStatusEnum::Cancelled),
                d_dsl::next_attempt_at.eq(None::<NaiveDateTime>),
                d_dsl::completed_at.eq(now),
                d_dsl::updated_at.eq(now),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while cancelling pending webhook deliveries")
            .into_db_result()
    }

    /// An expired lease is a failed attempt whose result was never recorded (worker
    /// death, a failed write): the claim already spent its budget, so the delivery is
    /// retried on the normal schedule, or FAILED once `max_attempts` claims are spent.
    /// `retry_delays_secs[n - 1]` is the delay after attempt n.
    pub async fn reclaim_expired(
        conn: &mut PgConn,
        now: NaiveDateTime,
        max_attempts: i32,
        retry_delays_secs: &[i64],
    ) -> DbResult<usize> {
        use diesel_async::RunQueryDsl;

        let raw_sql = r#"
            UPDATE webhook_out_delivery
            SET status = CASE WHEN attempt_count >= $2
                             THEN 'FAILED'::"WebhookDeliveryStatusEnum"
                             ELSE 'PENDING'::"WebhookDeliveryStatusEnum" END,
                next_attempt_at = CASE WHEN attempt_count >= $2 THEN NULL
                    ELSE $1 + (($3::bigint[])[GREATEST(attempt_count, 1)] * interval '1 second')
                END,
                completed_at = CASE WHEN attempt_count >= $2 THEN $1 END,
                lease_expires_at = NULL,
                last_response_status = NULL,
                last_error = $4,
                updated_at = $1
            WHERE status = 'IN_FLIGHT'
              AND lease_expires_at < $1
        "#;

        let query = diesel::sql_query(raw_sql)
            .bind::<sql_types::Timestamp, _>(now)
            .bind::<sql_types::Integer, _>(max_attempts)
            .bind::<sql_types::Array<sql_types::BigInt>, _>(retry_delays_secs.to_vec())
            .bind::<sql_types::Text, _>(
                "the attempt's lease expired before its result was recorded",
            );

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while reclaiming expired webhook deliveries")
            .into_db_result()
    }

    /// Claims due deliveries, never taking an endpoint past its own `max_in_flight`, so
    /// a slow or dead endpoint can only ever hold that many worker slots. A paused
    /// endpoint is skipped until `paused_until`; one whose `unreachable_streak` reached
    /// `probe_threshold` is probed one delivery at a time; a rate-limited one gets no
    /// more claims than its `rate_limit_per_sec` within the last second. The cost is
    /// bounded by endpoints, not by the backlog: only the `limit` endpoints with the
    /// oldest due work are considered, each up to its free slots. Every claim bumps
    /// `attempt_count`, which with the lease identifies the claim. Raw SQL: the
    /// LATERAL candidate set feeding a locking UPDATE is not expressible in Diesel.
    pub async fn claim_due(
        conn: &mut PgConn,
        limit: i64,
        lease_secs: i32,
        now: NaiveDateTime,
        probe_threshold: i32,
    ) -> DbResult<Vec<WebhookOutDeliveryRow>> {
        use diesel_async::RunQueryDsl;

        let raw_sql = r#"
            WITH inflight AS (
                SELECT endpoint_id, count(*) AS n
                FROM webhook_out_delivery
                WHERE status = 'IN_FLIGHT' AND lease_expires_at > $3
                GROUP BY endpoint_id
            ),
            capacity AS (
                SELECT e.id,
                       due.oldest,
                       LEAST(
                           -- An unreachable endpoint is probed one delivery at a time.
                           CASE WHEN e.unreachable_streak >= $4 THEN 1 ELSE e.max_in_flight END
                               - COALESCE(i.n, 0),
                           COALESCE(e.rate_limit_per_sec - recent.n, $1),
                           $1
                       ) AS free_slots
                FROM webhook_out_endpoint e
                LEFT JOIN inflight i ON i.endpoint_id = e.id
                LEFT JOIN LATERAL (
                    SELECT count(*) AS n
                    FROM webhook_out_delivery r
                    WHERE r.endpoint_id = e.id
                      AND r.claimed_at > $3 - interval '1 second'
                ) recent ON e.rate_limit_per_sec IS NOT NULL
                CROSS JOIN LATERAL (
                    SELECT min(p.next_attempt_at) AS oldest
                    FROM webhook_out_delivery p
                    WHERE p.endpoint_id = e.id
                      AND p.status = 'PENDING'
                      AND p.next_attempt_at <= $3
                ) due
                WHERE due.oldest IS NOT NULL
                  AND (e.paused_until IS NULL OR e.paused_until <= $3)
            ),
            open_endpoints AS (
                SELECT id, free_slots
                FROM capacity
                WHERE free_slots > 0
                ORDER BY oldest
                LIMIT $1
            ),
            candidates AS (
                SELECT c.id, c.next_attempt_at
                FROM open_endpoints o
                CROSS JOIN LATERAL (
                    SELECT d.id, d.next_attempt_at
                    FROM webhook_out_delivery d
                    WHERE d.endpoint_id = o.id
                      AND d.status = 'PENDING'
                      AND d.next_attempt_at <= $3
                    ORDER BY d.next_attempt_at, d.id
                    LIMIT o.free_slots
                ) c
            ),
            picked AS (
                SELECT id FROM candidates ORDER BY next_attempt_at, id LIMIT $1
            ),
            locked AS (
                SELECT w.id
                FROM webhook_out_delivery w
                WHERE w.id = ANY (SELECT id FROM picked)
                  AND w.status = 'PENDING'
                  AND w.next_attempt_at <= $3
                FOR UPDATE SKIP LOCKED
            )
            UPDATE webhook_out_delivery
            SET status = 'IN_FLIGHT',
                attempt_count = attempt_count + 1,
                lease_expires_at = $3 + ($2::int * interval '1 second'),
                claimed_at = $3,
                updated_at = $3
            WHERE id IN (SELECT id FROM locked)
            RETURNING id, tenant_id, message_id, endpoint_id, status, attempt_count,
                      next_attempt_at, lease_expires_at, last_response_status, last_error,
                      manual, created_at, updated_at, completed_at, claimed_at
        "#;

        let query = diesel::sql_query(raw_sql)
            .bind::<sql_types::BigInt, _>(limit)
            .bind::<sql_types::Integer, _>(lease_secs)
            .bind::<sql_types::Timestamp, _>(now)
            .bind::<sql_types::Integer, _>(probe_threshold);

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while claiming due webhook deliveries")
            .into_db_result()
    }

    /// Applies the result of one attempt, guarded on the row still holding the claim
    /// the attempt was made under. A resend, a reclaimed lease or a later claim moves
    /// it on, and then this write is dropped. Returns the number of rows updated.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_attempt_result(
        conn: &mut PgConn,
        delivery_id: WebhookDeliveryId,
        claimed_attempt_count: i32,
        claimed_lease_expires_at: NaiveDateTime,
        status: WebhookDeliveryStatusEnum,
        next_attempt_at: Option<NaiveDateTime>,
        response_status: Option<i16>,
        error: Option<String>,
        completed_at: Option<NaiveDateTime>,
        refund_attempt: bool,
        now: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::webhook_out_delivery::dsl as d_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(d_dsl::webhook_out_delivery)
            .filter(d_dsl::id.eq(delivery_id))
            .filter(d_dsl::status.eq(WebhookDeliveryStatusEnum::InFlight))
            .filter(d_dsl::attempt_count.eq(claimed_attempt_count))
            .filter(d_dsl::lease_expires_at.eq(claimed_lease_expires_at))
            .set((
                d_dsl::status.eq(status),
                // A probe of a paused endpoint does not spend the delivery's budget.
                d_dsl::attempt_count.eq(if refund_attempt {
                    claimed_attempt_count - 1
                } else {
                    claimed_attempt_count
                }),
                d_dsl::next_attempt_at.eq(next_attempt_at),
                d_dsl::lease_expires_at.eq(None::<NaiveDateTime>),
                d_dsl::last_response_status.eq(response_status),
                d_dsl::last_error.eq(error),
                d_dsl::completed_at.eq(completed_at),
                d_dsl::updated_at.eq(now),
            ));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while recording webhook delivery result")
            .into_db_result()
    }
}

impl WebhookOutAttemptRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<WebhookOutAttemptRow> {
        use crate::schema::webhook_out_attempt::dsl::webhook_out_attempt;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(webhook_out_attempt)
            .values(self)
            .returning(WebhookOutAttemptRow::as_returning());

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting webhook attempt")
            .into_db_result()
    }
}

impl WebhookOutAttemptRow {
    /// Attempt numbers are a per-delivery sequence that never restarts, so a resend
    /// continues the timeline instead of writing a second run of 1, 2, 3.
    pub async fn next_attempt_number(
        conn: &mut PgConn,
        delivery_id: WebhookDeliveryId,
    ) -> DbResult<i32> {
        use crate::schema::webhook_out_attempt::dsl as a_dsl;
        use diesel_async::RunQueryDsl;

        let highest: Option<i32> = a_dsl::webhook_out_attempt
            .filter(a_dsl::delivery_id.eq(delivery_id))
            .select(diesel::dsl::max(a_dsl::attempt_number))
            .first(conn)
            .await
            .attach("Error while reading the last webhook attempt number")
            .into_db_result()?;

        Ok(highest.unwrap_or(0) + 1)
    }

    /// Attempts made so far per delivery, across resends. Unlike `attempt_count`, which
    /// is the retry budget spent (refunded by probes, reset by a resend), this is what
    /// the attempt log shows. Deliveries without any attempt are absent.
    pub async fn count_by_deliveries(
        conn: &mut PgConn,
        delivery_ids: &[WebhookDeliveryId],
    ) -> DbResult<Vec<(WebhookDeliveryId, i64)>> {
        use crate::schema::webhook_out_attempt::dsl as a_dsl;
        use diesel_async::RunQueryDsl;

        let query = a_dsl::webhook_out_attempt
            .filter(a_dsl::delivery_id.eq_any(delivery_ids))
            .group_by(a_dsl::delivery_id)
            .select((a_dsl::delivery_id, diesel::dsl::count_star()));

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while counting webhook attempts")
            .into_db_result()
    }

    pub async fn list_by_delivery(
        conn: &mut PgConn,
        delivery_id: WebhookDeliveryId,
    ) -> DbResult<Vec<WebhookOutAttemptRow>> {
        use crate::schema::webhook_out_attempt::dsl as a_dsl;
        use diesel_async::RunQueryDsl;

        let query = a_dsl::webhook_out_attempt
            .filter(a_dsl::delivery_id.eq(delivery_id))
            .select(WebhookOutAttemptRow::as_select())
            .order(a_dsl::attempt_number.asc());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing webhook attempts")
            .into_db_result()
    }
}

impl WebhookOutEndpointRow {
    /// Serialises writes that count a tenant's endpoints, so a concurrent create cannot
    /// slip past the per-tenant cap.
    pub async fn lock_tenant_endpoints(conn: &mut PgConn, tenant_uid: TenantId) -> DbResult<()> {
        use diesel_async::RunQueryDsl;

        diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind::<sql_types::Text, _>(tenant_uid.to_string())
            .execute(conn)
            .await
            .attach("Error while locking a tenant's webhook endpoints")
            .into_db_result()?;

        Ok(())
    }
}

impl WebhookOutMessageRow {
    /// One bounded DELETE, so retention never takes a table-wide lock for minutes.
    pub async fn purge_chunk_older_than(
        conn: &mut PgConn,
        cutoff: NaiveDateTime,
        chunk_size: i64,
    ) -> DbResult<usize> {
        use diesel_async::RunQueryDsl;

        let query = diesel::sql_query(
            "DELETE FROM webhook_out_message WHERE id IN \
             (SELECT id FROM webhook_out_message WHERE created_at < $1 LIMIT $2)",
        )
        .bind::<sql_types::Timestamp, _>(cutoff)
        .bind::<sql_types::BigInt, _>(chunk_size);

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while purging webhook messages")
            .into_db_result()
    }
}

/// Deliveries of one day by outcome, for the endpoint charts.
#[derive(QueryableByName, Debug, Clone)]
pub struct WebhookOutDailyStatsRow {
    #[diesel(sql_type = sql_types::Date)]
    pub day: chrono::NaiveDate,
    #[diesel(sql_type = sql_types::BigInt)]
    pub succeeded: i64,
    #[diesel(sql_type = sql_types::BigInt)]
    pub failed: i64,
    #[diesel(sql_type = sql_types::BigInt)]
    pub retrying: i64,
    #[diesel(sql_type = sql_types::BigInt)]
    pub cancelled: i64,
}

/// Deliveries of one endpoint over a window by outcome, for the success rate column.
#[derive(QueryableByName, Debug, Clone)]
pub struct WebhookOutEndpointStatsRow {
    #[diesel(sql_type = sql_types::Uuid)]
    pub endpoint_id: WebhookEndpointId,
    #[diesel(sql_type = sql_types::BigInt)]
    pub succeeded: i64,
    #[diesel(sql_type = sql_types::BigInt)]
    pub failed: i64,
    #[diesel(sql_type = sql_types::BigInt)]
    pub retrying: i64,
    #[diesel(sql_type = sql_types::BigInt)]
    pub cancelled: i64,
}

/// Unfinished deliveries that already failed at least once: every failed attempt sets
/// `last_error`, and a resend that grants a fresh budget clears it.
const RETRYING_FILTER: &str = "status IN ('PENDING', 'IN_FLIGHT') AND last_error IS NOT NULL";

impl WebhookOutDeliveryRow {
    /// Deliveries per day since `since` by outcome, by creation day; days with none of
    /// them are absent. A pending delivery that has not failed yet is not counted.
    pub async fn daily_stats(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        endpoint_id: WebhookEndpointId,
        since: NaiveDateTime,
    ) -> DbResult<Vec<WebhookOutDailyStatsRow>> {
        use diesel_async::RunQueryDsl;

        let query = diesel::sql_query(format!(
            r#"
            SELECT created_at::date AS day,
                   count(*) FILTER (WHERE status = 'SUCCEEDED') AS succeeded,
                   count(*) FILTER (WHERE status = 'FAILED') AS failed,
                   count(*) FILTER (WHERE {RETRYING_FILTER}) AS retrying,
                   count(*) FILTER (WHERE status = 'CANCELLED') AS cancelled
            FROM webhook_out_delivery
            WHERE tenant_id = $1
              AND endpoint_id = $2
              AND created_at >= $3
              AND (status IN ('SUCCEEDED', 'FAILED', 'CANCELLED') OR {RETRYING_FILTER})
            GROUP BY 1
            ORDER BY 1
            "#,
        ))
        .bind::<sql_types::Uuid, _>(*tenant_uid)
        .bind::<sql_types::Uuid, _>(*endpoint_id)
        .bind::<sql_types::Timestamp, _>(since);

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while computing webhook daily stats")
            .into_db_result()
    }

    /// Deliveries per endpoint of a tenant since `since` by outcome, as [`Self::daily_stats`].
    pub async fn endpoint_stats(
        conn: &mut PgConn,
        tenant_uid: TenantId,
        since: NaiveDateTime,
    ) -> DbResult<Vec<WebhookOutEndpointStatsRow>> {
        use diesel_async::RunQueryDsl;

        let query = diesel::sql_query(format!(
            r#"
            SELECT endpoint_id,
                   count(*) FILTER (WHERE status = 'SUCCEEDED') AS succeeded,
                   count(*) FILTER (WHERE status = 'FAILED') AS failed,
                   count(*) FILTER (WHERE {RETRYING_FILTER}) AS retrying,
                   count(*) FILTER (WHERE status = 'CANCELLED') AS cancelled
            FROM webhook_out_delivery
            WHERE tenant_id = $1
              AND created_at >= $2
              AND (status IN ('SUCCEEDED', 'FAILED', 'CANCELLED') OR {RETRYING_FILTER})
            GROUP BY endpoint_id
            "#,
        ))
        .bind::<sql_types::Uuid, _>(*tenant_uid)
        .bind::<sql_types::Timestamp, _>(since);

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while computing webhook endpoint stats")
            .into_db_result()
    }
}
