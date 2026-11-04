//! Reads of the Account 360 overview (subscriptions, open invoices, payment failures and retries,
//! pending cancellation) and of per-account billing MRR.

use crate::enums::{BillingPeriodEnum, SubscriptionStatusEnum};
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};

use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{
    CustomerId, InvoiceId, PlanVersionId, ScheduledEventId, SubscriptionId, TenantId,
};
use diesel::{QueryableByName, sql_types};
use error_stack::ResultExt;

/// Billing MRR of one customer in one currency (minor units).
#[derive(QueryableByName, Debug, Clone, PartialEq, Eq)]
pub struct CustomerMrrRow {
    #[diesel(sql_type = sql_types::Uuid)]
    pub customer_id: CustomerId,
    #[diesel(sql_type = sql_types::Text)]
    pub currency: String,
    #[diesel(sql_type = sql_types::BigInt)]
    pub mrr_cents: i64,
}

pub const HAS_MINIMUM: &str = "EXISTS (SELECT 1 FROM subscription_adjustment a
    WHERE a.subscription_id = s.id AND a.kind = 'MINIMUM')";

/// `SUM(mrr_cents)` of each customer's billing-active subscriptions, per currency.
pub async fn customer_billing_mrr(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_ids: &[CustomerId],
) -> DbResult<Vec<CustomerMrrRow>> {
    use diesel_async::RunQueryDsl;

    if customer_ids.is_empty() {
        return Ok(vec![]);
    }

    diesel::sql_query(
        r#"
        SELECT s.customer_id, s.currency, SUM(s.mrr_cents)::bigint AS mrr_cents
        FROM subscription s
        WHERE s.tenant_id = $1
          AND s.customer_id = ANY($2)
          AND s.status IN ('ACTIVE', 'TRIAL_ACTIVE', 'PENDING_CHARGE')
        GROUP BY s.customer_id, s.currency
        ORDER BY s.customer_id, s.currency
        "#,
    )
    .bind::<sql_types::Uuid, _>(tenant_id)
    .bind::<sql_types::Array<sql_types::Uuid>, _>(customer_ids)
    .get_results(conn)
    .await
    .attach("Error while reading customer billing MRR")
    .into_db_result()
}

/// A billing-active subscription with what the signals read about its cycle and end.
#[derive(QueryableByName, Debug, Clone)]
pub struct SignalSubscriptionRow {
    #[diesel(sql_type = sql_types::Uuid)]
    pub id: SubscriptionId,
    #[diesel(sql_type = sql_types::Uuid)]
    pub customer_id: CustomerId,
    #[diesel(sql_type = sql_types::Text)]
    pub customer_name: String,
    #[diesel(sql_type = sql_types::Text)]
    pub currency: String,
    #[diesel(sql_type = sql_types::BigInt)]
    pub mrr_cents: i64,
    #[diesel(sql_type = crate::schema::sql_types::BillingPeriodEnum)]
    pub period: BillingPeriodEnum,
    #[diesel(sql_type = crate::schema::sql_types::SubscriptionStatusEnum)]
    pub status: SubscriptionStatusEnum,
    #[diesel(sql_type = sql_types::Date)]
    pub start_date: NaiveDate,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Date>)]
    pub billing_start_date: Option<NaiveDate>,
    #[diesel(sql_type = sql_types::SmallInt)]
    pub billing_day_anchor: i16,
    #[diesel(sql_type = sql_types::Date)]
    pub current_period_start: NaiveDate,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Date>)]
    pub current_period_end: Option<NaiveDate>,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Date>)]
    pub end_date: Option<NaiveDate>,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Timestamp>)]
    pub activated_at: Option<NaiveDateTime>,
    #[diesel(sql_type = sql_types::Text)]
    pub plan_name: String,
    /// Effective date of the earliest pending `CANCEL_SUBSCRIPTION` event.
    #[diesel(sql_type = sql_types::Nullable<sql_types::Date>)]
    pub pending_cancel_date: Option<NaiveDate>,
    #[diesel(sql_type = sql_types::Bool)]
    pub has_minimum: bool,
    #[diesel(sql_type = sql_types::Uuid)]
    pub plan_version_id: PlanVersionId,
    #[diesel(sql_type = sql_types::Bool)]
    pub free_plan: bool,
}

/// Billing-active subscriptions of `customer_ids`, ordered by customer, start date, id.
pub async fn list_billing_active_subscriptions(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_ids: &[CustomerId],
) -> DbResult<Vec<SignalSubscriptionRow>> {
    list_subscriptions_in_status(
        conn,
        tenant_id,
        customer_ids,
        &[
            SubscriptionStatusEnum::Active,
            SubscriptionStatusEnum::TrialActive,
            SubscriptionStatusEnum::PendingCharge,
        ],
    )
    .await
}

/// Subscriptions of `customer_ids` in `statuses`, ordered by customer, start date, id.
pub async fn list_subscriptions_in_status(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_ids: &[CustomerId],
    statuses: &[SubscriptionStatusEnum],
) -> DbResult<Vec<SignalSubscriptionRow>> {
    use diesel_async::RunQueryDsl;

    if customer_ids.is_empty() {
        return Ok(vec![]);
    }

    diesel::sql_query(format!(
        r#"
        SELECT s.id, s.customer_id, c.name AS customer_name, s.currency, s.mrr_cents, s.period,
               s.status, s.start_date, s.billing_start_date, s.billing_day_anchor,
               s.current_period_start, s.current_period_end, s.end_date, s.activated_at,
               p.name AS plan_name, cancel.at AS pending_cancel_date,
               {HAS_MINIMUM} AS has_minimum, s.plan_version_id,
               p.plan_type = 'FREE' AS free_plan
        FROM subscription s
        JOIN customer c ON c.id = s.customer_id AND c.tenant_id = $1
        JOIN plan_version pv ON pv.id = s.plan_version_id
        JOIN plan p ON p.id = pv.plan_id
        LEFT JOIN LATERAL (
            SELECT MIN(se.scheduled_time)::date AS at
            FROM scheduled_event se
            WHERE se.tenant_id = $1
              AND se.subscription_id = s.id
              AND se.event_type = 'CANCEL_SUBSCRIPTION'
              AND se.status = 'PENDING'
        ) cancel ON true
        WHERE s.tenant_id = $1
          AND s.customer_id = ANY($2)
          AND s.status = ANY($3)
        ORDER BY s.customer_id, s.start_date, s.id
        "#
    ))
    .bind::<sql_types::Uuid, _>(tenant_id)
    .bind::<sql_types::Array<sql_types::Uuid>, _>(customer_ids)
    .bind::<sql_types::Array<crate::schema::sql_types::SubscriptionStatusEnum>, _>(statuses)
    .get_results(conn)
    .await
    .attach("Error while listing subscriptions for signals")
    .into_db_result()
}

/// Finalized invoices of tenant `$1` still owed.
pub const OPEN_INVOICE_PREDICATE: &str = r#"
    i.tenant_id = $1
    AND i.status = 'FINALIZED'
    AND i.amount_due > 0
    AND i.payment_status IN ('UNPAID', 'PARTIALLY_PAID', 'ERRORED')
    AND i.voided_at IS NULL
    AND i.marked_as_uncollectible_at IS NULL
"#;

#[derive(QueryableByName, Debug, Clone)]
pub struct FailedPaymentRow {
    #[diesel(sql_type = sql_types::Uuid)]
    pub invoice_id: InvoiceId,
    #[diesel(sql_type = sql_types::Timestamp)]
    pub failed_at: NaiveDateTime,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Text>)]
    pub error_type: Option<String>,
}

/// The latest failed payment attempt of each invoice.
pub async fn list_last_failed_payments(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_ids: &[InvoiceId],
) -> DbResult<Vec<FailedPaymentRow>> {
    use diesel_async::RunQueryDsl;

    if invoice_ids.is_empty() {
        return Ok(vec![]);
    }

    diesel::sql_query(
        r#"
        SELECT DISTINCT ON (t.invoice_id)
               t.invoice_id, (t.created_at AT TIME ZONE 'UTC') AS failed_at, t.error_type
        FROM payment_transaction t
        WHERE t.tenant_id = $1
          AND t.invoice_id = ANY($2)
          AND t.status = 'FAILED'
          AND t.payment_type IN ('PAYMENT', 'TOP_UP', 'AUTO_TOPUP')
        ORDER BY t.invoice_id, t.created_at DESC, t.id DESC
        "#,
    )
    .bind::<sql_types::Uuid, _>(tenant_id)
    .bind::<sql_types::Array<sql_types::Uuid>, _>(invoice_ids)
    .get_results(conn)
    .await
    .attach("Error while listing failed payments")
    .into_db_result()
}

#[derive(QueryableByName, Debug, Clone)]
pub struct PendingRetryRow {
    #[diesel(sql_type = sql_types::Uuid)]
    pub invoice_id: InvoiceId,
    #[diesel(sql_type = sql_types::Timestamp)]
    pub next_retry_at: NaiveDateTime,
}

/// The next pending dunning retry (`RETRY_PAYMENT` event) of each invoice.
pub async fn list_pending_payment_retries(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_ids: &[InvoiceId],
) -> DbResult<Vec<PendingRetryRow>> {
    use diesel_async::RunQueryDsl;

    if invoice_ids.is_empty() {
        return Ok(vec![]);
    }

    diesel::sql_query(
        r#"
        SELECT r.invoice_id, MIN(r.next_attempt) AS next_retry_at
        FROM (
            -- A retry payment that failed itself is tried again at its next attempt.
            SELECT (se.event_data -> 'RetryPayment' ->> 'invoice_id')::uuid AS invoice_id,
                   COALESCE(se.next_attempt_at, se.scheduled_time) AS next_attempt
            FROM scheduled_event se
            WHERE se.tenant_id = $1
              AND se.event_type = 'RETRY_PAYMENT'
              AND se.status = 'PENDING'
        ) r
        WHERE r.invoice_id = ANY($2)
        GROUP BY r.invoice_id
        "#,
    )
    .bind::<sql_types::Uuid, _>(tenant_id)
    .bind::<sql_types::Array<sql_types::Uuid>, _>(invoice_ids)
    .get_results(conn)
    .await
    .attach("Error while listing pending payment retries")
    .into_db_result()
}

#[derive(QueryableByName, Debug, Clone)]
pub struct OpenInvoiceRow {
    #[diesel(sql_type = sql_types::Uuid)]
    pub id: InvoiceId,
    #[diesel(sql_type = sql_types::Text)]
    pub invoice_number: String,
    #[diesel(sql_type = sql_types::Text)]
    pub currency: String,
    #[diesel(sql_type = sql_types::BigInt)]
    pub amount_due: i64,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Timestamp>)]
    pub due_at: Option<NaiveDateTime>,
    #[diesel(sql_type = sql_types::Text)]
    pub payment_status: String,
}

/// The customer's open invoices, oldest due first.
pub async fn list_open_invoices(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
) -> DbResult<Vec<OpenInvoiceRow>> {
    use diesel_async::RunQueryDsl;

    diesel::sql_query(format!(
        r#"
        SELECT i.id, i.invoice_number, i.currency, i.amount_due, i.due_at,
               i.payment_status::text AS payment_status
        FROM invoice i
        WHERE {OPEN_INVOICE_PREDICATE}
          AND i.customer_id = $2
        ORDER BY i.due_at ASC NULLS LAST, i.invoice_date, i.id
        "#
    ))
    .bind::<sql_types::Uuid, _>(tenant_id)
    .bind::<sql_types::Uuid, _>(customer_id)
    .get_results(conn)
    .await
    .attach("Error while listing open invoices")
    .into_db_result()
}

#[derive(QueryableByName, Debug)]
struct ScheduledEventIdRow {
    #[diesel(sql_type = sql_types::Uuid)]
    id: ScheduledEventId,
}

/// The earliest pending `CANCEL_SUBSCRIPTION` event of the subscription.
pub async fn pending_cancellation_event(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> DbResult<Option<ScheduledEventId>> {
    use diesel::OptionalExtension;
    use diesel_async::RunQueryDsl;

    diesel::sql_query(
        r#"
        SELECT se.id
        FROM scheduled_event se
        WHERE se.tenant_id = $1
          AND se.subscription_id = $2
          AND se.event_type = 'CANCEL_SUBSCRIPTION'
          AND se.status = 'PENDING'
        ORDER BY se.scheduled_time, se.id
        LIMIT 1
        "#,
    )
    .bind::<sql_types::Uuid, _>(tenant_id)
    .bind::<sql_types::Uuid, _>(subscription_id)
    .get_result::<ScheduledEventIdRow>(conn)
    .await
    .optional()
    .map(|row| row.map(|r| r.id))
    .attach("Error while reading the pending cancellation")
    .into_db_result()
}
