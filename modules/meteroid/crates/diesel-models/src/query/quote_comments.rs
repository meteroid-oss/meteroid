use crate::errors::IntoDbResult;
use crate::quote_comments::{QuoteCommentRow, QuoteCommentRowNew};
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;
use common_domain::ids::{QuoteCommentId, QuoteId, TenantId};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

impl QuoteCommentRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<QuoteCommentRow> {
        use crate::schema::quote_comment::dsl as d;

        let query = diesel::insert_into(d::quote_comment).values(self);
        crate::debug_sql(&query);
        query
            .returning(QuoteCommentRow::as_returning())
            .get_result(conn)
            .await
            .attach("Error while inserting quote comment")
            .into_db_result()
    }
}

impl QuoteCommentRow {
    pub async fn list_by_quote_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
        shared_only: bool,
    ) -> DbResult<Vec<QuoteCommentRow>> {
        use crate::schema::quote_comment::dsl as d;

        let mut query = d::quote_comment
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::quote_id.eq(quote_id))
            .into_boxed();
        if shared_only {
            query = query.filter(d::shared.eq(true));
        }
        let query = query
            .order((d::created_at.asc(), d::id.asc()))
            .select(QuoteCommentRow::as_select());
        crate::debug_sql(&query);
        query
            .get_results(conn)
            .await
            .attach("Error while listing quote comments")
            .into_db_result()
    }

    pub async fn find(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: QuoteCommentId,
    ) -> DbResult<Option<QuoteCommentRow>> {
        use crate::schema::quote_comment::dsl as d;

        let query = d::quote_comment
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::id.eq(id))
            .select(QuoteCommentRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding quote comment")
            .into_db_result()
    }

    /// Resolves the open change requests of the quote; `only` narrows it to one of them.
    pub async fn resolve_change_requests(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
        only: Option<QuoteCommentId>,
        resolved_by: Option<Uuid>,
        at: NaiveDateTime,
    ) -> DbResult<usize> {
        use crate::schema::quote_comment::dsl as d;

        let mut query = diesel::update(d::quote_comment)
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::quote_id.eq(quote_id))
            .filter(d::is_change_request.eq(true))
            .filter(d::resolved_at.is_null())
            .into_boxed();
        if let Some(id) = only {
            query = query.filter(d::id.eq(id));
        }
        let query = query.set((d::resolved_at.eq(at), d::resolved_by.eq(resolved_by)));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .attach("Error while resolving quote change requests")
            .into_db_result()
    }

    pub async fn count_open_change_requests(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> DbResult<i64> {
        use crate::schema::quote_comment::dsl as d;

        let query = d::quote_comment
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::quote_id.eq(quote_id))
            .filter(d::is_change_request.eq(true))
            .filter(d::resolved_at.is_null())
            .count();
        crate::debug_sql(&query);
        query
            .get_result(conn)
            .await
            .attach("Error while counting quote change requests")
            .into_db_result()
    }

    /// Sets the quote's open change-request mark without touching `updated_at`, which guards
    /// concurrent edits of the quote itself.
    pub async fn mark_quote_changes_requested(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
        at: Option<NaiveDateTime>,
    ) -> DbResult<usize> {
        use crate::schema::quote::dsl as q;

        let query = diesel::update(q::quote)
            .filter(q::tenant_id.eq(tenant_id))
            .filter(q::id.eq(quote_id))
            .set(q::changes_requested_at.eq(at));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .attach("Error while marking the quote's change requests")
            .into_db_result()
    }
}
