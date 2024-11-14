use crate::errors::IntoDbResult;
use crate::plans::{
    PlanFilters, PlanRow, PlanRowForList, PlanRowNew, PlanRowPatch, PlanWithVersionRow,
};
use std::collections::HashMap;

use crate::{DbResult, PgConn};

use crate::enums::PlanStatusEnum;
use crate::extend::order::OrderByRequest;
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};
use diesel::{
    debug_query, BoolExpressionMethods, ExpressionMethods, JoinOnDsl, PgTextExpressionMethods,
    QueryDsl, SelectableHelper,
};
use error_stack::ResultExt;
use uuid::Uuid;

impl PlanRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<PlanRow> {
        use crate::schema::plan::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(plan).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting plan")
            .into_db_result()
    }
}

impl PlanRow {
    pub async fn get_by_local_id_and_tenant_id(
        conn: &mut PgConn,
        local_id: &str,
        tenant_id: Uuid,
    ) -> DbResult<PlanRow> {
        use crate::schema::plan::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = p_dsl::plan
            .filter(p_dsl::local_id.eq(local_id))
            .filter(p_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while getting plan")
            .into_db_result()
    }

    pub async fn get_by_id_and_tenant_id(
        conn: &mut PgConn,
        id: Uuid,
        tenant_id: Uuid,
    ) -> DbResult<PlanRow> {
        use crate::schema::plan::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = p_dsl::plan
            .filter(p_dsl::id.eq(id))
            .filter(p_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while getting plan")
            .into_db_result()
    }

    pub async fn activate(conn: &mut PgConn, id: Uuid, tenant_id: Uuid) -> DbResult<PlanRow> {
        use crate::schema::plan::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(p_dsl::plan)
            .filter(p_dsl::id.eq(id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .set((
                p_dsl::status.eq(PlanStatusEnum::Active),
                p_dsl::updated_at.eq(diesel::dsl::now),
            ))
            .returning(PlanRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while activating plan")
            .into_db_result()
    }

    pub async fn delete(conn: &mut PgConn, id: Uuid, tenant_id: Uuid) -> DbResult<usize> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(p_dsl::plan)
            .filter(p_dsl::id.eq(id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .filter(diesel::dsl::not(diesel::dsl::exists(
                pv_dsl::plan_version
                    .filter(pv_dsl::plan_id.eq(id))
                    .filter(pv_dsl::tenant_id.eq(tenant_id))
                    .select(diesel::dsl::sql::<diesel::sql_types::Integer>("1")),
            )));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .execute(conn)
            .await
            .attach_printable("Error while deleting plan")
            .into_db_result()
    }

    pub async fn get_with_version(
        conn: &mut PgConn,
        version_id: Uuid,
        tenant_id: Uuid,
    ) -> DbResult<PlanWithVersionRow> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = p_dsl::plan
            .inner_join(pv_dsl::plan_version.on(p_dsl::id.eq(pv_dsl::plan_id)))
            .filter(pv_dsl::id.eq(version_id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .select(PlanWithVersionRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while getting plan with version")
            .into_db_result()
    }

    pub async fn get_with_version_by_local_id(
        conn: &mut PgConn,
        local_id: &str,
        tenant_id: Uuid,
    ) -> DbResult<PlanWithVersionRow> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::plan_version::dsl as pv_dsl;
        use diesel_async::RunQueryDsl;

        let query = p_dsl::plan
            .inner_join(pv_dsl::plan_version.on(p_dsl::id.eq(pv_dsl::plan_id)))
            .filter(p_dsl::local_id.eq(local_id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .order(pv_dsl::version.desc())
            .select(PlanWithVersionRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while getting plan with version by local_id")
            .into_db_result()
    }
}

impl PlanRowForList {
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: Uuid,
        product_family_local_id: Option<String>,
        filters: PlanFilters,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> DbResult<PaginatedVec<PlanRowForList>> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::product_family::dsl as pf_dsl;

        let mut query = p_dsl::plan
            .inner_join(pf_dsl::product_family.on(p_dsl::product_family_id.eq(pf_dsl::id)))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .select(PlanRowForList::as_select())
            .into_boxed();

        if let Some(product_family_local_id) = product_family_local_id {
            query = query.filter(pf_dsl::local_id.eq(product_family_local_id))
        }

        if !filters.filter_status.is_empty() {
            query = query.filter(p_dsl::status.eq_any(filters.filter_status));
        }

        if !filters.filter_type.is_empty() {
            query = query.filter(p_dsl::plan_type.eq_any(filters.filter_type));
        }

        if let Some(search) = filters.search.filter(|s| !s.is_empty()) {
            query = query.filter(
                p_dsl::name
                    .ilike(format!("%{}%", search))
                    .or(p_dsl::local_id.ilike(format!("%{}%", search))),
            );
        }

        match order_by {
            OrderByRequest::NameAsc => query = query.order(p_dsl::name.asc()),
            OrderByRequest::NameDesc => query = query.order(p_dsl::name.desc()),
            OrderByRequest::DateAsc => query = query.order(p_dsl::created_at.asc()),
            OrderByRequest::DateDesc => query = query.order(p_dsl::created_at.desc()),
            _ => query = query.order(p_dsl::created_at.desc()),
        }

        let paginated_query = query.paginate(pagination);

        log::debug!(
            "{}",
            debug_query::<diesel::pg::Pg, _>(&paginated_query).to_string()
        );

        paginated_query
            .load_and_count_pages(conn)
            .await
            .attach_printable("Error while listing plans")
            .into_db_result()
    }
}

impl PlanRowPatch {
    pub async fn update(&self, conn: &mut PgConn) -> DbResult<PlanRow> {
        use crate::schema::plan::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(p_dsl::plan)
            .filter(p_dsl::id.eq(self.id))
            .filter(p_dsl::tenant_id.eq(self.tenant_id))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while updating plan")
            .into_db_result()
    }
}

pub async fn get_plan_names_by_version_ids(
    conn: &mut PgConn,
    version_ids: Vec<Uuid>,
) -> DbResult<HashMap<Uuid, String>> {
    use crate::schema::plan::dsl as p_dsl;
    use crate::schema::plan_version::dsl as pv_dsl;
    use diesel_async::RunQueryDsl;

    let query = pv_dsl::plan_version
        .inner_join(p_dsl::plan.on(pv_dsl::plan_id.eq(p_dsl::id)))
        .filter(pv_dsl::id.eq_any(version_ids))
        .select((pv_dsl::id, p_dsl::name));

    log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

    query
        .load(conn)
        .await
        .attach_printable("Error while getting plan names by version ids")
        .into_db_result()
        .map(|rows: Vec<(Uuid, String)>| rows.into_iter().collect())
}
