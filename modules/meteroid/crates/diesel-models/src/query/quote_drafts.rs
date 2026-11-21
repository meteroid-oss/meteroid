use crate::errors::IntoDbResult;
use crate::quote_drafts::{QuoteDraftRow, QuoteDraftRowNew};
use crate::{DbResult, PgConn};
use common_domain::ids::{QuoteDraftId, QuoteId, TenantId, UserId};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl QuoteDraftRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<QuoteDraftRow> {
        use crate::schema::quote_draft::dsl as d;

        let query = diesel::insert_into(d::quote_draft).values(self);
        crate::debug_sql(&query);
        query
            .returning(QuoteDraftRow::as_returning())
            .get_result(conn)
            .await
            .attach("Error while inserting quote draft")
            .into_db_result()
    }
}

impl QuoteDraftRow {
    pub async fn find_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: QuoteDraftId,
    ) -> DbResult<Option<QuoteDraftRow>> {
        use crate::schema::quote_draft::dsl as d;

        let query = d::quote_draft
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::id.eq(id))
            .select(QuoteDraftRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding quote draft")
            .into_db_result()
    }

    pub async fn find_by_quote_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> DbResult<Option<QuoteDraftRow>> {
        use crate::schema::quote_draft::dsl as d;

        let query = d::quote_draft
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::quote_id.eq(quote_id))
            .select(QuoteDraftRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding quote draft by quote")
            .into_db_result()
    }

    /// Drafts of quotes not created yet, most recent first.
    pub async fn list_unsaved(
        conn: &mut PgConn,
        tenant_id: TenantId,
        limit: i64,
    ) -> DbResult<Vec<QuoteDraftRow>> {
        use crate::schema::quote_draft::dsl as d;

        let query = d::quote_draft
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::quote_id.is_null())
            .order(d::updated_at.desc())
            .limit(limit)
            .select(QuoteDraftRow::as_select());
        crate::debug_sql(&query);
        query
            .get_results(conn)
            .await
            .attach("Error while listing quote drafts")
            .into_db_result()
    }

    /// Writes the payload only when the stored version is still `expected_version`; `None` when
    /// someone else saved in between.
    pub async fn update_if_version(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: QuoteDraftId,
        expected_version: i32,
        payload: serde_json::Value,
        payload_version: i32,
        label: Option<String>,
        updated_by: UserId,
    ) -> DbResult<Option<QuoteDraftRow>> {
        use crate::schema::quote_draft::dsl as d;

        let query = diesel::update(d::quote_draft)
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::id.eq(id))
            .filter(d::version.eq(expected_version))
            .set((
                d::payload.eq(payload),
                d::payload_version.eq(payload_version),
                d::label.eq(label),
                d::updated_by.eq(updated_by),
                d::version.eq(d::version + 1),
                d::updated_at.eq(common_clock::now().naive_utc()),
            ));
        crate::debug_sql(&query);
        query
            .returning(QuoteDraftRow::as_returning())
            .get_result(conn)
            .await
            .optional()
            .attach("Error while updating quote draft")
            .into_db_result()
    }

    pub async fn delete(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: QuoteDraftId,
    ) -> DbResult<usize> {
        use crate::schema::quote_draft::dsl as d;

        let query = diesel::delete(d::quote_draft)
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::id.eq(id));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .attach("Error while deleting quote draft")
            .into_db_result()
    }

    pub async fn delete_by_quote_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> DbResult<usize> {
        use crate::schema::quote_draft::dsl as d;

        let query = diesel::delete(d::quote_draft)
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::quote_id.eq(quote_id));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .attach("Error while deleting quote draft")
            .into_db_result()
    }
}
