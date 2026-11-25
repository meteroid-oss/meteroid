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
    #[diesel(sql_type = sql_types::Uuid)]
    pub quantity_id: common_domain::ids::ManagedQuantityId,
    #[diesel(sql_type = sql_types::Integer)]
    pub active_slots: i32,
}

/// Seats in use at `at` for each `(subscription, managed quantity)`, read without locking: the
/// same count as `SlotTransactionRow::fetch_active_slots`, for many pairs at once. A pair without
/// transactions is absent.
pub async fn list_active_slots(
    conn: &mut PgConn,
    tenant_id: TenantId,
    pairs: &[(SubscriptionId, common_domain::ids::ManagedQuantityId)],
    at: chrono::NaiveDateTime,
) -> DbResult<Vec<PortalActiveSlotsRow>> {
    if pairs.is_empty() {
        return Ok(Vec::new());
    }
    let (subscription_ids, quantities): (Vec<Uuid>, Vec<Uuid>) = pairs
        .iter()
        .map(|(id, quantity)| (id.as_uuid(), quantity.as_uuid()))
        .unzip();
    diesel::sql_query(
        // Each pair counted as `SlotTransactionRow::fetch_active_slots` counts it.
        "WITH pairs AS (
             SELECT * FROM unnest($2::uuid[], $3::uuid[]) AS p(subscription_id, quantity_id)
         ), active AS (
             SELECT st.subscription_id, st.managed_quantity_id AS quantity_id, st.delta,
                    st.prev_active_slots, st.effective_at, st.transaction_at
             FROM slot_transaction st
             JOIN pairs p ON p.subscription_id = st.subscription_id
                 AND p.quantity_id = st.managed_quantity_id
             JOIN subscription s ON s.id = st.subscription_id
             WHERE s.tenant_id = $1 AND st.effective_at <= $4 AND st.status = 'ACTIVE'
         ), base AS (
             SELECT DISTINCT ON (subscription_id, quantity_id)
                    subscription_id, quantity_id, prev_active_slots, effective_at, delta = 0 AS seed
             FROM active
             ORDER BY subscription_id, quantity_id, (delta = 0) DESC,
                      CASE WHEN delta = 0 THEN effective_at END DESC,
                      CASE WHEN delta = 0 THEN transaction_at END DESC,
                      effective_at, transaction_at
         )
         SELECT b.subscription_id, b.quantity_id,
                (b.prev_active_slots + COALESCE((
                    SELECT SUM(a.delta) FROM active a
                    WHERE a.subscription_id = b.subscription_id AND a.quantity_id = b.quantity_id
                      AND a.delta <> 0 AND (NOT b.seed OR a.effective_at >= b.effective_at)
                ), 0))::integer AS active_slots
         FROM base b",
    )
    .bind::<sql_types::Uuid, _>(tenant_id.as_uuid())
    .bind::<sql_types::Array<sql_types::Uuid>, _>(subscription_ids)
    .bind::<sql_types::Array<sql_types::Uuid>, _>(quantities)
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
