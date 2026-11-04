//! Read-only lookups the customer portal's credit pages are built from.
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use chrono::{DateTime, Utc};
use common_domain::ids::{
    BaseId, CreditGrantId, CustomerId, InvoiceId, SubscriptionId, TenantId, WalletId,
};
use diesel::{ExpressionMethods, QueryDsl, QueryableByName, sql_types};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

/// What a grant's own row tells beyond the domain grant: its rollover, whether an automatic
/// top-up bought it, and the number of the invoice that paid for it.
#[derive(Debug, Clone, QueryableByName)]
pub struct PortalGrantDetailRow {
    #[diesel(sql_type = sql_types::Uuid)]
    pub grant_id: CreditGrantId,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Jsonb>)]
    pub rollover: Option<serde_json::Value>,
    #[diesel(sql_type = sql_types::Bool)]
    pub automatic_top_up: bool,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Text>)]
    pub invoice_number: Option<String>,
}

/// The sum of a wallet's ledger entries of one kind, over its grants already in effect.
#[derive(Debug, Clone, QueryableByName)]
pub struct PortalLedgerKindTotalRow {
    #[diesel(sql_type = sql_types::Uuid)]
    pub account_id: WalletId,
    #[diesel(sql_type = sql_types::Text)]
    pub kind: String,
    #[diesel(sql_type = sql_types::BigInt)]
    pub total: i64,
}

pub async fn list_portal_grant_details(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    automatic_top_up_namespace: &str,
) -> DbResult<Vec<PortalGrantDetailRow>> {
    diesel::sql_query(
        "WITH automatic AS (
             SELECT DISTINCT invoice_id
             FROM customer_credit_operation
             WHERE tenant_id = $1 AND customer_id = $2 AND namespace = $3
               AND invoice_id IS NOT NULL
         )
         SELECT g.id AS grant_id,
                g.rollover,
                (a.invoice_id IS NOT NULL) AS automatic_top_up,
                i.invoice_number
         FROM customer_credit_grant g
         LEFT JOIN automatic a ON a.invoice_id = g.invoice_id
         LEFT JOIN invoice i ON i.id = g.invoice_id AND i.tenant_id = g.tenant_id
         WHERE g.tenant_id = $1 AND g.customer_id = $2",
    )
    .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
    .bind::<sql_types::Uuid, _>(customer_id.as_uuid())
    .bind::<sql_types::Text, _>(automatic_top_up_namespace)
    .get_results::<PortalGrantDetailRow>(conn)
    .await
    .attach("Error while reading the portal credit grant details")
    .into_db_result()
}

/// Grants taking effect after `now` are left out, as they are of the wallet's available credit.
pub async fn list_portal_ledger_kind_totals(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    now: DateTime<Utc>,
) -> DbResult<Vec<PortalLedgerKindTotalRow>> {
    diesel::sql_query(
        "SELECT g.account_id, e.kind, SUM(e.amount)::int8 AS total
         FROM customer_credit_entry e
         JOIN customer_credit_grant g ON g.id = e.grant_id AND g.tenant_id = e.tenant_id
         WHERE e.tenant_id = $1 AND e.customer_id = $2 AND g.effective_at <= $3
         GROUP BY g.account_id, e.kind",
    )
    .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
    .bind::<sql_types::Uuid, _>(customer_id.as_uuid())
    .bind::<sql_types::Timestamptz, _>(now)
    .get_results::<PortalLedgerKindTotalRow>(conn)
    .await
    .attach("Error while summing the portal credit ledger")
    .into_db_result()
}

pub async fn list_customer_invoice_numbers(
    conn: &mut PgConn,
    tenant_id: TenantId,
    customer_id: CustomerId,
    invoice_ids: &[InvoiceId],
) -> DbResult<Vec<(InvoiceId, String)>> {
    use crate::schema::invoice::dsl as i;
    if invoice_ids.is_empty() {
        return Ok(Vec::new());
    }
    i::invoice
        .filter(i::tenant_id.eq(tenant_id))
        .filter(i::customer_id.eq(customer_id))
        .filter(i::id.eq_any(invoice_ids))
        .select((i::id, i::invoice_number))
        .load::<(InvoiceId, String)>(conn)
        .await
        .attach("Error while reading invoice numbers")
        .into_db_result()
}

#[derive(Debug, Clone, QueryableByName)]
pub struct PortalActiveSlotsRow {
    #[diesel(sql_type = sql_types::Uuid)]
    pub subscription_id: SubscriptionId,
    #[diesel(sql_type = sql_types::Text)]
    pub unit: String,
    #[diesel(sql_type = sql_types::Integer)]
    pub active_slots: i32,
}

/// Seats in use at `at` for each `(subscription, unit)`, read without locking: the same count
/// as `SlotTransactionRow::fetch_active_slots`, for many pairs at once. A pair without
/// transactions is absent.
pub async fn list_active_slots(
    conn: &mut PgConn,
    tenant_id: TenantId,
    pairs: &[(SubscriptionId, String)],
    at: chrono::NaiveDateTime,
) -> DbResult<Vec<PortalActiveSlotsRow>> {
    if pairs.is_empty() {
        return Ok(Vec::new());
    }
    let (subscription_ids, units): (Vec<Uuid>, Vec<String>) = pairs
        .iter()
        .map(|(id, unit)| (id.as_uuid(), unit.clone()))
        .unzip();
    diesel::sql_query(
        "WITH pairs AS (
             SELECT * FROM unnest($2::uuid[], $3::text[]) AS p(subscription_id, unit)
         ), checkpoint AS (
             SELECT DISTINCT ON (st.subscription_id, st.unit)
                    st.subscription_id, st.unit, st.prev_active_slots, st.transaction_at
             FROM slot_transaction st
             JOIN pairs p ON p.subscription_id = st.subscription_id AND p.unit = st.unit
             JOIN subscription s ON s.id = st.subscription_id
             WHERE s.tenant_id = $1 AND st.transaction_at <= $4 AND st.status = 'ACTIVE'
             ORDER BY st.subscription_id, st.unit, st.transaction_at DESC
         )
         SELECT c.subscription_id, c.unit,
                (COALESCE(c.prev_active_slots, 0) + COALESCE(SUM(st.delta), 0))::integer
                    AS active_slots
         FROM checkpoint c
         LEFT JOIN slot_transaction st
           ON st.subscription_id = c.subscription_id AND st.unit = c.unit
          AND st.effective_at >= c.transaction_at AND st.effective_at <= $4
          AND st.status = 'ACTIVE'
         GROUP BY c.subscription_id, c.unit, c.prev_active_slots",
    )
    .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
    .bind::<sql_types::Array<sql_types::Uuid>, _>(subscription_ids)
    .bind::<sql_types::Array<sql_types::Text>, _>(units)
    .bind::<sql_types::Timestamp, _>(at)
    .get_results::<PortalActiveSlotsRow>(conn)
    .await
    .attach("Error while reading active seats")
    .into_db_result()
}

pub async fn list_applied_coupons_for_subscriptions(
    conn: &mut PgConn,
    subscription_ids: &[SubscriptionId],
) -> DbResult<Vec<crate::applied_coupons::AppliedCouponDetailedRow>> {
    use crate::applied_coupons::AppliedCouponDetailedRow;
    use crate::schema::applied_coupon::dsl as ac;
    use crate::schema::coupon::dsl as c;
    use diesel::SelectableHelper;
    if subscription_ids.is_empty() {
        return Ok(Vec::new());
    }
    ac::applied_coupon
        .inner_join(c::coupon)
        .filter(ac::subscription_id.eq_any(subscription_ids))
        .select(AppliedCouponDetailedRow::as_select())
        .load(conn)
        .await
        .attach("Error while listing the applied coupons of subscriptions")
        .into_db_result()
}
