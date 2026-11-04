use crate::email_kind_audience::EmailKindAudienceRow;
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use common_domain::ids::{InvoicingEntityId, TenantId};
use diesel::upsert::excluded;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl EmailKindAudienceRow {
    /// The caller has checked that the entity belongs to `self.tenant_id`.
    pub async fn upsert(&self, conn: &mut PgConn) -> DbResult<EmailKindAudienceRow> {
        use crate::schema::email_kind_audience::dsl as a_dsl;

        let query = diesel::insert_into(a_dsl::email_kind_audience)
            .values(self)
            .on_conflict((a_dsl::invoicing_entity_id, a_dsl::kind))
            .do_update()
            .set((
                a_dsl::mode.eq(excluded(a_dsl::mode)),
                a_dsl::audience.eq(excluded(a_dsl::audience)),
                a_dsl::updated_at.eq(excluded(a_dsl::updated_at)),
            ))
            .returning(EmailKindAudienceRow::as_returning());
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while upserting email_kind_audience")
            .into_db_result()
    }

    pub async fn list_by_entity(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
    ) -> DbResult<Vec<EmailKindAudienceRow>> {
        use crate::schema::email_kind_audience::dsl as a_dsl;

        let query = a_dsl::email_kind_audience
            .filter(a_dsl::tenant_id.eq(tenant_id))
            .filter(a_dsl::invoicing_entity_id.eq(invoicing_entity_id))
            .order(a_dsl::kind.asc())
            .select(EmailKindAudienceRow::as_select());
        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing email_kind_audience")
            .into_db_result()
    }

    pub async fn find(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        kind: &str,
    ) -> DbResult<Option<EmailKindAudienceRow>> {
        use crate::schema::email_kind_audience::dsl as a_dsl;

        let query = a_dsl::email_kind_audience
            .filter(a_dsl::tenant_id.eq(tenant_id))
            .filter(a_dsl::invoicing_entity_id.eq(invoicing_entity_id))
            .filter(a_dsl::kind.eq(kind))
            .select(EmailKindAudienceRow::as_select());
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding email_kind_audience")
            .into_db_result()
    }

    /// The removed row, if there was one.
    pub async fn delete(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
        kind: &str,
    ) -> DbResult<Option<EmailKindAudienceRow>> {
        use crate::schema::email_kind_audience::dsl as a_dsl;

        let query = diesel::delete(a_dsl::email_kind_audience)
            .filter(a_dsl::tenant_id.eq(tenant_id))
            .filter(a_dsl::invoicing_entity_id.eq(invoicing_entity_id))
            .filter(a_dsl::kind.eq(kind))
            .returning(EmailKindAudienceRow::as_returning());
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while deleting email_kind_audience")
            .into_db_result()
    }
}
