use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

use crate::errors::IntoDbResult;
use crate::production_revenue::DailyRevenueRow;
use crate::{DbResult, PgConn};

/// Days in `[from, to)` with revenue, by currency: invoices minus credit notes, pre-tax.
pub async fn daily_production_revenue(
    conn: &mut PgConn,
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
) -> DbResult<Vec<DailyRevenueRow>> {
    diesel::sql_query(
        "SELECT day, currency, SUM(amount)::bigint AS amount_cents, \
                COUNT(*) FILTER (WHERE NOT credit_note) AS invoice_count, \
                COUNT(*) FILTER (WHERE credit_note) AS credit_note_count \
         FROM ( \
           SELECT i.finalized_at::date AS day, i.currency, i.total - i.tax_amount AS amount, \
                  FALSE AS credit_note \
           FROM invoice i JOIN tenant t ON t.id = i.tenant_id \
           WHERE t.environment = 'PRODUCTION' \
             AND i.finalized_at >= $1::timestamp AND i.finalized_at < $2::timestamp \
           UNION ALL \
           SELECT (c.finalized_at AT TIME ZONE 'UTC')::date, c.currency, \
                  c.tax_amount - c.total, TRUE \
           FROM credit_note c JOIN tenant t ON t.id = c.tenant_id \
           WHERE t.environment = 'PRODUCTION' \
             AND c.finalized_at >= ($1::timestamp AT TIME ZONE 'UTC') \
             AND c.finalized_at < ($2::timestamp AT TIME ZONE 'UTC') \
         ) revenue \
         GROUP BY day, currency \
         ORDER BY day, currency",
    )
    .bind::<diesel::sql_types::Date, _>(from)
    .bind::<diesel::sql_types::Date, _>(to)
    .load(conn)
    .await
    .attach("Error while summing production revenue by day")
    .into_db_result()
}
