use crate::errors::IntoDbResult;
use crate::plan_versions::{
    PlanVersionRow, PlanVersionRowNew, PlanVersionRowOverview, PlanVersionRowPatch,
    PlanVersionTrialRowPatch,
};

use crate::{DbResult, PgConn};

use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use common_domain::ids::{BaseId, PlanId, PlanVersionId, ProductFamilyId, TenantId};
use diesel::prelude::{ExpressionMethods, QueryDsl};
use diesel::{JoinOnDsl, OptionalExtension, SelectableHelper};
use error_stack::ResultExt;

impl PlanVersionRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PlanVersionRow> {
        use crate::schema::plan_version::dsl::plan_version;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(plan_version).values(self);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting plan version")
            .into_db_result()
    }
}

impl PlanVersionRow {
    pub async fn next_version_number(
        conn: &mut PgConn,
        param_plan_id: PlanId,
        param_tenant_id: TenantId,
    ) -> DbResult<i32> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let current_max: Option<i32> = pv_dsl::plan_version
            .filter(pv_dsl::plan_id.eq(param_plan_id))
            .filter(pv_dsl::tenant_id.eq(param_tenant_id))
            .select(diesel::dsl::max(pv_dsl::version))
            .first(conn)
            .await
            .attach("Error while getting max version number")
            .into_db_result()?;

        Ok(current_max.unwrap_or(0) + 1)
    }

    pub async fn find_by_id_and_tenant_id(
        conn: &mut PgConn,
        id: PlanVersionId,
        tenant_id: TenantId,
    ) -> DbResult<PlanVersionRow> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = pv_dsl::plan_version
            .filter(pv_dsl::id.eq(id))
            .filter(pv_dsl::tenant_id.eq(tenant_id));
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding plan version by id")
            .into_db_result()
    }

    /// Locks the plan_version row with FOR NO KEY UPDATE so callers can safely check and mutate
    /// related state (e.g. minimum commitment) without racing a concurrent `publish`.
    pub async fn select_for_update_by_id_and_tenant(
        conn: &mut PgConn,
        id: PlanVersionId,
        tenant_id: TenantId,
    ) -> DbResult<PlanVersionRow> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel::SelectableHelper;
        use diesel_async::RunQueryDsl;

        let query = pv_dsl::plan_version
            .for_no_key_update()
            .filter(pv_dsl::id.eq(id))
            .filter(pv_dsl::tenant_id.eq(tenant_id))
            .select(PlanVersionRow::as_select());
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while locking plan version by id")
            .into_db_result()
    }

    pub async fn get_product_family_id(
        conn: &mut PgConn,
        id: PlanVersionId,
        tenant_id: TenantId,
    ) -> DbResult<ProductFamilyId> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        pv_dsl::plan_version
            .inner_join(p_dsl::plan.on(pv_dsl::plan_id.eq(p_dsl::id)))
            .filter(pv_dsl::id.eq(id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .select(p_dsl::product_family_id)
            .first(conn)
            .await
            .attach("Error while getting product_family_id for plan version")
            .into_db_result()
    }

    pub async fn find_published_by_plan_id_and_version_and_tenant_id(
        conn: &mut PgConn,
        plan_id: PlanId,
        plan_version: Option<i32>,
        tenant_id: TenantId,
    ) -> DbResult<PlanVersionRow> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let mut query = pv_dsl::plan_version
            .filter(pv_dsl::plan_id.eq(plan_id))
            .filter(pv_dsl::tenant_id.eq(tenant_id))
            .filter(pv_dsl::is_draft_version.eq(false))
            .into_boxed();

        if let Some(plan_version) = plan_version {
            query = query.filter(pv_dsl::version.eq(plan_version));
        }

        query = query.order_by(pv_dsl::version.desc());

        crate::debug_sql(&query);

        query = query.limit(1);

        query
            .first(conn)
            .await
            .attach("Error while finding plan version")
            .into_db_result()
    }

    pub async fn find_latest_by_plan_id_and_tenant_id(
        conn: &mut PgConn,
        plan_id: PlanId,
        tenant_id: TenantId,
        is_draft: Option<bool>,
    ) -> DbResult<Option<PlanVersionRow>> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let mut query = pv_dsl::plan_version
            .filter(pv_dsl::plan_id.eq(plan_id))
            .filter(pv_dsl::tenant_id.eq(tenant_id))
            .into_boxed();

        if let Some(is_draft) = is_draft {
            query = query.filter(pv_dsl::is_draft_version.eq(is_draft));
        }

        query = query.order_by(pv_dsl::version.desc());

        crate::debug_sql(&query);

        query = query.limit(1);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding latest plan version")
            .into_db_result()
    }

    pub async fn get_latest_by_plan_id_and_tenant_id(
        conn: &mut PgConn,
        plan_id: PlanId,
        tenant_id: TenantId,
    ) -> DbResult<PlanVersionRow> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let mut query = pv_dsl::plan_version
            .filter(pv_dsl::plan_id.eq(plan_id))
            .filter(pv_dsl::tenant_id.eq(tenant_id))
            .into_boxed();

        query = query.order_by(pv_dsl::version.desc());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding latest plan version")
            .into_db_result()
    }

    pub async fn list_by_plan_id_and_tenant_id(
        conn: &mut PgConn,
        plan_id: PlanId,
        tenant_id: TenantId,
        pagination: PaginationRequest,
    ) -> DbResult<PaginatedVec<PlanVersionRow>> {
        use crate::schema::plan_version::dsl as pv_dsl;

        let paginated_query = pv_dsl::plan_version
            .filter(pv_dsl::plan_id.eq(plan_id))
            .filter(pv_dsl::tenant_id.eq(tenant_id))
            .order(pv_dsl::version.desc())
            .into_boxed()
            .paginate(pagination);

        crate::debug_sql(&paginated_query);

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach("Error while listing plan versions")
            .into_db_result()
    }

    pub async fn delete_others_draft(
        conn: &mut PgConn,
        excl_plan_version_id: PlanVersionId,
        plan_id: PlanId,
        tenant_id: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(
            pv_dsl::plan_version
                .filter(pv_dsl::plan_id.eq(plan_id))
                .filter(pv_dsl::tenant_id.eq(tenant_id))
                .filter(pv_dsl::is_draft_version.eq(true))
                .filter(pv_dsl::id.ne(excl_plan_version_id)),
        );

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting draft plan versions")
            .into_db_result()
    }

    pub async fn publish(
        conn: &mut PgConn,
        id: PlanVersionId,
        tenant_id: TenantId,
    ) -> DbResult<PlanVersionRow> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(pv_dsl::plan_version)
            .filter(pv_dsl::id.eq(id))
            .filter(pv_dsl::tenant_id.eq(tenant_id))
            .set(pv_dsl::is_draft_version.eq(false))
            .returning(PlanVersionRow::as_select());

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while finding plan version by id")
            .into_db_result()
    }

    pub async fn delete_draft(
        conn: &mut PgConn,
        id: PlanVersionId,
        tenant_id: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(pv_dsl::plan_version)
            .filter(pv_dsl::id.eq(id))
            .filter(pv_dsl::tenant_id.eq(tenant_id))
            .filter(pv_dsl::is_draft_version.eq(true));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting plan version")
            .into_db_result()
    }

    pub async fn resolve_published_version_id(
        conn: &mut PgConn,
        plan_id: PlanId,
        plan_version: Option<i32>,
        tenant_id: TenantId,
    ) -> DbResult<PlanVersionId> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let mut query = pv_dsl::plan_version
            .filter(pv_dsl::plan_id.eq(plan_id))
            .filter(pv_dsl::tenant_id.eq(tenant_id))
            .filter(pv_dsl::is_draft_version.eq(false))
            .select(pv_dsl::id)
            .into_boxed();

        if let Some(plan_version) = plan_version {
            query = query.filter(pv_dsl::version.eq(plan_version));
        }

        query = query.order_by(pv_dsl::version.desc());

        crate::debug_sql(&query);

        query
            .first::<PlanVersionId>(conn)
            .await
            .attach("Error while finding plan version")
            .into_db_result()
    }

    pub async fn list_by_ids_and_tenant_id(
        conn: &mut PgConn,
        ids: &[PlanVersionId],
        tenant_id: TenantId,
    ) -> DbResult<Vec<PlanVersionRow>> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let query = pv_dsl::plan_version
            .filter(pv_dsl::id.eq_any(ids))
            .filter(pv_dsl::tenant_id.eq(tenant_id))
            .select(PlanVersionRow::as_select());

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing plan versions by ids")
            .into_db_result()
    }

    pub async fn list_published_by_plan_ids(
        conn: &mut PgConn,
        plan_ids: &[PlanId],
        tenant_id: TenantId,
    ) -> DbResult<Vec<PlanVersionRow>> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        if plan_ids.is_empty() {
            return Ok(Vec::new());
        }

        let query = pv_dsl::plan_version
            .filter(pv_dsl::plan_id.eq_any(plan_ids))
            .filter(pv_dsl::tenant_id.eq(tenant_id))
            .filter(pv_dsl::is_draft_version.eq(false))
            .select(PlanVersionRow::as_select());

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing published plan versions by plan ids")
            .into_db_result()
    }

    /// Get plan_ids for multiple plan_version_ids
    pub async fn get_plan_ids_by_version_ids(
        conn: &mut PgConn,
        plan_version_ids: &[PlanVersionId],
    ) -> DbResult<std::collections::HashMap<PlanVersionId, PlanId>> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        if plan_version_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let query = pv_dsl::plan_version
            .filter(pv_dsl::id.eq_any(plan_version_ids))
            .select((pv_dsl::id, pv_dsl::plan_id));

        crate::debug_sql(&query);

        let rows: Vec<(PlanVersionId, PlanId)> = query
            .load(conn)
            .await
            .attach("Error while getting plan_ids by version_ids")
            .into_db_result()?;

        Ok(rows.into_iter().collect())
    }
}

impl PlanVersionRowOverview {
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<Vec<PlanVersionRowOverview>> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use crate::schema::product_family::dsl as pf_dsl;
        use diesel_async::RunQueryDsl;

        let query = pv_dsl::plan_version
            .inner_join(p_dsl::plan.on(pv_dsl::plan_id.eq(p_dsl::id)))
            .inner_join(pf_dsl::product_family.on(p_dsl::product_family_id.eq(pf_dsl::id)))
            .filter(pv_dsl::tenant_id.eq(tenant_id))
            .filter(pv_dsl::is_draft_version.eq(false))
            .order((
                pv_dsl::plan_id,
                pv_dsl::version.desc(),
                pv_dsl::created_at.desc(),
            ))
            .distinct_on(pv_dsl::plan_id)
            .select(PlanVersionRowOverview::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing plans")
            .into_db_result()
    }
}

impl PlanVersionRow {
    /// Sets the default rule of a version whose rule was not chosen: with `pause_by_default`, usage
    /// pauses once the credit runs out when the version grants credit or prices usage in a credit
    /// unit; otherwise it is invoiced.
    pub async fn settle_credit_exhausted_rule(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: PlanVersionId,
        pause_by_default: bool,
    ) -> DbResult<()> {
        use diesel::sql_types::{Bool, Uuid as SqlUuid};
        use diesel_async::RunQueryDsl;

        diesel::sql_query(
            "UPDATE plan_version pv
             SET credit_exhausted_rule = CASE
                 WHEN $3 AND EXISTS (
                     SELECT 1
                     FROM price_component pc
                     JOIN plan_component_price pcp ON pcp.plan_component_id = pc.id
                     JOIN price p ON p.id = pcp.price_id
                     WHERE pc.plan_version_id = pv.id
                       AND (p.credit_unit_id IS NOT NULL OR p.pricing ->> 'type' = 'Credit'))
                 THEN 'PAUSE'::\"CreditExhaustedRuleEnum\"
                 ELSE 'INVOICE'::\"CreditExhaustedRuleEnum\"
             END
             WHERE pv.id = $1 AND pv.tenant_id = $2 AND pv.credit_exhausted_rule IS NULL",
        )
        .bind::<SqlUuid, _>(id.as_uuid())
        .bind::<SqlUuid, _>(tenant_id.as_uuid())
        .bind::<Bool, _>(pause_by_default)
        .execute(conn)
        .await
        .attach("Error while setting the default credit rule of a plan version")
        .into_db_result()?;
        Ok(())
    }

    /// Whether a price of the version is in a credit unit without a rate for the credits beyond
    /// the balance.
    pub async fn has_unrated_credit_price(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: PlanVersionId,
    ) -> DbResult<bool> {
        use crate::schema::{plan_component_price as pcp, price, price_component as pc};
        use diesel_async::RunQueryDsl;

        diesel::select(diesel::dsl::exists(
            pc::table
                .inner_join(pcp::table.on(pcp::plan_component_id.eq(pc::id)))
                .inner_join(price::table.on(price::id.eq(pcp::price_id)))
                .filter(pc::plan_version_id.eq(id))
                .filter(price::tenant_id.eq(tenant_id))
                .filter(price::credit_unit_id.is_not_null())
                .filter(price::credit_unit_rate.eq(rust_decimal::Decimal::ZERO)),
        ))
        .get_result(conn)
        .await
        .attach("Error while checking the credit rates of a plan version")
        .into_db_result()
    }
}

impl PlanVersionRowPatch {
    pub async fn update_draft(&self, conn: &mut PgConn) -> DbResult<PlanVersionRow> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(pv_dsl::plan_version)
            .filter(pv_dsl::id.eq(self.id))
            .filter(pv_dsl::tenant_id.eq(self.tenant_id))
            .filter(pv_dsl::is_draft_version.eq(true))
            .set(self);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while updating plan version")
            .into_db_result()
    }
}

impl PlanVersionTrialRowPatch {
    pub async fn update_trial(&self, conn: &mut PgConn) -> DbResult<PlanVersionRow> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(pv_dsl::plan_version)
            .filter(pv_dsl::id.eq(self.id))
            .filter(pv_dsl::tenant_id.eq(self.tenant_id))
            .set(self);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while updating plan version trial")
            .into_db_result()
    }
}
