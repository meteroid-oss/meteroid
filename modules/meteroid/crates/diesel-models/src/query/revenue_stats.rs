//! The Postgres home dashboard: MRR from `bi_mrr_movement_log`, billed net revenue from
//! finalized invoices minus finalized credit notes. Amounts stay in their own currency.

use crate::errors::IntoDbResult;
use crate::revenue_stats::{CustomerNameRow, MrrLogRow, MrrMovementDayRow, RevenueDayRow};
use crate::{DbResult, PgConn};
use chrono::NaiveDate;
use common_domain::ids::TenantId;
use diesel::sql_types;
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

const MOVEMENT_COLUMNS: &str = r"
    SUM(m.net_mrr_change)::bigint AS net,
    COALESCE(SUM(m.net_mrr_change) FILTER (WHERE m.movement_type = 'NEW_BUSINESS'), 0)::bigint AS new_business,
    (COUNT(*) FILTER (WHERE m.movement_type = 'NEW_BUSINESS'))::int AS new_business_count,
    COALESCE(SUM(m.net_mrr_change) FILTER (WHERE m.movement_type = 'EXPANSION'), 0)::bigint AS expansion,
    (COUNT(*) FILTER (WHERE m.movement_type = 'EXPANSION'))::int AS expansion_count,
    COALESCE(SUM(m.net_mrr_change) FILTER (WHERE m.movement_type = 'CONTRACTION'), 0)::bigint AS contraction,
    (COUNT(*) FILTER (WHERE m.movement_type = 'CONTRACTION'))::int AS contraction_count,
    COALESCE(SUM(m.net_mrr_change) FILTER (WHERE m.movement_type = 'CHURN'), 0)::bigint AS churn,
    (COUNT(*) FILTER (WHERE m.movement_type = 'CHURN'))::int AS churn_count,
    COALESCE(SUM(m.net_mrr_change) FILTER (WHERE m.movement_type = 'REACTIVATION'), 0)::bigint AS reactivation,
    (COUNT(*) FILTER (WHERE m.movement_type = 'REACTIVATION'))::int AS reactivation_count";

impl MrrMovementDayRow {
    /// Movements applying on `[from, to]`, or before `to` when `from` is `None`, grouped by day
    /// and currency (and plan when `split_by_plan`).
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        from: Option<NaiveDate>,
        to: NaiveDate,
        to_inclusive: bool,
        plan_ids: &[Uuid],
        split_by_plan: bool,
    ) -> DbResult<Vec<MrrMovementDayRow>> {
        let to_op = if to_inclusive { "<=" } else { "<" };
        let raw_sql = format!(
            r"
            SELECT m.applies_to AS date,
                   m.currency AS currency,
                   CASE WHEN $5 THEN pv.plan_id END AS plan_id,
                   {MOVEMENT_COLUMNS}
            FROM bi_mrr_movement_log m
            JOIN plan_version pv ON pv.id = m.plan_version_id
            WHERE m.tenant_id = $1
              AND ($2::date IS NULL OR m.applies_to >= $2)
              AND m.applies_to {to_op} $3
              AND (cardinality($4::uuid[]) = 0 OR pv.plan_id = ANY($4))
            GROUP BY 1, 2, 3
            "
        );
        diesel::sql_query(raw_sql)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .bind::<sql_types::Nullable<sql_types::Date>, _>(from)
            .bind::<sql_types::Date, _>(to)
            .bind::<sql_types::Array<sql_types::Uuid>, _>(plan_ids)
            .bind::<sql_types::Bool, _>(split_by_plan)
            .get_results::<MrrMovementDayRow>(conn)
            .await
            .attach("Error while listing MRR movements")
            .into_db_result()
    }
}

impl MrrLogRow {
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        limit: i64,
        window: Option<(NaiveDate, NaiveDate)>,
    ) -> DbResult<Vec<MrrLogRow>> {
        let raw_sql = r"
            SELECT m.created_at, m.applies_to, m.customer_id, c.name AS customer_name,
                   m.subscription_id, p.name AS plan_name, m.description, m.movement_type,
                   m.net_mrr_change, m.currency
            FROM bi_mrr_movement_log m
            JOIN customer c ON c.id = m.customer_id
            JOIN plan_version pv ON pv.id = m.plan_version_id
            JOIN plan p ON p.id = pv.plan_id
            WHERE m.tenant_id = $1
              AND ($2::date IS NULL OR m.applies_to BETWEEN $2 AND $3)
            ORDER BY m.created_at DESC, m.id DESC
            LIMIT $4
        ";
        diesel::sql_query(raw_sql)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .bind::<sql_types::Nullable<sql_types::Date>, _>(window.map(|(from, _)| from))
            .bind::<sql_types::Nullable<sql_types::Date>, _>(window.map(|(_, to)| to))
            .bind::<sql_types::BigInt, _>(limit)
            .get_results::<MrrLogRow>(conn)
            .await
            .attach("Error while listing the MRR log")
            .into_db_result()
    }
}

impl RevenueDayRow {
    /// Issued invoices by invoice date and finalized credit notes (stored negative) by
    /// finalization date, net of discounts and tax. See `RevenueStats::revenue_daily`.
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        range: Option<(NaiveDate, NaiveDate)>,
        plan_ids: &[Uuid],
        by_customer: bool,
    ) -> DbResult<Vec<RevenueDayRow>> {
        // `cash`: the share of an invoice not paid with purchased credit, which counted as
        // revenue when it was sold; what the retired customer balance paid counts as cash.
        // `weight`: the share of an invoice on the filtered plans (consolidated: by child subtotal).
        let raw_sql = r"
            WITH legacy AS (
                SELECT DISTINCT t.invoice_id
                FROM customer_balance_tx t
                WHERE t.tenant_id = $1 AND t.amount_cents < 0 AND t.invoice_id IS NOT NULL
            ),
            noted AS (
                SELECT cn.invoice_id, cn.finalized_at, cn.currency, cn.customer_id,
                       (cn.total - cn.tax_amount) AS net
                FROM credit_note cn
                WHERE cn.tenant_id = $1
                  AND cn.status = 'FINALIZED'
                  AND cn.finalized_at IS NOT NULL
                  AND ($2::date IS NULL OR (cn.finalized_at AT TIME ZONE 'UTC')::date BETWEEN $2 AND $3)
            ),
            children AS (
                SELECT c.consolidated_into_invoice_id AS invoice_id, pv.plan_id, c.subtotal
                FROM invoice c
                LEFT JOIN plan_version pv ON pv.id = c.plan_version_id
                WHERE c.tenant_id = $1 AND c.consolidated_into_invoice_id IS NOT NULL
                  AND cardinality($4::uuid[]) > 0
            ),
            consolidated_weight AS (
                SELECT invoice_id,
                       COALESCE(SUM(subtotal) FILTER (WHERE plan_id = ANY($4)), 0)::numeric
                           / NULLIF(SUM(subtotal), 0) AS weight
                FROM children
                GROUP BY invoice_id
            ),
            issued AS (
                SELECT i.id, i.invoice_date, i.currency, i.customer_id,
                       CASE WHEN i.total <= 0 OR l.invoice_id IS NOT NULL THEN 1::numeric
                            ELSE GREATEST(i.total - i.applied_credits, 0)::numeric / i.total
                       END AS cash,
                       CASE WHEN cardinality($4::uuid[]) = 0 THEN 1::numeric
                            WHEN cw.invoice_id IS NOT NULL THEN COALESCE(cw.weight, 0)
                            WHEN pv.plan_id = ANY($4) THEN 1::numeric
                            ELSE 0::numeric
                       END AS weight,
                       (i.total - i.tax_amount) AS net
                FROM invoice i
                LEFT JOIN legacy l ON l.invoice_id = i.id
                LEFT JOIN plan_version pv ON pv.id = i.plan_version_id
                LEFT JOIN consolidated_weight cw ON cw.invoice_id = i.id
                WHERE i.tenant_id = $1
                  AND i.status IN ('FINALIZED', 'UNCOLLECTIBLE', 'VOID')
                  AND i.finalized_at IS NOT NULL
                  AND i.consolidated_into_invoice_id IS NULL
                  AND ($2::date IS NULL OR i.invoice_date BETWEEN $2 AND $3
                       OR i.id IN (SELECT invoice_id FROM noted))
            )
            SELECT r.d AS date, r.currency, CASE WHEN $5 THEN r.customer_id END AS customer_id,
                   SUM(r.amount)::bigint AS net
            FROM (
                SELECT i.invoice_date AS d, i.currency, i.customer_id,
                       ROUND(i.net * i.cash * i.weight) AS amount
                FROM issued i
                WHERE i.weight > 0
                  AND ($2::date IS NULL OR i.invoice_date BETWEEN $2 AND $3)
                UNION ALL
                SELECT (cn.finalized_at AT TIME ZONE 'UTC')::date AS d, cn.currency, cn.customer_id,
                       ROUND(cn.net * i.cash * i.weight) AS amount
                FROM noted cn
                JOIN issued i ON i.id = cn.invoice_id
                WHERE i.weight > 0
            ) r
            GROUP BY 1, 2, 3
        ";
        diesel::sql_query(raw_sql)
            .bind::<sql_types::Uuid, _>(tenant_id)
            .bind::<sql_types::Nullable<sql_types::Date>, _>(range.map(|(from, _)| from))
            .bind::<sql_types::Nullable<sql_types::Date>, _>(range.map(|(_, to)| to))
            .bind::<sql_types::Array<sql_types::Uuid>, _>(plan_ids)
            .bind::<sql_types::Bool, _>(by_customer)
            .get_results::<RevenueDayRow>(conn)
            .await
            .attach("Error while listing billed revenue")
            .into_db_result()
    }
}

impl CustomerNameRow {
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        ids: &[Uuid],
    ) -> DbResult<Vec<CustomerNameRow>> {
        diesel::sql_query("SELECT id, name FROM customer WHERE tenant_id = $1 AND id = ANY($2)")
            .bind::<sql_types::Uuid, _>(tenant_id)
            .bind::<sql_types::Array<sql_types::Uuid>, _>(ids)
            .get_results::<CustomerNameRow>(conn)
            .await
            .attach("Error while resolving customer names")
            .into_db_result()
    }
}
