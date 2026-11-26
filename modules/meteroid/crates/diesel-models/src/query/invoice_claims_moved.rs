use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;
use common_domain::ids::{BaseId, CustomerId, InvoiceId, TenantId};
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

/// Drafts whose usage changed hands since their lines were computed (R-143).
pub struct InvoiceClaimsMoved;

impl InvoiceClaimsMoved {
    /// The customer's drafts not edited by hand: marked moved, and left for the refresh sweep.
    pub async fn mark_customer_drafts(
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        at: NaiveDateTime,
    ) -> DbResult<usize> {
        diesel::sql_query(
            "WITH drafts AS (
               UPDATE invoice SET data_updated_at = NULL
               WHERE tenant_id = $1 AND customer_id = $2 AND status = 'DRAFT'
                 AND manually_edited_at IS NULL
                 AND (subscription_id IS NOT NULL
                   OR id IN (SELECT c.consolidated_into_invoice_id FROM invoice c
                             WHERE c.tenant_id = $1 AND c.customer_id = $2))
               RETURNING id)
             INSERT INTO invoice_claims_moved (invoice_id, tenant_id, moved_at)
             SELECT id, $1, $3 FROM drafts
             ON CONFLICT (invoice_id) DO UPDATE SET moved_at = EXCLUDED.moved_at",
        )
        .bind::<diesel::sql_types::Uuid, _>(tenant_id.as_uuid())
        .bind::<diesel::sql_types::Uuid, _>(customer_id.as_uuid())
        .bind::<diesel::sql_types::Timestamp, _>(at)
        .execute(conn)
        .await
        .attach("Error while marking the customer's drafts moved")
        .into_db_result()
    }

    pub async fn clear(conn: &mut PgConn, tenant_id: TenantId, id: InvoiceId) -> DbResult<usize> {
        use crate::schema::invoice_claims_moved::dsl;

        diesel::delete(
            dsl::invoice_claims_moved
                .filter(dsl::tenant_id.eq(tenant_id))
                .filter(dsl::invoice_id.eq(id)),
        )
        .execute(conn)
        .await
        .attach("Error while clearing an invoice's moved claims")
        .into_db_result()
    }

    pub async fn is_moved(conn: &mut PgConn, tenant_id: TenantId, id: InvoiceId) -> DbResult<bool> {
        use crate::schema::invoice_claims_moved::dsl;

        diesel::select(diesel::dsl::exists(
            dsl::invoice_claims_moved
                .filter(dsl::tenant_id.eq(tenant_id))
                .filter(dsl::invoice_id.eq(id)),
        ))
        .get_result(conn)
        .await
        .attach("Error while reading an invoice's moved claims")
        .into_db_result()
    }
}
