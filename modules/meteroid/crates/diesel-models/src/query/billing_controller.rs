use crate::billing_controller::BillingControllerRow;
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use chrono::{DateTime, Utc};
use common_domain::ids::{BaseId, CustomerId, TenantId};
use diesel::{OptionalExtension, QueryableByName, sql_types};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl BillingControllerRow {
    /// Leases up to `limit` customers whose wake is due or whose inputs changed, oldest first,
    /// skipping those another replica leased: tenants paused or seeding a sandbox wait.
    pub async fn claim_due(
        conn: &mut PgConn,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
        limit: i64,
    ) -> DbResult<Vec<BillingControllerRow>> {
        diesel::sql_query(
            "UPDATE billing_controller bc
             SET lease_until = $2, updated_at = $1
             FROM (
                 SELECT c.tenant_id, c.customer_id
                 FROM billing_controller c
                 JOIN tenant t ON t.id = c.tenant_id AND t.status = 'ACTIVE'
                 LEFT JOIN sandbox_seed sd ON sd.tenant_id = c.tenant_id AND sd.status = 'IN_PROGRESS'
                 WHERE sd.tenant_id IS NULL
                   -- A failed run waits for its retry, dirty or not.
                   AND (c.next_wake_at <= $1 OR (c.dirty_since IS NOT NULL AND c.attempts = 0))
                   AND (c.lease_until IS NULL OR c.lease_until < $1)
                 ORDER BY LEAST(c.next_wake_at, c.dirty_since)
                 LIMIT $3
                 FOR UPDATE OF c SKIP LOCKED
             ) due
             WHERE bc.tenant_id = due.tenant_id AND bc.customer_id = due.customer_id
             RETURNING bc.*",
        )
        .bind::<sql_types::Timestamptz, _>(now)
        .bind::<sql_types::Timestamptz, _>(lease_until)
        .bind::<sql_types::BigInt, _>(limit)
        .get_results(conn)
        .await
        .attach("Error while claiming due billing controllers")
        .into_db_result()
    }

    /// Ends a run that claimed the customer dirty as of `claimed_dirty`: wakes at `next_wake_at`;
    /// an input that changed since the claim (a newer mark, whatever the clocks) keeps it dirty.
    pub async fn complete(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        claimed_dirty: Option<DateTime<Utc>>,
        next_wake_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> DbResult<()> {
        diesel::sql_query(
            "UPDATE billing_controller
             SET next_wake_at = $3,
                 dirty_since = CASE WHEN dirty_since IS NOT DISTINCT FROM $4 THEN NULL
                                    ELSE dirty_since END,
                 lease_until = NULL, attempts = 0, last_error = NULL, updated_at = $5
             WHERE tenant_id = $1 AND customer_id = $2",
        )
        .bind::<sql_types::Uuid, _>(tenant_id)
        .bind::<sql_types::Uuid, _>(customer_id)
        .bind::<sql_types::Nullable<sql_types::Timestamptz>, _>(next_wake_at)
        .bind::<sql_types::Nullable<sql_types::Timestamptz>, _>(claimed_dirty)
        .bind::<sql_types::Timestamptz, _>(now)
        .execute(conn)
        .await
        .attach("Error while completing a billing controller run")
        .into_db_result()
        .map(|_| ())
    }

    /// A run that failed: retried at `retry_at`, its error kept.
    pub async fn fail(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        error: &str,
        retry_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> DbResult<()> {
        diesel::sql_query(
            "UPDATE billing_controller
             SET next_wake_at = $4, lease_until = NULL, attempts = attempts + 1,
                 last_error = $3, updated_at = $5
             WHERE tenant_id = $1 AND customer_id = $2",
        )
        .bind::<sql_types::Uuid, _>(tenant_id)
        .bind::<sql_types::Uuid, _>(customer_id)
        .bind::<sql_types::Text, _>(error)
        .bind::<sql_types::Timestamptz, _>(retry_at)
        .bind::<sql_types::Timestamptz, _>(now)
        .execute(conn)
        .await
        .attach("Error while recording a failed billing controller run")
        .into_db_result()
        .map(|_| ())
    }

    /// The sweep's recomputed wake of a customer not leased; returns the wake it replaced and
    /// whether it was dirty, None when the customer is leased (its run sets the wake itself).
    pub async fn rewake(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        next_wake_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> DbResult<Option<(Option<DateTime<Utc>>, bool)>> {
        #[derive(QueryableByName)]
        struct Previous {
            #[diesel(sql_type = sql_types::Nullable<sql_types::Timestamptz>)]
            previous: Option<DateTime<Utc>>,
            #[diesel(sql_type = sql_types::Bool)]
            dirty: bool,
        }
        diesel::sql_query(
            "INSERT INTO billing_controller (tenant_id, customer_id, next_wake_at, updated_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (tenant_id, customer_id) DO UPDATE
             SET next_wake_at = excluded.next_wake_at, updated_at = excluded.updated_at
             WHERE billing_controller.lease_until IS NULL OR billing_controller.lease_until < $4
             RETURNING (SELECT b.next_wake_at FROM billing_controller b
                        WHERE b.tenant_id = $1 AND b.customer_id = $2) AS previous,
                       billing_controller.dirty_since IS NOT NULL AS dirty",
        )
        .bind::<sql_types::Uuid, _>(tenant_id)
        .bind::<sql_types::Uuid, _>(customer_id)
        .bind::<sql_types::Nullable<sql_types::Timestamptz>, _>(next_wake_at)
        .bind::<sql_types::Timestamptz, _>(now)
        .get_result::<Previous>(conn)
        .await
        .optional()
        .map(|row| row.map(|row| (row.previous, row.dirty)))
        .attach("Error while recomputing a billing controller's wake")
        .into_db_result()
    }

    /// The customers the controller runs (with a subscription, credit still holding value, or a
    /// row already), `limit` after `after`, for the sweep.
    pub async fn list_customers(
        conn: &mut PgConn,
        after: Option<(TenantId, CustomerId)>,
        limit: i64,
    ) -> DbResult<Vec<(TenantId, CustomerId)>> {
        #[derive(QueryableByName)]
        struct Key {
            #[diesel(sql_type = sql_types::Uuid)]
            tenant_id: TenantId,
            #[diesel(sql_type = sql_types::Uuid)]
            customer_id: CustomerId,
        }
        let (tenant, customer) = after
            .map(|(t, c)| (t.as_uuid(), c.as_uuid()))
            .unwrap_or((uuid::Uuid::nil(), uuid::Uuid::nil()));
        diesel::sql_query(
            "SELECT tenant_id, customer_id FROM (
                 SELECT tenant_id, customer_id FROM subscription
                 UNION
                 SELECT tenant_id, customer_id FROM customer_credit_grant WHERE remaining > 0
                 UNION
                 SELECT tenant_id, customer_id FROM billing_controller
             ) k
             WHERE (tenant_id, customer_id) > ($1, $2)
             ORDER BY tenant_id, customer_id
             LIMIT $3",
        )
        .bind::<sql_types::Uuid, _>(tenant)
        .bind::<sql_types::Uuid, _>(customer)
        .bind::<sql_types::BigInt, _>(limit)
        .get_results::<Key>(conn)
        .await
        .map(|keys| {
            keys.into_iter()
                .map(|key| (key.tenant_id, key.customer_id))
                .collect()
        })
        .attach("Error while listing customers for the billing controller sweep")
        .into_db_result()
    }
}

/// What makes a customer's billing steps due, the earliest of each kind (see the controller).
#[derive(Debug, Clone, Default, QueryableByName)]
pub struct DueTimes {
    /// A live subscription's next transition: its period end (midnight), after any retry backoff.
    #[diesel(sql_type = sql_types::Nullable<sql_types::Timestamptz>)]
    pub transition: Option<DateTime<Utc>>,
    /// A pending scheduled event, after any retry backoff.
    #[diesel(sql_type = sql_types::Nullable<sql_types::Timestamptz>)]
    pub event: Option<DateTime<Utc>>,
    /// The earliest lapse of credit still holding value.
    #[diesel(sql_type = sql_types::Nullable<sql_types::Timestamptz>)]
    pub lapse: Option<DateTime<Utc>>,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Integer>)]
    pub grace_period_hours: Option<i32>,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Bool>)]
    pub consolidate_recurring_invoices: Option<bool>,
    /// An ending's settlement still to complete (its next check).
    #[diesel(sql_type = sql_types::Nullable<sql_types::Timestamptz>)]
    pub ending: Option<DateTime<Utc>>,
    /// A refreshable draft's next hourly refresh.
    #[diesel(sql_type = sql_types::Nullable<sql_types::Timestamptz>)]
    pub draft: Option<DateTime<Utc>>,
}

impl DueTimes {
    pub async fn of_customer(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        now: DateTime<Utc>,
    ) -> DbResult<DueTimes> {
        diesel::sql_query(
            "SELECT
               (SELECT MIN(GREATEST(s.current_period_end::timestamp AT TIME ZONE 'UTC',
                                    COALESCE(s.next_retry AT TIME ZONE 'UTC',
                                             s.current_period_end::timestamp AT TIME ZONE 'UTC')))
                FROM subscription s
                WHERE s.tenant_id = $1 AND s.customer_id = $2
                  AND s.current_period_end IS NOT NULL
                  AND s.status IN ('PENDING_ACTIVATION', 'PENDING_CHARGE', 'TRIAL_ACTIVE', 'ACTIVE', 'PAUSED')
                  AND NOT s.lifecycle_suspended) AS transition,
               (SELECT MIN(GREATEST(e.scheduled_time,
                                    COALESCE(e.next_attempt_at, e.scheduled_time)) AT TIME ZONE 'UTC')
                FROM scheduled_event e
                JOIN subscription s ON s.id = e.subscription_id
                WHERE e.tenant_id = $1 AND s.customer_id = $2 AND e.status = 'PENDING'
                  -- As the claim: a lifecycle event at or after the period end is the renewal's.
                  AND (e.event_type IN ('FINALIZE_INVOICE', 'RETRY_PAYMENT')
                       OR s.current_period_end IS NULL
                       OR s.current_period_end > e.scheduled_time::date)) AS event,
               (SELECT MIN(g.lapses_at) FROM customer_credit_grant g
                WHERE g.tenant_id = $1 AND g.customer_id = $2 AND g.remaining > 0
                  AND g.lapses_at IS NOT NULL) AS lapse,
               (SELECT ie.grace_period_hours FROM customer c
                JOIN invoicing_entity ie ON ie.id = c.invoicing_entity_id
                WHERE c.id = $2) AS grace_period_hours,
               (SELECT ie.consolidate_recurring_invoices FROM customer c
                JOIN invoicing_entity ie ON ie.id = c.invoicing_entity_id
                WHERE c.id = $2) AS consolidate_recurring_invoices,
               (SELECT MIN(COALESCE(ev.outgoing_next_check_at AT TIME ZONE 'UTC', $3))
                FROM subscription_event ev
                JOIN subscription s ON s.id = ev.subscription_id
                WHERE s.tenant_id = $1 AND s.customer_id = $2
                  AND ev.event_type IN ('SWITCH', 'CANCELLED')
                  AND ev.details ->> 'outgoing' = 'settled'
                  AND NOT (ev.details ? 'outgoing_complete')) AS ending,
               (SELECT MIN(COALESCE(i.data_updated_at AT TIME ZONE 'UTC' + interval '1 hour', $3))
                FROM invoice i
                -- As `refreshable_drafts_filter`, which lists the drafts the run refreshes.
                WHERE i.tenant_id = $1 AND i.customer_id = $2
                  AND i.status = 'DRAFT' AND i.invoice_type IN ('RECURRING', 'FINAL')
                  AND i.subscription_id IS NOT NULL
                  AND i.consolidated_into_invoice_id IS NULL
                  AND i.manually_edited_at IS NULL
                  AND NOT EXISTS (SELECT 1 FROM payment_transaction pt
                                  WHERE pt.invoice_id = i.id
                                    AND pt.payment_type IN ('PAYMENT', 'TOP_UP', 'AUTO_TOPUP')
                                    AND pt.status IN ('PENDING', 'READY'))
                  AND (i.billing_period_start > ($3::timestamptz AT TIME ZONE 'UTC')::date - 7
                       OR i.billing_period_start
                          + COALESCE((SELECT ie.grace_period_hours FROM invoicing_entity ie
                                      WHERE ie.id = i.invoicing_entity_id), 0) * interval '1 hour'
                          > ($3::timestamptz AT TIME ZONE 'UTC'))) AS draft",
        )
        .bind::<sql_types::Uuid, _>(tenant_id)
        .bind::<sql_types::Uuid, _>(customer_id)
        .bind::<sql_types::Timestamptz, _>(now)
        .get_result(conn)
        .await
        .attach("Error while reading a customer's due billing times")
        .into_db_result()
    }
}
