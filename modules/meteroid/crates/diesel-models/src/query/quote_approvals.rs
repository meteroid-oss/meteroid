use crate::enums::QuoteApprovalStatusEnum;
use crate::errors::IntoDbResult;
use crate::quote_approvals::{
    QuoteApprovalRow, QuoteApprovalRowNew, QuoteApprovalRuleRow, QuoteApprovalRuleRowNew,
    QuoteApprovalRuleRowPatch,
};
use crate::{DbResult, PgConn};
use common_domain::ids::{QuoteApprovalId, QuoteApprovalRuleId, QuoteId, TenantId};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

impl QuoteApprovalRuleRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<QuoteApprovalRuleRow> {
        use crate::schema::quote_approval_rule::dsl as d;

        let query = diesel::insert_into(d::quote_approval_rule).values(self);
        crate::debug_sql(&query);
        query
            .returning(QuoteApprovalRuleRow::as_returning())
            .get_result(conn)
            .await
            .attach("Error while inserting quote approval rule")
            .into_db_result()
    }
}

impl QuoteApprovalRuleRow {
    pub async fn list_active(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<Vec<QuoteApprovalRuleRow>> {
        use crate::schema::quote_approval_rule::dsl as d;

        let query = d::quote_approval_rule
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::archived_at.is_null())
            .order(d::created_at.asc())
            .select(QuoteApprovalRuleRow::as_select());
        crate::debug_sql(&query);
        query
            .get_results(conn)
            .await
            .attach("Error while listing quote approval rules")
            .into_db_result()
    }

    pub async fn update(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: QuoteApprovalRuleId,
        patch: QuoteApprovalRuleRowPatch,
    ) -> DbResult<Option<QuoteApprovalRuleRow>> {
        use crate::schema::quote_approval_rule::dsl as d;

        let query = diesel::update(d::quote_approval_rule)
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::id.eq(id))
            .filter(d::archived_at.is_null())
            .set(patch);
        crate::debug_sql(&query);
        query
            .returning(QuoteApprovalRuleRow::as_returning())
            .get_result(conn)
            .await
            .optional()
            .attach("Error while updating quote approval rule")
            .into_db_result()
    }

    pub async fn archive(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: QuoteApprovalRuleId,
    ) -> DbResult<usize> {
        use crate::schema::quote_approval_rule::dsl as d;

        let query = diesel::update(d::quote_approval_rule)
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::id.eq(id))
            .filter(d::archived_at.is_null())
            .set(d::archived_at.eq(common_clock::now().naive_utc()));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .attach("Error while archiving quote approval rule")
            .into_db_result()
    }
}

impl QuoteApprovalRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<QuoteApprovalRow> {
        use crate::schema::quote_approval::dsl as d;

        let query = diesel::insert_into(d::quote_approval).values(self);
        crate::debug_sql(&query);
        query
            .returning(QuoteApprovalRow::as_returning())
            .get_result(conn)
            .await
            .attach("Error while inserting quote approval")
            .into_db_result()
    }
}

impl QuoteApprovalRow {
    pub async fn find(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: QuoteApprovalId,
    ) -> DbResult<Option<QuoteApprovalRow>> {
        use crate::schema::quote_approval::dsl as d;

        let query = d::quote_approval
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::id.eq(id))
            .select(QuoteApprovalRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding quote approval")
            .into_db_result()
    }

    pub async fn latest_for_quote(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> DbResult<Option<QuoteApprovalRow>> {
        use crate::schema::quote_approval::dsl as d;

        let query = d::quote_approval
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::quote_id.eq(quote_id))
            .order(d::requested_at.desc())
            .select(QuoteApprovalRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while reading quote approval")
            .into_db_result()
    }

    pub async fn list_pending(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<Vec<QuoteApprovalRow>> {
        use crate::schema::quote_approval::dsl as d;

        let query = d::quote_approval
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::status.eq(QuoteApprovalStatusEnum::Pending))
            .order(d::requested_at.asc())
            .select(QuoteApprovalRow::as_select());
        crate::debug_sql(&query);
        query
            .get_results(conn)
            .await
            .attach("Error while listing pending quote approvals")
            .into_db_result()
    }

    /// Decides a pending request; `None` when it was decided or withdrawn meanwhile.
    pub async fn decide(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: QuoteApprovalId,
        status: QuoteApprovalStatusEnum,
        decided_by: Uuid,
        note: Option<String>,
    ) -> DbResult<Option<QuoteApprovalRow>> {
        use crate::schema::quote_approval::dsl as d;

        let query = diesel::update(d::quote_approval)
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::id.eq(id))
            .filter(d::status.eq(QuoteApprovalStatusEnum::Pending))
            .set((
                d::status.eq(status),
                d::decided_by.eq(decided_by),
                d::decided_at.eq(common_clock::now().naive_utc()),
                d::decision_note.eq(note),
            ));
        crate::debug_sql(&query);
        query
            .returning(QuoteApprovalRow::as_returning())
            .get_result(conn)
            .await
            .optional()
            .attach("Error while deciding quote approval")
            .into_db_result()
    }
}
