use crate::errors::IntoDbResult;
use crate::quote_revisions::{QuoteRevisionRow, QuoteRevisionRowNew};
use crate::{DbResult, PgConn};
use common_domain::ids::{QuoteId, StoredDocumentId, TenantId};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl QuoteRevisionRowNew {
    /// Records the revision once: publishing the same revision again keeps the first snapshot.
    pub async fn insert_if_absent(&self, conn: &mut PgConn) -> DbResult<()> {
        use crate::schema::quote_revision::dsl as d;

        let query = diesel::insert_into(d::quote_revision)
            .values(self)
            .on_conflict((d::quote_id, d::revision))
            .do_nothing();
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while inserting quote revision")
            .into_db_result()
    }
}

impl QuoteRevisionRow {
    pub async fn list_by_quote_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> DbResult<Vec<QuoteRevisionRow>> {
        use crate::schema::quote_revision::dsl as d;

        let query = d::quote_revision
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::quote_id.eq(quote_id))
            .order(d::revision.desc())
            .select(QuoteRevisionRow::as_select());
        crate::debug_sql(&query);
        query
            .get_results(conn)
            .await
            .attach("Error while listing quote revisions")
            .into_db_result()
    }

    pub async fn find(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
        revision: i32,
    ) -> DbResult<Option<QuoteRevisionRow>> {
        use crate::schema::quote_revision::dsl as d;

        let query = d::quote_revision
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::quote_id.eq(quote_id))
            .filter(d::revision.eq(revision))
            .select(QuoteRevisionRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding quote revision")
            .into_db_result()
    }

    /// Sets the revision's PDF unless it has one: `false` when another was set first.
    pub async fn set_pdf_document_if_absent(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
        revision: i32,
        pdf_document_id: StoredDocumentId,
    ) -> DbResult<bool> {
        use crate::schema::quote_revision::dsl as d;

        let query = diesel::update(d::quote_revision)
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::quote_id.eq(quote_id))
            .filter(d::revision.eq(revision))
            .filter(d::pdf_document_id.is_null())
            .set(d::pdf_document_id.eq(pdf_document_id));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .map(|updated| updated > 0)
            .attach("Error while setting the quote revision PDF")
            .into_db_result()
    }

    pub async fn mark_superseded(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
        before_revision: i32,
    ) -> DbResult<()> {
        use crate::schema::quote_revision::dsl as d;

        let query = diesel::update(d::quote_revision)
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::quote_id.eq(quote_id))
            .filter(d::revision.lt(before_revision))
            .filter(d::superseded_at.is_null())
            .set(d::superseded_at.eq(common_clock::now().naive_utc()));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while superseding quote revisions")
            .into_db_result()
    }
}
