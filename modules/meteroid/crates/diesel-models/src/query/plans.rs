use crate::errors::IntoDbResult;
use crate::plans::{Plan, PlanForList, PlanNew};

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

impl PlanNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<Plan> {
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

impl Plan {
    pub async fn get_by_external_id_and_tenant_id(
        conn: &mut PgConn,
        external_id: &str,
        tenant_id: Uuid,
    ) -> DbResult<Plan> {
        use crate::schema::plan::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = p_dsl::plan
            .filter(p_dsl::external_id.eq(external_id))
            .filter(p_dsl::tenant_id.eq(tenant_id));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while getting plan")
            .into_db_result()
    }

    pub async fn activate(conn: &mut PgConn, id: Uuid, tenant_id: Uuid) -> DbResult<Plan> {
        use crate::schema::plan::dsl as p_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(p_dsl::plan)
            .filter(p_dsl::id.eq(id))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .set((
                p_dsl::status.eq(PlanStatusEnum::Active),
                p_dsl::updated_at.eq(diesel::dsl::now),
            ))
            .returning(Plan::as_select());

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
}

impl PlanForList {
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: Uuid,
        search: Option<String>,
        product_family_external_id: Option<String>,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> DbResult<PaginatedVec<PlanForList>> {
        use crate::schema::plan::dsl as p_dsl;
        use crate::schema::product_family::dsl as pf_dsl;

        let mut query = p_dsl::plan
            .inner_join(pf_dsl::product_family.on(p_dsl::product_family_id.eq(pf_dsl::id)))
            .filter(p_dsl::tenant_id.eq(tenant_id))
            .select(PlanForList::as_select())
            .into_boxed();

        if let Some(product_family_external_id) = product_family_external_id {
            query = query.filter(pf_dsl::external_id.eq(product_family_external_id))
        }

        if let Some(search) = search {
            query = query.filter(
                p_dsl::name
                    .ilike(format!("%{}%", search))
                    .or(p_dsl::external_id.ilike(format!("%{}%", search))),
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
