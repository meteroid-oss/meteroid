//! Invoices a sandbox seed replayed from past billing, kept from collection and customer emails.
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use common_domain::ids::{InvoiceId, TenantId};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

pub async fn hold_invoice_delivery(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
) -> DbResult<()> {
    use crate::schema::invoice::dsl as i;
    diesel::update(
        i::invoice
            .filter(i::tenant_id.eq(tenant_id))
            .filter(i::id.eq(invoice_id))
            .filter(i::delivery_held_at.is_null()),
    )
    .set(i::delivery_held_at.eq(diesel::dsl::now))
    .execute(conn)
    .await
    .map(|_| ())
    .attach("Error while holding an invoice's delivery")
    .into_db_result()
}

pub async fn is_invoice_delivery_held(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
) -> DbResult<bool> {
    invoice_delivery_held_at(conn, tenant_id, invoice_id)
        .await
        .map(|held_at| held_at.is_some())
}

/// When the invoice's delivery was held, if it is.
pub async fn invoice_delivery_held_at(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
) -> DbResult<Option<chrono::DateTime<chrono::Utc>>> {
    use crate::schema::invoice::dsl as i;
    i::invoice
        .filter(i::tenant_id.eq(tenant_id))
        .filter(i::id.eq(invoice_id))
        .select(i::delivery_held_at)
        .first::<Option<chrono::DateTime<chrono::Utc>>>(conn)
        .await
        .optional()
        .map(Option::flatten)
        .attach("Error while reading an invoice's delivery hold")
        .into_db_result()
}
