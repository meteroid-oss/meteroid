use crate::errors::IntoDbResult;
use crate::plan_versions::{PlanVersion, PlanVersionLatest, PlanVersionNew};

use crate::{DbResult, PgConn};

use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use diesel::prelude::{ExpressionMethods, QueryDsl};
use diesel::{debug_query, JoinOnDsl, SelectableHelper};
use error_stack::ResultExt;

impl PlanVersionNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PlanVersion> {
        use crate::schema::plan_version::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(plan_version).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting plan version")
            .into_db_result()
    }
}

impl PlanVersion {
    pub async fn find_by_id_and_tenant_id(
        conn: &mut PgConn,
        id: uuid::Uuid,
        tenant_id: uuid::Uuid,
    ) -> DbResult<PlanVersion> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = pv_dsl::plan_version
            .filter(pv_dsl::id.eq(id))
            .filter(pv_dsl::tenant_id.eq(tenant_id));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding plan version by id")
            .into_db_result()
    }

    pub async fn find_latest_by_plan_id_and_tenant_id(
        conn: &mut PgConn,
        plan_id: uuid::Uuid,
        tenant_id: uuid::Uuid,
        is_draft: Option<bool>,
    ) -> DbResult<PlanVersion> {
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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while finding latest plan version")
            .into_db_result()
    }

    pub async fn list_by_plan_id_and_tenant_id(
        conn: &mut PgConn,
        plan_id: uuid::Uuid,
        tenant_id: uuid::Uuid,
        pagination: PaginationRequest,
    ) -> DbResult<PaginatedVec<PlanVersion>> {
        use crate::schema::plan_version::dsl as pv_dsl;

        let paginated_query = pv_dsl::plan_version
            .filter(pv_dsl::plan_id.eq(plan_id))
            .filter(pv_dsl::tenant_id.eq(tenant_id))
            .order(pv_dsl::version.desc())
            .into_boxed()
            .paginate(pagination);

        log::debug!(
            "{}",
            debug_query::<diesel::pg::Pg, _>(&paginated_query).to_string()
        );

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach_printable("Error while listing plan versions")
            .into_db_result()
    }

    pub async fn delete_others_draft(
        conn: &mut PgConn,
        excl_plan_version_id: uuid::Uuid,
        plan_id: uuid::Uuid,
        tenant_id: uuid::Uuid,
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

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .execute(conn)
            .await
            .attach_printable("Error while deleting draft plan versions")
            .into_db_result()
    }

    pub async fn publish(
        conn: &mut PgConn,
        id: uuid::Uuid,
        tenant_id: uuid::Uuid,
    ) -> DbResult<PlanVersion> {
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(pv_dsl::plan_version)
            .filter(pv_dsl::id.eq(id))
            .filter(pv_dsl::tenant_id.eq(tenant_id))
            .set(pv_dsl::is_draft_version.eq(false))
            .returning(PlanVersion::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while finding plan version by id")
            .into_db_result()
    }
}

impl PlanVersionLatest {
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: uuid::Uuid,
    ) -> DbResult<Vec<PlanVersionLatest>> {
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
            .select(PlanVersionLatest::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while listing plans")
            .into_db_result()
    }
}
