use crate::errors::IntoDbResult;
use crate::plan_versions::{
    PlanVersionRow, PlanVersionRowNew, PlanVersionRowOverview, PlanVersionRowPatch,
    PlanVersionTrialRowPatch,
};

use crate::{DbResult, PgConn};

use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use common_domain::ids::{PlanId, TenantId};
use diesel::prelude::{ExpressionMethods, QueryDsl};
use diesel::{JoinOnDsl, OptionalExtension, SelectableHelper, debug_query};
use error_stack::ResultExt;

impl PlanVersionRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PlanVersionRow> {
        use crate::schema::plan_version::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(plan_version).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting plan version")
            .into_db_result()
    }
}

impl PlanVersionRow {
    pub async fn find_by_id_and_tenant_id(
        conn: &mut PgConn,
        id: uuid::Uuid,
        tenant_id: TenantId,
    ) -> DbResult<PlanVersionRow> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = pv_dsl::plan_version
            .filter(pv_dsl::id.eq(id))
            .filter(pv_dsl::tenant_id.eq(tenant_id));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding plan version by id")
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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query = query.limit(1);

        query
            .first(conn)
            .await
            .optional()
            .attach_printable("Error while finding latest plan version")
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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .first(conn)
            .await
            .attach_printable("Error while finding latest plan version")
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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&paginated_query));

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach_printable("Error while listing plan versions")
            .into_db_result()
    }

    pub async fn delete_others_draft(
        conn: &mut PgConn,
        excl_plan_version_id: uuid::Uuid,
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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach_printable("Error while deleting draft plan versions")
            .into_db_result()
    }

    pub async fn publish(
        conn: &mut PgConn,
        id: uuid::Uuid,
        tenant_id: TenantId,
    ) -> DbResult<PlanVersionRow> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(pv_dsl::plan_version)
            .filter(pv_dsl::id.eq(id))
            .filter(pv_dsl::tenant_id.eq(tenant_id))
            .set(pv_dsl::is_draft_version.eq(false))
            .returning(PlanVersionRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while finding plan version by id")
            .into_db_result()
    }

    pub async fn delete_draft(
        conn: &mut PgConn,
        id: uuid::Uuid,
        tenant_id: TenantId,
    ) -> DbResult<usize> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(pv_dsl::plan_version)
            .filter(pv_dsl::id.eq(id))
            .filter(pv_dsl::tenant_id.eq(tenant_id))
            .filter(pv_dsl::is_draft_version.eq(true));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .execute(conn)
            .await
            .attach_printable("Error while deleting plan version")
            .into_db_result()
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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach_printable("Error while listing plans")
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

        log::info!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while updating plan version")
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

        log::info!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_result(conn)
            .await
            .attach_printable("Error while updating plan version trial")
            .into_db_result()
    }
}
